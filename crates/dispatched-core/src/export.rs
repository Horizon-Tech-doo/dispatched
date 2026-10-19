//! Stable JSON shape for messages leaving Dispatched (CLI output, files).

use crate::messaging::{MessageBody, PeekPage, PeekedMessage};
use crate::{Error, Result};
use horizon_azure_kit::safety::CancellationToken;
use serde::Serialize;
use std::fs::File;
use std::future::Future;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Schema identifier written into every exported message.
pub const EXPORT_SCHEMA: &str = "dispatched.message/v1";

/// A message in export form.
#[derive(Debug, Serialize)]
pub struct ExportedMessage<'a> {
    /// Always [`EXPORT_SCHEMA`].
    pub schema: &'static str,
    /// The message fields.
    #[serde(flatten)]
    pub message: &'a PeekedMessage,
    /// The body as text when it is UTF-8 binary (convenience; `body` stays authoritative).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
}

impl<'a> ExportedMessage<'a> {
    /// Wrap `message`.
    pub fn new(message: &'a PeekedMessage) -> Self {
        let body_text = match &message.body {
            MessageBody::Binary(_) => message.body.as_text(),
            _ => None,
        };
        Self {
            schema: EXPORT_SCHEMA,
            message,
            body_text,
        }
    }
}

/// Output layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One pretty-printed JSON array.
    Json,
    /// One JSON object per line.
    Ndjson,
}

impl ExportFormat {
    /// Parse `json` or `ndjson`.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(ExportFormat::Json),
            "ndjson" | "jsonl" => Ok(ExportFormat::Ndjson),
            other => Err(Error::Invalid(format!(
                "unknown export format `{other}`; use json or ndjson"
            ))),
        }
    }
}

/// The largest number of matches requested in one export page.
pub const EXPORT_PAGE: u32 = 1000;

/// How a paged read ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuation {
    /// The end of the entity was reached.
    End,
    /// Stopped early; continue from this sequence number.
    More(i64),
    /// The scan made no progress, so the end of the entity was not confirmed.
    Stalled,
    /// Everything asked for was read; no continuation is known or needed.
    Done,
}

/// How `page` ended.
pub fn continuation(page: &PeekPage) -> Continuation {
    if page.exhausted {
        Continuation::End
    } else {
        page.next_sequence
            .map_or(Continuation::Stalled, Continuation::More)
    }
}

/// Totals of [`stream_pages`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streamed {
    /// Messages handed to the sink.
    pub written: u64,
    /// Messages examined.
    pub scanned: u64,
    /// How it ended.
    pub end: Continuation,
}

/// Read pages until `top` messages (all when `None`) or the end, handing each page to `sink`
/// as it arrives. Each page's `next_sequence` feeds the next request; a page that does not
/// move forward ends the read as `Stalled`. Checks `cancel` before every page.
pub async fn stream_pages<F, Fut>(
    top: Option<u64>,
    from_seq: i64,
    cancel: Option<&CancellationToken>,
    mut fetch: F,
    mut sink: impl FnMut(Vec<PeekedMessage>, u64) -> Result<()>,
) -> Result<Streamed>
where
    F: FnMut(i64, u32) -> Fut,
    Fut: Future<Output = Result<PeekPage>>,
{
    let (mut written, mut scanned, mut from) = (0u64, 0u64, from_seq);
    let end = loop {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(Error::Cancelled);
        }
        let remaining = top.map_or(u64::from(EXPORT_PAGE), |t| t.saturating_sub(written));
        let page = fetch(from, remaining.min(u64::from(EXPORT_PAGE)) as u32).await?;
        scanned += u64::from(page.scanned);
        let end = continuation(&page);
        let mut messages = page.messages;
        if let Some(t) = top {
            messages.truncate(t.saturating_sub(written) as usize);
        }
        written += messages.len() as u64;
        sink(messages, scanned)?;
        let full = top.is_some_and(|t| written >= t);
        match end {
            Continuation::More(n) if !full => {
                if n <= from {
                    // A page must move forward; never loop on one position.
                    break Continuation::Stalled;
                }
                from = n;
            }
            // Asked-for count reached: complete, even if the last page made no progress past it.
            Continuation::Stalled if full => break Continuation::Done,
            other => break other,
        }
    };
    Ok(Streamed {
        written,
        scanned,
        end,
    })
}

fn io(e: impl std::fmt::Display) -> Error {
    Error::Io(format!(
        "cannot write export: {e}; check the output path, permissions and free space"
    ))
}

fn exists(path: &Path) -> Error {
    Error::Invalid(format!(
        "{} already exists; choose another file or allow replacing it",
        path.display()
    ))
}

/// A target that is not a regular file (a device or FIFO) is written directly and is never an
/// "existing file" that needs permission to replace. The CLI's `check_output` uses this too.
pub fn is_special(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| !m.is_file() && !m.is_dir())
}

fn temp_path(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.dispatched-tmp-{}-{n}", std::process::id()))
}

/// An export file written page by page. Regular files are written to a sibling temp file
/// that `finish` syncs and puts in place atomically: with `overwrite` it is renamed over the
/// target; without, a hard link creates the target only if it is still absent (falling back
/// to a check and a rename where hard links are unsupported). Dropping an unfinished export
/// removes the temp file; an existing target is never touched by a failed or cancelled export.
pub struct FileExport {
    target: PathBuf,
    tmp: Option<PathBuf>,
    out: Option<BufWriter<File>>,
    format: ExportFormat,
    overwrite: bool,
    written: u64,
    raw: bool,
}

impl FileExport {
    /// Start an export to `path`. Fails at once when a regular file is there and
    /// `overwrite` is off, or when `path` is a directory.
    pub fn create(path: &Path, format: ExportFormat, overwrite: bool) -> Result<Self> {
        if path.is_dir() {
            return Err(Error::Invalid(format!(
                "{} is a directory; choose a file name inside it",
                path.display()
            )));
        }
        if is_special(path) {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(io)?;
            return Ok(Self {
                target: path.into(),
                tmp: None,
                out: Some(BufWriter::new(file)),
                format,
                overwrite,
                written: 0,
                raw: false,
            });
        }
        if !overwrite && path.exists() {
            return Err(exists(path));
        }
        let tmp = temp_path(path);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(io)?;
        Ok(Self {
            target: path.into(),
            tmp: Some(tmp),
            out: Some(BufWriter::new(file)),
            format,
            overwrite,
            written: 0,
            raw: false,
        })
    }

    /// Messages written so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Append `messages`.
    pub fn write(&mut self, messages: &[PeekedMessage]) -> Result<()> {
        let w = self.out.as_mut().expect("open until finish");
        for m in messages {
            let item = ExportedMessage::new(m);
            match self.format {
                ExportFormat::Ndjson => {
                    serde_json::to_writer(&mut *w, &item).map_err(io)?;
                    w.write_all(b"\n").map_err(io)?;
                }
                ExportFormat::Json => {
                    w.write_all(if self.written == 0 { b"[\n" } else { b",\n" })
                        .map_err(io)?;
                    let pretty = serde_json::to_string_pretty(&item).map_err(io)?;
                    for (i, line) in pretty.lines().enumerate() {
                        if i > 0 {
                            w.write_all(b"\n").map_err(io)?;
                        }
                        w.write_all(b"  ").map_err(io)?;
                        w.write_all(line.as_bytes()).map_err(io)?;
                    }
                }
            }
            self.written += 1;
        }
        Ok(())
    }

    /// Append raw bytes (a caller-built document, e.g. a job report). The format's own array
    /// framing is then skipped at `finish`; the atomic temp-file-and-rename logic is shared.
    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.raw = true;
        let w = self.out.as_mut().expect("open until finish");
        w.write_all(bytes).map_err(io)
    }

    /// Close the document, flush, sync and put the file in place. Returns the message count.
    pub fn finish(mut self) -> Result<u64> {
        let mut w = self.out.take().expect("open until finish");
        if self.format == ExportFormat::Json && !self.raw {
            w.write_all(if self.written == 0 { b"[]\n" } else { b"\n]\n" })
                .map_err(io)?;
        }
        w.flush().map_err(io)?;
        let file = w.into_inner().map_err(|e| io(e.into_error()))?;
        let Some(tmp) = self.tmp.clone() else {
            return Ok(self.written);
        };
        file.sync_all().map_err(io)?;
        drop(file);
        if self.overwrite {
            std::fs::rename(&tmp, &self.target).map_err(io)?;
            self.tmp = None;
            return Ok(self.written);
        }
        match std::fs::hard_link(&tmp, &self.target) {
            // Drop removes the temp name; the target keeps the data.
            Ok(()) => Ok(self.written),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(exists(&self.target)),
            Err(_) => {
                if self.target.exists() {
                    return Err(exists(&self.target));
                }
                std::fs::rename(&tmp, &self.target).map_err(io)?;
                self.tmp = None;
                Ok(self.written)
            }
        }
    }
}

impl std::fmt::Debug for FileExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileExport")
            .field("target", &self.target)
            .field("format", &self.format)
            .field("written", &self.written)
            .finish_non_exhaustive()
    }
}

impl Drop for FileExport {
    fn drop(&mut self) {
        // An unfinished export: close the file without flushing the buffered tail (it would be
        // written to an abandoned FIFO or device), then remove the temp file.
        if let Some(w) = self.out.take() {
            drop(w.into_parts());
        }
        if let Some(tmp) = self.tmp.take() {
            let _ = std::fs::remove_file(tmp);
        }
    }
}

/// Write `messages` to `w`. Returns the number written.
pub fn write_messages(
    mut w: impl Write,
    messages: &[PeekedMessage],
    format: ExportFormat,
) -> Result<usize> {
    let exported: Vec<ExportedMessage<'_>> = messages.iter().map(ExportedMessage::new).collect();
    match format {
        ExportFormat::Json => {
            serde_json::to_writer_pretty(&mut w, &exported).map_err(io)?;
            writeln!(w).map_err(io)?;
        }
        ExportFormat::Ndjson => {
            for m in &exported {
                serde_json::to_writer(&mut w, m).map_err(io)?;
                writeln!(w).map_err(io)?;
            }
        }
    }
    w.flush().map_err(io)?;
    Ok(exported.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{MessageState, test_message};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    fn two() -> Vec<PeekedMessage> {
        let mut a = test_message(1);
        a.body = MessageBody::Binary(b"{\"x\":1}".to_vec());
        let mut b = test_message(2);
        b.body = MessageBody::Binary(vec![0xff, 0x00]);
        vec![a, b]
    }

    fn export_json(messages: &[PeekedMessage]) -> serde_json::Value {
        let mut out = Vec::new();
        write_messages(&mut out, messages, ExportFormat::Json).unwrap();
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn json_array_carries_schema_and_text_convenience() {
        let mut out = Vec::new();
        assert_eq!(
            write_messages(&mut out, &two(), ExportFormat::Json).unwrap(),
            2
        );
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v[0]["schema"], EXPORT_SCHEMA);
        assert_eq!(v[0]["sequence_number"], 1);
        assert_eq!(v[0]["body_text"], "{\"x\":1}");
        assert_eq!(v[0]["body"]["kind"], "binary");
        assert!(v[1].get("body_text").is_none());
    }

    #[test]
    fn json_output_is_a_single_array_of_two() {
        let v = export_json(&two());
        let arr = v.as_array().expect("top-level JSON array");
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn ndjson_is_one_object_per_line() {
        let mut out = Vec::new();
        write_messages(&mut out, &two(), ExportFormat::Ndjson).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["schema"], EXPORT_SCHEMA);
        }
    }

    #[test]
    fn ndjson_has_exactly_two_newline_terminated_lines() {
        let mut out = Vec::new();
        write_messages(&mut out, &two(), ExportFormat::Ndjson).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with('\n'));
        assert_eq!(text.matches('\n').count(), 2, "no other newlines: {text:?}");
        assert!(!text.contains('\r'));
        let seqs: Vec<i64> = text
            .lines()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["sequence_number"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
        assert_eq!(seqs, vec![1, 2]);
    }

    #[test]
    fn exported_state_is_pinned() {
        let mut m = test_message(7);
        m.state = MessageState::Scheduled;
        let v = export_json(&[m]);
        assert_eq!(v[0]["state"], "scheduled");
    }

    #[test]
    fn binary_body_round_trips_through_base64() {
        let bytes = vec![0u8, 1, 2, 254, 255];
        let mut m = test_message(1);
        m.body = MessageBody::Binary(bytes.clone());
        let v = export_json(&[m]);
        assert_eq!(v[0]["body"]["kind"], "binary");
        let encoded = v[0]["body"]["value"].as_str().unwrap();
        assert_eq!(STANDARD.decode(encoded).unwrap(), bytes);
        // Not valid UTF-8, so no text convenience field.
        assert!(v[0].get("body_text").is_none());
    }

    #[test]
    fn enqueued_time_is_rfc3339_and_parses_back() {
        let t = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let mut m = test_message(1);
        m.enqueued_time = Some(t);
        let v = export_json(&[m]);
        let s = v[0]["enqueued_time"].as_str().unwrap();
        assert_eq!(OffsetDateTime::parse(s, &Rfc3339).unwrap(), t);
    }

    #[test]
    fn text_body_has_no_body_text_convenience() {
        let mut m = test_message(1);
        m.body = MessageBody::Text("hello".into());
        let v = export_json(&[m]);
        assert_eq!(
            v[0]["body"],
            serde_json::json!({"kind": "text", "value": "hello"})
        );
        assert!(v[0].get("body_text").is_none());
    }

    #[test]
    fn format_names() {
        assert_eq!(ExportFormat::parse("NDJSON").unwrap(), ExportFormat::Ndjson);
        assert_eq!(ExportFormat::parse("jsonl").unwrap(), ExportFormat::Ndjson);
        assert_eq!(ExportFormat::parse(" json ").unwrap(), ExportFormat::Json);
        assert!(ExportFormat::parse("csv").is_err());
    }

    #[test]
    fn write_failure_is_io_error_with_hint() {
        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let err = write_messages(Failing, &two(), ExportFormat::Ndjson).unwrap_err();
        assert!(
            matches!(err, Error::Io(ref m) if m.contains("disk full") && m.contains("free space")),
            "{err}"
        );
    }

    // --- paged reads ---

    /// A fake service: hands out the queued pages and records each request.
    struct Fake {
        pages: RefCell<VecDeque<PeekPage>>,
        requests: RefCell<Vec<(i64, u32)>>,
    }

    impl Fake {
        fn new(pages: Vec<PeekPage>) -> Self {
            Self {
                pages: RefCell::new(pages.into()),
                requests: RefCell::new(Vec::new()),
            }
        }
        async fn fetch(&self, from: i64, max: u32) -> Result<PeekPage> {
            self.requests.borrow_mut().push((from, max));
            Ok(self
                .pages
                .borrow_mut()
                .pop_front()
                .expect("unexpected request"))
        }
    }

    fn page(seqs: &[i64], scanned: u32, next: Option<i64>, exhausted: bool) -> PeekPage {
        PeekPage {
            messages: seqs.iter().map(|s| test_message(*s)).collect(),
            scanned,
            next_sequence: next,
            exhausted,
        }
    }

    #[test]
    fn continuation_tells_end_more_and_stalled_apart() {
        assert_eq!(continuation(&page(&[], 0, None, true)), Continuation::End);
        assert_eq!(
            continuation(&page(&[1], 1, Some(2), false)),
            Continuation::More(2)
        );
        // No progress: next_sequence None, not exhausted.
        assert_eq!(
            continuation(&page(&[], 0, None, false)),
            Continuation::Stalled
        );
    }

    #[tokio::test]
    async fn export_pages_until_the_entity_is_exhausted() {
        let fake = Fake::new(vec![
            page(&[1, 2], 5, Some(6), false),
            page(&[8], 3, Some(9), false),
            page(&[], 0, None, true),
        ]);
        let mut got: Vec<PeekedMessage> = Vec::new();
        let s = stream_pages(
            Some(10),
            0,
            None,
            |f, m| fake.fetch(f, m),
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await
        .unwrap();
        let seqs: Vec<i64> = got.iter().map(|m| m.sequence_number).collect();
        assert_eq!(seqs, vec![1, 2, 8]);
        assert_eq!(s.scanned, 8);
        assert_eq!(s.end, Continuation::End);
        // Each page's next_sequence feeds the next request; max is what remains.
        assert_eq!(*fake.requests.borrow(), vec![(0, 10), (6, 8), (9, 7)]);
    }

    #[tokio::test]
    async fn export_stops_at_top_and_reports_where_to_continue() {
        let fake = Fake::new(vec![page(&[1, 2, 3], 3, Some(4), false)]);
        let mut got: Vec<PeekedMessage> = Vec::new();
        let s = stream_pages(
            Some(3),
            0,
            None,
            |f, m| fake.fetch(f, m),
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(s.end, Continuation::More(4));
        assert_eq!(fake.requests.borrow().len(), 1);
    }

    #[tokio::test]
    async fn export_caps_page_size_and_truncates_to_top() {
        let fake = Fake::new(vec![page(&[1, 2, 3], 3, Some(4), true)]);
        let mut got: Vec<PeekedMessage> = Vec::new();
        stream_pages(
            Some(2),
            5,
            None,
            |f, m| fake.fetch(f, m),
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(got.len(), 2);
        let fake = Fake::new(vec![page(&[], 0, None, true)]);
        stream_pages(Some(5_000), 0, None, |f, m| fake.fetch(f, m), |_, _| Ok(()))
            .await
            .unwrap();
        assert_eq!(*fake.requests.borrow(), vec![(0, EXPORT_PAGE)]);
    }

    #[tokio::test]
    async fn export_without_progress_is_stalled_not_end() {
        let fake = Fake::new(vec![
            page(&[1], 1, Some(2), false),
            page(&[], 0, None, false),
        ]);
        let mut got: Vec<PeekedMessage> = Vec::new();
        let s = stream_pages(
            Some(10),
            0,
            None,
            |f, m| fake.fetch(f, m),
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(s.end, Continuation::Stalled);
    }

    #[tokio::test]
    async fn export_never_loops_on_a_page_that_does_not_advance() {
        // next_sequence equal to the request position would loop forever.
        let fake = Fake::new(vec![page(&[], 4, Some(0), false)]);
        let mut got: Vec<PeekedMessage> = Vec::new();
        let s = stream_pages(
            Some(10),
            0,
            None,
            |f, m| fake.fetch(f, m),
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(s.end, Continuation::Stalled);
        assert_eq!(fake.requests.borrow().len(), 1);
    }

    #[tokio::test]
    async fn export_propagates_errors_from_a_page() {
        let mut got: Vec<PeekedMessage> = Vec::new();
        let r = stream_pages(
            Some(10),
            0,
            None,
            |_, _| async { Err::<PeekPage, _>(Error::Network("boom".into())) },
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn short_export_is_stalled_but_a_full_one_is_not() {
        // Fewer than top and no progress: end not confirmed.
        let fake = Fake::new(vec![page(&[1], 1, None, false)]);
        let mut got: Vec<PeekedMessage> = Vec::new();
        let s = stream_pages(
            Some(3),
            0,
            None,
            |f, m| fake.fetch(f, m),
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(s.end, Continuation::Stalled);
        // top reached: complete even though the last page made no progress.
        let fake = Fake::new(vec![page(&[1, 2, 3], 3, None, false)]);
        let mut got: Vec<PeekedMessage> = Vec::new();
        let s = stream_pages(
            Some(3),
            0,
            None,
            |f, m| fake.fetch(f, m),
            |m, _| {
                got.extend(m);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(s.end, Continuation::Done);
    }

    #[tokio::test]
    async fn unbounded_stream_reads_to_the_end_and_cancel_stops_it() {
        let pages = vec![page(&[1, 2], 2, Some(3), false), page(&[3], 1, None, true)];
        let fake = Fake::new(pages);
        let mut n = 0;
        let s = stream_pages(
            None,
            0,
            None,
            |f, m| fake.fetch(f, m),
            |msgs, _| {
                n += msgs.len();
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!((n, s.written, s.end), (3, 3, Continuation::End));
        let token = CancellationToken::new();
        token.cancel();
        let fake = Fake::new(vec![page(&[1], 1, None, true)]);
        let e = stream_pages(
            None,
            0,
            Some(&token),
            |f, m| fake.fetch(f, m),
            |_, _| Ok(()),
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Cancelled));
    }

    // --- export files ---

    #[test]
    fn streamed_json_is_byte_identical_to_the_one_shot_writer() {
        let dir = tempfile::tempdir().unwrap();
        for count in [0usize, 1, 2, 5] {
            let msgs: Vec<PeekedMessage> = two().into_iter().cycle().take(count).collect();
            let mut expected = Vec::new();
            write_messages(&mut expected, &msgs, ExportFormat::Json).unwrap();
            let path = dir.path().join(format!("out-{count}.json"));
            let mut f = FileExport::create(&path, ExportFormat::Json, false).unwrap();
            for chunk in msgs.chunks(2) {
                f.write(chunk).unwrap();
            }
            assert_eq!(f.finish().unwrap(), count as u64);
            assert_eq!(std::fs::read(&path).unwrap(), expected, "{count} messages");
        }
    }

    #[test]
    fn ndjson_file_matches_the_one_shot_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.ndjson");
        let mut f = FileExport::create(&path, ExportFormat::Ndjson, false).unwrap();
        f.write(&two()).unwrap();
        f.finish().unwrap();
        let mut expected = Vec::new();
        write_messages(&mut expected, &two(), ExportFormat::Ndjson).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), expected);
    }

    fn names(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn existing_file_is_refused_without_overwrite_and_replaced_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        std::fs::write(&path, "old").unwrap();
        let e = FileExport::create(&path, ExportFormat::Json, false).unwrap_err();
        assert!(e.to_string().contains("already exists"), "{e}");
        let mut f = FileExport::create(&path, ExportFormat::Json, true).unwrap();
        f.write(&two()).unwrap();
        f.finish().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().starts_with('['));
        assert_eq!(names(dir.path()), ["x.json"]);
    }

    #[test]
    fn an_unfinished_export_leaves_nothing_behind_and_the_original_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keep.json");
        std::fs::write(&path, "original").unwrap();
        {
            let mut f = FileExport::create(&path, ExportFormat::Json, true).unwrap();
            f.write(&two()).unwrap();
            // dropped without finish (cancelled or failed)
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(names(dir.path()), ["keep.json"]);
    }

    #[test]
    fn a_target_that_appears_during_the_write_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("race.json");
        let mut f = FileExport::create(&path, ExportFormat::Json, false).unwrap();
        f.write(&two()).unwrap();
        std::fs::write(&path, "someone else").unwrap();
        assert!(
            f.finish()
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else");
        assert_eq!(names(dir.path()), ["race.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn devices_are_written_directly() {
        let mut f = FileExport::create(
            std::path::Path::new("/dev/null"),
            ExportFormat::Ndjson,
            false,
        )
        .unwrap();
        f.write(&two()).unwrap();
        assert_eq!(f.finish().unwrap(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_written_directly() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        match std::process::Command::new("mkfifo").arg(&fifo).status() {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("skipping: mkfifo is not available");
                return;
            }
        }
        assert!(is_special(&fifo));
        let reader = {
            let fifo = fifo.clone();
            std::thread::spawn(move || std::fs::read_to_string(fifo).unwrap())
        };
        // No overwrite, and no temp file or rename: the FIFO itself is written.
        let mut f = FileExport::create(&fifo, ExportFormat::Ndjson, false).unwrap();
        f.write(&two()).unwrap();
        assert_eq!(f.finish().unwrap(), 2);
        let got = reader.join().unwrap();
        assert_eq!(got.lines().count(), 2, "{got}");
        assert_eq!(names(dir.path()), ["pipe"]);
        assert!(
            !std::fs::metadata(&fifo).unwrap().is_file(),
            "the FIFO must not be replaced by a regular file"
        );
    }

    #[test]
    fn io_errors_name_the_fix_without_an_em_dash() {
        let e = FileExport::create(
            std::path::Path::new("/nonexistent-dir/x.json"),
            ExportFormat::Json,
            false,
        )
        .unwrap_err();
        let text = e.to_string();
        assert!(text.contains("check the output path"), "{text}");
        assert!(!text.contains('\u{2014}'));
    }

    #[test]
    fn text_bodies_are_escaped_by_serde_not_sanitised() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = test_message(1);
        m.body = MessageBody::Text("a\x1b[31m\u{202e}b".into());
        for (format, name) in [
            (ExportFormat::Ndjson, "t.ndjson"),
            (ExportFormat::Json, "t.json"),
        ] {
            let path = dir.path().join(name);
            let mut f = FileExport::create(&path, format, false).unwrap();
            f.write(std::slice::from_ref(&m)).unwrap();
            f.finish().unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("\\u001b"), "{name}: {text}");
            // Control characters are escaped; bidi marks are written as they are, not stripped.
            assert!(!text.contains('\x1b'), "{name}");
            let v: serde_json::Value = if format == ExportFormat::Json {
                serde_json::from_str::<serde_json::Value>(&text).unwrap()[0].clone()
            } else {
                serde_json::from_str(text.lines().next().unwrap()).unwrap()
            };
            assert_eq!(v["body"]["value"], "a\x1b[31m\u{202e}b", "{name}");
        }
    }

    #[test]
    fn an_unfinished_export_to_a_new_path_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.json");
        {
            let mut f = FileExport::create(&path, ExportFormat::Json, false).unwrap();
            f.write(&two()).unwrap();
        }
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn a_finished_export_leaves_only_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        for overwrite in [false, true] {
            let mut f = FileExport::create(&path, ExportFormat::Json, overwrite).unwrap();
            f.write(&two()).unwrap();
            f.finish().unwrap();
            assert_eq!(names(dir.path()), ["out.json"], "overwrite={overwrite}");
            if !overwrite {
                std::fs::remove_file(&path).unwrap();
            }
        }
    }

    /// An export whose temp file cannot be written: a read-only handle makes every write fail.
    fn failing_export(path: &Path, overwrite: bool) -> FileExport {
        let tmp = temp_path(path);
        File::create_new(&tmp).unwrap();
        let file = File::open(&tmp).unwrap();
        FileExport {
            target: path.into(),
            tmp: Some(tmp),
            out: Some(BufWriter::new(file)),
            format: ExportFormat::Json,
            overwrite,
            written: 0,
            raw: false,
        }
    }

    #[test]
    fn a_failed_write_leaves_the_original_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        std::fs::write(&path, "original").unwrap();
        for overwrite in [false, true] {
            // Fails when the buffer fills during `write`...
            let mut f = failing_export(&path, overwrite);
            let mut big = test_message(1);
            big.body = MessageBody::Text("x".repeat(100_000));
            let e = f.write(&[big]).unwrap_err();
            assert!(e.to_string().contains("check the output path"), "{e}");
            drop(f);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
            assert_eq!(names(dir.path()), ["out.json"]);
            // ...or when `finish` flushes the tail.
            let mut f = failing_export(&path, overwrite);
            f.write(&two()).unwrap();
            let e = f.finish().unwrap_err();
            assert!(e.to_string().contains("check the output path"), "{e}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
            assert_eq!(names(dir.path()), ["out.json"]);
        }
    }

    #[test]
    fn a_directory_target_fails_at_create_with_or_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        for overwrite in [false, true] {
            let e = FileExport::create(dir.path(), ExportFormat::Json, overwrite).unwrap_err();
            assert!(e.to_string().contains("is a directory"), "{e}");
        }
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn debug_shows_the_path_only() {
        let dir = tempfile::tempdir().unwrap();
        let f = FileExport::create(&dir.path().join("d.json"), ExportFormat::Json, false).unwrap();
        let text = format!("{f:?}");
        assert!(
            text.contains("d.json") && text.contains("FileExport"),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_abandoned_fifo_export_does_not_flush_its_buffered_tail() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        match std::process::Command::new("mkfifo").arg(&fifo).status() {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("skipping: mkfifo is not available");
                return;
            }
        }
        let reader = {
            let fifo = fifo.clone();
            std::thread::spawn(move || std::fs::read_to_string(fifo).unwrap())
        };
        {
            let mut f = FileExport::create(&fifo, ExportFormat::Ndjson, false).unwrap();
            f.write(&two()).unwrap();
            // dropped unfinished: the buffered lines must not reach the pipe
        }
        assert_eq!(reader.join().unwrap(), "");
    }
}
