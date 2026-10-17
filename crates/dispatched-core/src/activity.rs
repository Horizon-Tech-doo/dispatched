//! The activity log: an append-only record of every operation, kept in
//! `activity.db` next to the profile database. Surfaces read it for the
//! Activity view and `dispatched activity`; the safety gate writes it through
//! the [`Audit`] trait before and after each mutation.

use crate::export::ExportFormat;
use crate::profiles::store::create_private_dir;
use crate::safety::OpKind;
use crate::{Error, Result};
use horizon_azure_kit::safety::Origin;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const SCHEMA_VERSION: i64 = 1;
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Largest page a query may ask for.
pub const MAX_QUERY_LIMIT: u32 = 10_000;
/// Longest `summary` or `error` the log stores, in characters.
pub const MAX_TEXT_CHARS: usize = 2_000;
const EXPORT_PAGE_ROWS: u32 = 1_000;

/// Stored timestamp layout: RFC 3339, UTC, fixed width, so text order is time order.
const STORED_AT: &str = "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z";

/// Where in an operation's life an entry was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// Written before the broker is touched.
    Started,
    /// Written after the operation ended.
    Finished,
}

/// How an operation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Not finished yet (a `Started` entry).
    Pending,
    /// Everything worked.
    Succeeded,
    /// Some items worked, some did not.
    PartlyFailed,
    /// Nothing worked.
    Failed,
    /// Stopped by the user.
    Cancelled,
    /// Refused by a policy or a declined confirmation.
    Denied,
}

/// One row of the activity log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    /// Row id; 0 before insert.
    pub id: i64,
    /// Ties the `Started` and `Finished` entries of one operation together.
    pub op_id: String,
    /// When the entry was written.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// Which surface initiated it.
    pub origin: Origin,
    /// Profile name.
    pub profile: String,
    /// Entity the operation targeted, if any.
    pub entity: Option<String>,
    /// The verb, such as `purge`.
    pub action: String,
    /// Read or mutating.
    pub kind: OpKind,
    /// Started or finished.
    pub phase: Phase,
    /// How it ended.
    pub outcome: Outcome,
    /// Messages the operation covered.
    pub count: Option<u64>,
    /// Messages that worked.
    pub succeeded: Option<u64>,
    /// Messages that failed.
    pub failed: Option<u64>,
    /// Messages skipped.
    pub skipped: Option<u64>,
    /// One sentence saying what happened.
    pub summary: String,
    /// Error text, if it failed.
    pub error: Option<String>,
}

/// Receives audit entries. The safety gate holds one of these.
pub trait Audit: Send + Sync {
    /// Store the entry; returns its id.
    fn record(&self, entry: &ActivityEntry) -> Result<i64>;
}

/// How much history to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Keep at most this many rows (the newest).
    pub max_rows: u64,
    /// Drop rows older than this.
    pub max_age: time::Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_rows: 50_000,
            max_age: time::Duration::days(90),
        }
    }
}

/// Which entries to list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityQuery {
    /// Page size (default 100, at most 10,000).
    pub limit: u32,
    /// Only this surface.
    pub origin: Option<Origin>,
    /// Only this profile.
    pub profile: Option<String>,
    /// Only entries with a larger id (for following the log).
    pub after_id: Option<i64>,
    /// Only this operation.
    pub op_id: Option<String>,
}

impl Default for ActivityQuery {
    fn default() -> Self {
        Self {
            limit: 100,
            origin: None,
            profile: None,
            after_id: None,
            op_id: None,
        }
    }
}

/// The activity log database.
#[derive(Debug)]
pub struct ActivityLog {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl ActivityLog {
    /// Open (creating if needed) the log at `path` with the default limits.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_limits(path, Limits::default())
    }

    /// Open the log and prune it to `limits`.
    pub fn open_with_limits(path: &Path, limits: Limits) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            create_private_dir(dir).map_err(|e| db_err(path, e))?;
        }
        let conn = Connection::open(path).map_err(|e| db_err(path, e))?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|e| db_err(path, e))?;
        // `journal_mode` returns a row, so read it instead of executing.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
            .map_err(|e| db_err(path, e))?;
        let log = Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        };
        log.migrate()?;
        log.prune(limits)?;
        Ok(log)
    }

    /// In-memory log (tests).
    pub fn open_in_memory() -> Result<Self> {
        let path = PathBuf::from(":memory:");
        let conn = Connection::open_in_memory().map_err(|e| db_err(&path, e))?;
        let log = Self {
            conn: Mutex::new(conn),
            path,
        };
        log.migrate()?;
        Ok(log)
    }

    fn err(&self, e: impl std::fmt::Display) -> Error {
        db_err(&self.path, e)
    }

    fn migrate(&self) -> Result<()> {
        let mut conn = self.conn.lock().expect("activity db poisoned");
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| self.err(e))?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| self.err(e))?;
        if version > SCHEMA_VERSION {
            return Err(Error::Io(format!(
                "the activity log at {} was written by a newer Dispatched (schema {version}); \
                 update Dispatched, or move the file aside to start a fresh log",
                self.path.display()
            )));
        }
        if version < 1 {
            tx.execute_batch(
                "CREATE TABLE activity (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    op_id TEXT NOT NULL,
                    at TEXT NOT NULL,
                    origin TEXT NOT NULL,
                    profile TEXT NOT NULL,
                    entity TEXT,
                    action TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    count INTEGER,
                    succeeded INTEGER,
                    failed INTEGER,
                    skipped INTEGER,
                    summary TEXT NOT NULL,
                    error TEXT
                );
                CREATE INDEX activity_op_id ON activity (op_id);
                CREATE INDEX activity_at ON activity (at);
                PRAGMA user_version = 1;",
            )
            .map_err(|e| self.err(e))?;
        }
        tx.commit().map_err(|e| self.err(e))
    }

    fn prune(&self, limits: Limits) -> Result<()> {
        let cutoff = stored_at(OffsetDateTime::now_utc() - limits.max_age)?;
        let keep = i64::try_from(limits.max_rows).unwrap_or(i64::MAX);
        let conn = self.conn.lock().expect("activity db poisoned");
        conn.execute("DELETE FROM activity WHERE at < ?1", params![cutoff])
            .map_err(|e| self.err(e))?;
        conn.execute(
            "DELETE FROM activity WHERE id <= \
             (SELECT id FROM activity ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            params![keep],
        )
        .map_err(|e| self.err(e))?;
        Ok(())
    }

    /// Entries matching `q`, newest first.
    pub fn list(&self, q: &ActivityQuery) -> Result<Vec<ActivityEntry>> {
        self.page(q, q.limit.clamp(1, MAX_QUERY_LIMIT), None)
    }

    /// Up to `limit` entries matching `q` (its own `limit` ignored) with an id
    /// below `before`, newest first.
    fn page(
        &self,
        q: &ActivityQuery,
        limit: u32,
        before: Option<i64>,
    ) -> Result<Vec<ActivityEntry>> {
        let origin = q.origin.map(enum_text).transpose()?;
        let conn = self.conn.lock().expect("activity db poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT id, op_id, at, origin, profile, entity, action, kind, phase, outcome, \
                        count, succeeded, failed, skipped, summary, error \
                 FROM activity \
                 WHERE (?1 IS NULL OR origin = ?1) \
                   AND (?2 IS NULL OR profile = ?2) \
                   AND (?3 IS NULL OR id > ?3) \
                   AND (?4 IS NULL OR op_id = ?4) \
                   AND (?6 IS NULL OR id < ?6) \
                 ORDER BY id DESC LIMIT ?5",
            )
            .map_err(|e| self.err(e))?;
        let rows = stmt
            .query_map(
                params![origin, q.profile, q.after_id, q.op_id, limit, before],
                |r| {
                    Ok(RawRow {
                        id: r.get(0)?,
                        op_id: r.get(1)?,
                        at: r.get(2)?,
                        origin: r.get(3)?,
                        profile: r.get(4)?,
                        entity: r.get(5)?,
                        action: r.get(6)?,
                        kind: r.get(7)?,
                        phase: r.get(8)?,
                        outcome: r.get(9)?,
                        count: r.get(10)?,
                        succeeded: r.get(11)?,
                        failed: r.get(12)?,
                        skipped: r.get(13)?,
                        summary: r.get(14)?,
                        error: r.get(15)?,
                    })
                },
            )
            .map_err(|e| self.err(e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| self.err(e))?.into_entry(&self.path)?);
        }
        Ok(out)
    }

    /// Write every retained entry matching `q` (its `limit` is ignored) to
    /// `path`, newest first; returns how many.
    pub fn export(&self, q: &ActivityQuery, path: &Path, format: ExportFormat) -> Result<u64> {
        self.export_paged(q, path, format, EXPORT_PAGE_ROWS)
    }

    fn export_paged(
        &self,
        q: &ActivityQuery,
        path: &Path,
        format: ExportFormat,
        page_rows: u32,
    ) -> Result<u64> {
        let io = |e: std::io::Error| Error::Io(format!("cannot write {}: {e}", path.display()));
        let mut file = create_private_file(path).map_err(io)?;
        let json = format == ExportFormat::Json;
        if json {
            file.write_all(b"[").map_err(io)?;
        }
        let (mut total, mut before) = (0u64, None);
        loop {
            let rows = self.page(q, page_rows, before)?;
            let Some(last) = rows.last() else { break };
            before = Some(last.id);
            for e in &rows {
                if json {
                    let sep: &[u8] = if total == 0 { b"\n" } else { b",\n" };
                    file.write_all(sep).map_err(io)?;
                    let text = serde_json::to_string_pretty(e).expect("entry serializes");
                    file.write_all(text.as_bytes()).map_err(io)?;
                } else {
                    let line = serde_json::to_string(e).expect("entry serializes");
                    file.write_all(line.as_bytes()).map_err(io)?;
                    file.write_all(b"\n").map_err(io)?;
                }
                total += 1;
            }
        }
        if json {
            file.write_all(if total == 0 { b"]\n" } else { b"\n]\n" })
                .map_err(io)?;
        }
        file.flush().map_err(io)?;
        Ok(total)
    }
}

impl Audit for ActivityLog {
    fn record(&self, entry: &ActivityEntry) -> Result<i64> {
        let at = stored_at(entry.at)?;
        let kind = serde_json::to_string(&entry.kind).expect("OpKind serializes");
        let conn = self.conn.lock().expect("activity db poisoned");
        conn.execute(
            "INSERT INTO activity (op_id, at, origin, profile, entity, action, kind, phase, \
                outcome, count, succeeded, failed, skipped, summary, error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                plain(&entry.op_id),
                at,
                enum_text(entry.origin)?,
                plain(&entry.profile),
                entry.entity.as_deref().map(plain),
                plain(&entry.action),
                kind,
                enum_text(entry.phase)?,
                enum_text(entry.outcome)?,
                entry.count.map(to_i64),
                entry.succeeded.map(to_i64),
                entry.failed.map(to_i64),
                entry.skipped.map(to_i64),
                bounded(&entry.summary),
                entry.error.as_deref().map(bounded),
            ],
        )
        .map_err(|e| self.err(e))?;
        Ok(conn.last_insert_rowid())
    }
}

/// An [`Audit`] that keeps entries in memory, for tests in core and surfaces.
#[derive(Debug, Default)]
pub struct MemoryAudit {
    entries: Mutex<Vec<ActivityEntry>>,
    fail_next: AtomicBool,
}

impl MemoryAudit {
    /// An empty audit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything recorded so far, oldest first.
    pub fn entries(&self) -> Vec<ActivityEntry> {
        self.entries.lock().expect("memory audit poisoned").clone()
    }

    /// Make the next `record` call fail (once).
    pub fn fail_next(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }
}

impl Audit for MemoryAudit {
    fn record(&self, entry: &ActivityEntry) -> Result<i64> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(Error::Io("activity log unavailable (test failure)".into()));
        }
        let mut entries = self.entries.lock().expect("memory audit poisoned");
        let id = entries.len() as i64 + 1;
        let mut stored = entry.clone();
        stored.id = id;
        entries.push(stored);
        Ok(id)
    }
}

struct RawRow {
    id: i64,
    op_id: String,
    at: String,
    origin: String,
    profile: String,
    entity: Option<String>,
    action: String,
    kind: String,
    phase: String,
    outcome: String,
    count: Option<i64>,
    succeeded: Option<i64>,
    failed: Option<i64>,
    skipped: Option<i64>,
    summary: String,
    error: Option<String>,
}

impl RawRow {
    fn into_entry(self, path: &Path) -> Result<ActivityEntry> {
        let bad = |what: &str, e: &dyn std::fmt::Display| db_err(path, format!("bad {what}: {e}"));
        let from_i64 = |v: Option<i64>| v.map(|n| u64::try_from(n).unwrap_or(0));
        Ok(ActivityEntry {
            id: self.id,
            op_id: self.op_id,
            at: OffsetDateTime::parse(&self.at, &Rfc3339).map_err(|e| bad("time", &e))?,
            origin: parse_enum(&self.origin).map_err(|e| bad("origin", &e))?,
            profile: self.profile,
            entity: self.entity,
            action: self.action,
            kind: serde_json::from_str(&self.kind).map_err(|e| bad("kind", &e))?,
            phase: parse_enum(&self.phase).map_err(|e| bad("phase", &e))?,
            outcome: parse_enum(&self.outcome).map_err(|e| bad("outcome", &e))?,
            count: from_i64(self.count),
            succeeded: from_i64(self.succeeded),
            failed: from_i64(self.failed),
            skipped: from_i64(self.skipped),
            summary: self.summary,
            error: self.error,
        })
    }
}

fn stored_at(t: OffsetDateTime) -> Result<String> {
    let fmt =
        time::format_description::parse_borrowed::<2>(STORED_AT).expect("static format parses");
    t.to_offset(time::UtcOffset::UTC)
        .format(&fmt)
        .map_err(|e| Error::Io(format!("cannot format activity time: {e}")))
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The serde text of a unit-variant enum (`"cli"` -> `cli`).
fn enum_text<T: Serialize>(v: T) -> Result<String> {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        _ => Err(Error::Io("activity enum did not serialize to text".into())),
    }
}

fn parse_enum<T: serde::de::DeserializeOwned>(
    s: &str,
) -> std::result::Result<T, serde_json::Error> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
}

/// Replace control and invisible formatting characters (bidi overrides,
/// zero-width marks) with a space so a hostile entity name or error text
/// cannot move the cursor, inject escape sequences or reorder text when
/// printed later.
fn plain(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || is_format(c) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn is_format(c: char) -> bool {
    matches!(c,
        '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

/// Free text for `summary` and `error`: cleaned, scrubbed of secrets, capped.
fn bounded(s: &str) -> String {
    let clean = redact(&plain(s));
    if clean.chars().count() <= MAX_TEXT_CHARS {
        return clean;
    }
    let mut cut: String = clean.chars().take(MAX_TEXT_CHARS - 1).collect();
    cut.push('\u{2026}');
    cut
}

/// Defense in depth: callers must never pass secrets, but if one slips in
/// (a broker error echoing a connection string, say) it is not persisted.
pub(crate) fn redact(s: &str) -> String {
    static PATTERNS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r#"(?i)SharedAccessSignature\s+[^\s"']+"#,
            r#"(?i)SharedAccessKey\s*=\s*[^;\s"']+"#,
            r#"(?i)AccountKey\s*=\s*[^;\s"']+"#,
            r#"(?i)Authorization\s*[:=]\s*(?:(?:Bearer|Basic)\s+)?[^\s,;"']+"#,
            r#"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"#,
            r#"(?i)\bsig=[^&\s;"']+"#,
        ]
        .iter()
        .map(|p| regex::Regex::new(p).expect("static pattern"))
        .collect()
    });
    let mut out = s.to_string();
    for re in patterns {
        out = re.replace_all(&out, "[redacted]").into_owned();
    }
    out
}

fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

fn db_err(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Io(format!("activity log error at {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::Mutation;

    fn entry(op: &str, phase: Phase, outcome: Outcome) -> ActivityEntry {
        ActivityEntry {
            id: 0,
            op_id: op.into(),
            at: OffsetDateTime::now_utc(),
            origin: Origin::Cli,
            profile: "dev".into(),
            entity: Some("orders".into()),
            action: "purge".into(),
            kind: OpKind::Mutating(Mutation::Purge),
            phase,
            outcome,
            count: None,
            succeeded: None,
            failed: None,
            skipped: None,
            summary: "Remove 3 messages from orders on dev".into(),
            error: None,
        }
    }

    #[test]
    fn started_and_finished_entries_round_trip_newest_first() {
        let log = ActivityLog::open_in_memory().unwrap();
        let a = log
            .record(&entry("op-1", Phase::Started, Outcome::Pending))
            .unwrap();
        let mut fin = entry("op-1", Phase::Finished, Outcome::PartlyFailed);
        (fin.succeeded, fin.failed, fin.skipped) = (Some(2), Some(1), Some(0));
        let b = log.record(&fin).unwrap();
        assert!(b > a);
        let all = log.list(&ActivityQuery::default()).unwrap();
        assert_eq!(all.iter().map(|e| e.id).collect::<Vec<_>>(), [b, a]);
        assert_eq!(all[0].outcome, Outcome::PartlyFailed);
        assert_eq!(all[0].failed, Some(1));
        assert_eq!(all[1].phase, Phase::Started);
    }

    #[test]
    fn queries_filter_by_origin_profile_op_and_after_id() {
        let log = ActivityLog::open_in_memory().unwrap();
        let mut gui = entry("op-g", Phase::Started, Outcome::Pending);
        gui.origin = Origin::Gui;
        let first = log
            .record(&entry("op-c", Phase::Started, Outcome::Pending))
            .unwrap();
        log.record(&gui).unwrap();
        assert_eq!(
            log.list(&ActivityQuery {
                origin: Some(Origin::Gui),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            log.list(&ActivityQuery {
                op_id: Some("op-c".into()),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            log.list(&ActivityQuery {
                after_id: Some(first),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            log.list(&ActivityQuery {
                profile: Some("other".into()),
                ..Default::default()
            })
            .unwrap()
            .len(),
            0
        );
    }

    #[test]
    fn two_handles_on_one_file_both_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.db");
        let (a, b) = (
            ActivityLog::open(&path).unwrap(),
            ActivityLog::open(&path).unwrap(),
        );
        a.record(&entry("x", Phase::Started, Outcome::Pending))
            .unwrap();
        b.record(&entry("y", Phase::Started, Outcome::Pending))
            .unwrap();
        assert_eq!(a.list(&ActivityQuery::default()).unwrap().len(), 2);
    }

    #[test]
    fn open_prunes_to_the_row_and_age_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.db");
        let limits = Limits {
            max_rows: 3,
            max_age: time::Duration::days(90),
        };
        {
            let log = ActivityLog::open_with_limits(&path, limits).unwrap();
            let mut old = entry("old", Phase::Started, Outcome::Pending);
            old.at = OffsetDateTime::now_utc() - time::Duration::days(91);
            log.record(&old).unwrap();
            for i in 0..4 {
                log.record(&entry(&format!("n{i}"), Phase::Started, Outcome::Pending))
                    .unwrap();
            }
        }
        let log = ActivityLog::open_with_limits(&path, limits).unwrap();
        let ops: Vec<String> = log
            .list(&ActivityQuery::default())
            .unwrap()
            .into_iter()
            .map(|e| e.op_id)
            .collect();
        assert_eq!(ops, ["n3", "n2", "n1"]);
    }

    #[test]
    fn export_writes_json_and_ndjson() {
        let log = ActivityLog::open_in_memory().unwrap();
        log.record(&entry("op", Phase::Started, Outcome::Pending))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let n = log
            .export(
                &ActivityQuery::default(),
                &dir.path().join("a.ndjson"),
                ExportFormat::Ndjson,
            )
            .unwrap();
        assert_eq!(n, 1);
        let line = std::fs::read_to_string(dir.path().join("a.ndjson")).unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["origin"], "cli");
        assert_eq!(v["kind"], serde_json::json!({"mutating": "purge"}));

        let n = log
            .export(
                &ActivityQuery::default(),
                &dir.path().join("a.json"),
                ExportFormat::Json,
            )
            .unwrap();
        assert_eq!(n, 1);
        let arr: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("a.json")).unwrap())
                .unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_newer_schema_is_refused_with_advice() {
        // user_version 99 written by a newer Dispatched: refuse, do not wipe.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version = 99")
            .unwrap();
        let e = ActivityLog::open(&path).unwrap_err().to_string();
        assert!(e.contains("newer"), "{e}");
    }

    #[test]
    fn control_characters_are_replaced_on_write() {
        let log = ActivityLog::open_in_memory().unwrap();
        let mut e = entry("op", Phase::Started, Outcome::Pending);
        e.entity = Some("a\u{1b}[31mb\nc".into());
        e.summary = "x\ry".into();
        e.error = Some("bad\u{7}".into());
        e.profile = "p\tq".into();
        log.record(&e).unwrap();
        let got = &log.list(&ActivityQuery::default()).unwrap()[0];
        assert_eq!(got.entity.as_deref(), Some("a [31mb c"));
        assert_eq!(got.summary, "x y");
        assert_eq!(got.error.as_deref(), Some("bad "));
        assert_eq!(got.profile, "p q");
    }

    #[test]
    fn memory_audit_records_and_fails_once_on_request() {
        let audit = MemoryAudit::new();
        let id = audit
            .record(&entry("a", Phase::Started, Outcome::Pending))
            .unwrap();
        assert_eq!(id, 1);
        audit.fail_next();
        assert!(
            audit
                .record(&entry("b", Phase::Started, Outcome::Pending))
                .is_err()
        );
        audit
            .record(&entry("c", Phase::Started, Outcome::Pending))
            .unwrap();
        let ops: Vec<String> = audit.entries().into_iter().map(|e| e.op_id).collect();
        assert_eq!(ops, ["a", "c"]);
    }

    #[test]
    fn export_covers_every_row_beyond_the_query_limit_and_page_size() {
        let log = ActivityLog::open_in_memory().unwrap();
        for i in 0..250 {
            log.record(&entry(&format!("o{i}"), Phase::Started, Outcome::Pending))
                .unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        // Default query limit is 100; the export must still cover all 250.
        let p = dir.path().join("all.ndjson");
        let n = log
            .export(&ActivityQuery::default(), &p, ExportFormat::Ndjson)
            .unwrap();
        assert_eq!(n, 250);
        assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 250);
        // A tiny page size exercises the paging seam, newest first, no gaps.
        let p = dir.path().join("paged.json");
        let n = log
            .export_paged(&ActivityQuery::default(), &p, ExportFormat::Json, 7)
            .unwrap();
        assert_eq!(n, 250);
        let v: Vec<ActivityEntry> =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let ids: Vec<i64> = v.iter().map(|e| e.id).collect();
        assert_eq!(ids, (1..=250).rev().collect::<Vec<_>>());
        // Empty export is still valid JSON.
        let p = dir.path().join("none.json");
        let q = ActivityQuery {
            profile: Some("nobody".into()),
            ..Default::default()
        };
        assert_eq!(log.export(&q, &p, ExportFormat::Json).unwrap(), 0);
        let v: Vec<ActivityEntry> =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn secret_patterns_are_scrubbed_from_summary_and_error() {
        let cases = [
            "Endpoint=sb://x.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=abc123+/=;EntityPath=q",
            "token SharedAccessSignature sr=sb%3A%2F%2Fx&sig=ZZZ%2Bsecret&se=1&skn=k used",
            "url ?sr=x&sig=TOPSECRET&se=1",
            "Authorization: Bearer eyJhbGciOi.payload.sigpart",
            "header Authorization=Basic dXNlcjpwYXNz",
            "got Bearer abc.def-ghi",
            "DefaultEndpointsProtocol=https;AccountKey=Zm9vYmFy==;EndpointSuffix=x",
        ];
        let secrets = [
            "abc123",
            "ZZZ",
            "TOPSECRET",
            "eyJhbGciOi",
            "dXNlcjpwYXNz",
            "abc.def",
            "Zm9vYmFy",
        ];
        let log = ActivityLog::open_in_memory().unwrap();
        for c in cases {
            let mut e = entry("op", Phase::Finished, Outcome::Failed);
            e.summary = c.into();
            e.error = Some(c.into());
            log.record(&e).unwrap();
        }
        for got in log.list(&ActivityQuery::default()).unwrap() {
            for text in [got.summary.as_str(), got.error.as_deref().unwrap()] {
                assert!(text.contains("[redacted]"), "{text}");
                for s in secrets {
                    assert!(!text.contains(s), "{s} leaked in {text}");
                }
            }
        }
    }

    #[test]
    fn long_text_is_capped_with_an_ellipsis_and_ordinary_text_is_kept() {
        let log = ActivityLog::open_in_memory().unwrap();
        let mut e = entry("op", Phase::Finished, Outcome::Failed);
        e.summary = "s".repeat(5_000);
        e.error = Some("é".repeat(MAX_TEXT_CHARS));
        log.record(&e).unwrap();
        let got = &log.list(&ActivityQuery::default()).unwrap()[0];
        assert_eq!(got.summary.chars().count(), MAX_TEXT_CHARS);
        assert!(got.summary.ends_with('\u{2026}'));
        assert_eq!(got.error.as_deref(), e.error.as_deref());
        assert_eq!(bounded("design=1 signal"), "design=1 signal");
    }

    #[test]
    fn bidi_and_zero_width_characters_are_replaced() {
        assert_eq!(plain("a\u{202e}b\u{200b}c"), "a b c");
    }
}
