//! `dispatched session state get|set|clear`.

use super::browse::{check_output, explorer_for};
use super::operator_for;
use crate::Format;
use crate::context::Ctx;
use crate::jobs::{JobExit, print_done};
use crate::output::sanitize;
use crate::resolve::Selection;
use crate::safety::{ConfirmArgs, MutateArgs, Planned};
use anyhow::Context as _;
use base64::Engine as _;
use clap::{ArgGroup, Args, Subcommand};
use dispatched_core::{EntityPath, Mutation};
use std::path::PathBuf;

/// The largest session state without `--allow-large` (Standard tier).
pub const STATE_LIMIT: usize = 256 * 1024;

/// `session state set` (the planned kind and verb of the set operation).
pub const SET_STATE: Planned = Planned {
    kind: Mutation::SessionState,
    verb: "set-session-state",
};
/// `session state clear`.
pub const CLEAR_STATE: Planned = Planned {
    kind: Mutation::SessionState,
    verb: "clear-session-state",
};

/// `session state get|set|clear`.
#[derive(Subcommand, Debug)]
pub enum StateCmd {
    /// Show the state of a session.
    Get(GetArgs),
    /// Replace the state of a session.
    Set(SetArgs),
    /// Clear the state of a session.
    Clear(ClearArgs),
}

/// `session state get`.
#[derive(Args, Debug)]
pub struct GetArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The session id.
    pub session: String,
    /// Write the raw state bytes to this file (or `-` for stdout) instead of printing them.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,
    /// With --out: replace the file if it exists.
    #[arg(long, requires = "out")]
    pub force: bool,
}

/// `session state set`.
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("source").required(true).args(["value", "file"])))]
pub struct SetArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The session id.
    pub session: String,
    /// The new state, as text (visible in the process list; prefer --file for sensitive state).
    #[arg(long, value_name = "TEXT")]
    pub value: Option<String>,
    /// Read the new state (raw bytes) from this file.
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,
    /// Allow a state over 256 KB (Premium); the broker's own limit then applies.
    #[arg(long)]
    pub allow_large: bool,
    #[command(flatten)]
    pub confirm: ConfirmArgs,
}

/// `session state clear`.
#[derive(Args, Debug)]
pub struct ClearArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// The session id.
    pub session: String,
    #[command(flatten)]
    pub confirm: ConfirmArgs,
}

/// Refuse a state over 256 KiB unless `allow_large`.
pub fn check_state_size(state: &[u8], allow_large: bool) -> anyhow::Result<()> {
    check_state_len(state.len() as u64, allow_large)
}

fn check_state_len(len: u64, allow_large: bool) -> anyhow::Result<()> {
    if len > STATE_LIMIT as u64 && !allow_large {
        anyhow::bail!(
            "session state is limited to 256 KB on Standard; on Premium the limit is the \
             entity's maximum message size: pass --allow-large"
        );
    }
    Ok(())
}

/// The state as shown: UTF-8 text as is, otherwise base64 with a note.
pub fn render_state(state: Option<&[u8]>) -> String {
    match state {
        None => "(no state)".into(),
        Some(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => text.into(),
            Err(_) => format!("{} (binary, base64)", BASE64.encode(bytes)),
        },
    }
}

const BASE64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// The new state of `set`, checked against the size limit before any I/O beyond the file's
/// own metadata.
fn new_state(args: &SetArgs) -> anyhow::Result<Vec<u8>> {
    if let Some(value) = &args.value {
        check_state_len(value.len() as u64, args.allow_large)?;
        return Ok(value.clone().into_bytes());
    }
    let path = args.file.as_ref().context("pass --value or --file")?;
    let shown = sanitize(&path.display().to_string());
    let len = std::fs::metadata(path)
        .with_context(|| format!("reading {shown}"))?
        .len();
    check_state_len(len, args.allow_large)?;
    let bytes = std::fs::read(path).with_context(|| format!("reading {shown}"))?;
    check_state_size(&bytes, args.allow_large)?;
    Ok(bytes)
}

/// Print lines of untrusted text without terminal escapes (line breaks stay).
fn printable(text: &str) -> String {
    text.split('\n')
        .map(sanitize)
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn run(
    ctx: &Ctx,
    sel: &Selection<'_>,
    cmd: &StateCmd,
    format: Format,
) -> anyhow::Result<JobExit> {
    match cmd {
        StateCmd::Get(a) => {
            get(ctx, sel, a, format).await?;
            Ok(JobExit::Ok)
        }
        StateCmd::Set(a) => set(ctx, sel, a).await,
        StateCmd::Clear(a) => clear(ctx, sel, a).await,
    }
}

/// A read: not gated, not audited.
async fn get(ctx: &Ctx, sel: &Selection<'_>, args: &GetArgs, format: Format) -> anyhow::Result<()> {
    let entity = EntityPath::parse(&args.entity)?;
    let out = args.out.as_ref().map(|p| p.display().to_string());
    if let Some(out) = &out {
        check_output(out, args.force)?;
    }
    let explorer = explorer_for(ctx, sel)?;
    let result = explorer.session_state(&entity, &args.session).await;
    explorer.close().await;
    let state = result?;
    if let Some(out) = &out {
        let Some(bytes) = &state else {
            eprintln!("(no state; nothing was written to {})", sanitize(out));
            return Ok(());
        };
        if out == "-" {
            use std::io::Write as _;
            let mut so = std::io::stdout().lock();
            so.write_all(bytes).and_then(|()| so.flush())?;
            return Ok(());
        }
        write_state(std::path::Path::new(out), bytes, args.force)?;
        eprintln!("Wrote {} byte(s) to {}", bytes.len(), sanitize(out));
        return Ok(());
    }
    if let Format::Json = format {
        let value = state_json(&args.session, state.as_deref());
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{}", printable(&render_state(state.as_deref())));
    }
    Ok(())
}

/// The JSON of `get`: the state as base64, and as text when it is UTF-8 (null when there is
/// none, and `state_text` null for binary state).
pub fn state_json(session: &str, state: Option<&[u8]>) -> serde_json::Value {
    serde_json::json!({
        "session_id": session,
        "state_base64": state.map(|b| BASE64.encode(b)),
        "state_text": state.and_then(|b| std::str::from_utf8(b).ok()),
    })
}

/// Write the raw bytes; without `force` the file must not exist (checked again at creation,
/// so a file that appeared meanwhile is not replaced).
fn write_state(path: &std::path::Path, bytes: &[u8], force: bool) -> anyhow::Result<()> {
    use std::io::Write as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if force || dispatched_core::export::is_special(path) {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    let shown = sanitize(&path.display().to_string());
    let mut f = opts
        .open(path)
        .with_context(|| format!("writing {shown}"))?;
    f.write_all(bytes)
        .with_context(|| format!("writing {shown}"))
}

async fn set(ctx: &Ctx, sel: &Selection<'_>, args: &SetArgs) -> anyhow::Result<JobExit> {
    // The size check comes before any I/O (profile, keychain, network).
    let state = new_state(args)?;
    let entity = EntityPath::parse(&args.entity)?;
    let bytes = state.len();
    change(
        ctx,
        sel,
        &args.confirm.mutate(),
        SET_STATE,
        &entity,
        &args.session,
        Some(state),
    )
    .await?;
    Ok(print_done(&format!(
        "Set the state of session {} on {} ({bytes} byte(s))",
        sanitize(&args.session),
        sanitize(&entity.base())
    )))
}

async fn clear(ctx: &Ctx, sel: &Selection<'_>, args: &ClearArgs) -> anyhow::Result<JobExit> {
    let entity = EntityPath::parse(&args.entity)?;
    change(
        ctx,
        sel,
        &args.confirm.mutate(),
        CLEAR_STATE,
        &entity,
        &args.session,
        None,
    )
    .await?;
    Ok(print_done(&format!(
        "Cleared the state of session {} on {}",
        sanitize(&args.session),
        sanitize(&entity.base())
    )))
}

/// The gated change: resolve, precheck (production, report path), confirmable, gate, then the
/// operator call. A denial is an error (exit 1).
async fn change(
    ctx: &Ctx,
    sel: &Selection<'_>,
    mutate: &MutateArgs,
    planned: Planned,
    entity: &EntityPath,
    session: &str,
    state: Option<Vec<u8>>,
) -> anyhow::Result<()> {
    let (operator, _cancel) =
        operator_for(ctx, sel, mutate, planned, std::slice::from_ref(entity))?;
    let result = operator.set_session_state(entity, session, state).await;
    operator.explorer().close().await;
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn try_parse<const N: usize>(args: [&str; N]) -> Result<crate::Cli, clap::Error> {
        use clap::Parser;
        crate::Cli::try_parse_from(std::iter::once("dispatched").chain(args))
    }

    #[test]
    fn state_commands_parse() {
        assert!(try_parse(["session", "state", "get", "q", "RMA-1"]).is_ok());
        assert!(
            try_parse(["session", "state", "set", "q", "RMA-1"]).is_err(),
            "needs --value or --file"
        );
        assert!(try_parse(["session", "state", "clear", "q", "RMA-1", "--yes"]).is_ok());
        assert!(
            try_parse([
                "session",
                "state",
                "set",
                "q",
                "RMA-1",
                "--value",
                "x",
                "--allow-large"
            ])
            .is_ok()
        );
    }

    #[test]
    fn planned_kinds_match_the_operations() {
        use dispatched_core::{Operation, SubQueue, Target};
        let t = || Target::new(EntityPath::queue("q"), SubQueue::Active);
        for (planned, op) in [
            (SET_STATE, Operation::set_session_state(t(), "s", true)),
            (CLEAR_STATE, Operation::set_session_state(t(), "s", false)),
        ] {
            assert_eq!((planned.kind, planned.verb), (op.mutation, op.verb()));
        }
    }

    #[test]
    fn set_and_clear_take_no_report_file() {
        // They change one value and run no per-message job: nothing to report.
        assert!(
            try_parse([
                "session", "state", "clear", "q", "s", "--yes", "--report", "r.json"
            ])
            .is_err()
        );
        assert!(
            try_parse([
                "session", "state", "set", "q", "s", "--value", "x", "--report", "r.json"
            ])
            .is_err()
        );
        assert!(
            try_parse([
                "session",
                "state",
                "clear",
                "q",
                "s",
                "--yes",
                "--confirm-prod",
                "q"
            ])
            .is_ok()
        );
    }

    #[test]
    fn get_json_has_text_base64_and_nulls() {
        let j = state_json("RMA-1", Some(b"{\"step\":2}"));
        assert_eq!(j["session_id"], "RMA-1");
        assert_eq!(j["state_text"], "{\"step\":2}");
        assert_eq!(j["state_base64"], "eyJzdGVwIjoyfQ==");
        let bin = state_json("s", Some(&[0xff, 0x00]));
        assert_eq!(bin["state_base64"], "/wA=");
        assert!(bin["state_text"].is_null());
        let none = state_json("s", None);
        assert!(none["state_base64"].is_null() && none["state_text"].is_null());
    }

    #[test]
    fn large_state_needs_allow_large() {
        let big = vec![b'x'; 256 * 1024 + 1];
        assert!(
            check_state_size(&big, false)
                .unwrap_err()
                .to_string()
                .contains("--allow-large")
        );
        check_state_size(&big, true).unwrap();
        check_state_size(&big[..256 * 1024], false).unwrap();
    }

    #[test]
    fn state_is_shown_as_text_or_base64() {
        assert_eq!(render_state(Some(b"{\"step\":2}")), "{\"step\":2}");
        assert_eq!(render_state(Some(&[0xff, 0x00])), "/wA= (binary, base64)");
        assert_eq!(render_state(None), "(no state)");
    }

    #[test]
    fn a_state_file_over_the_limit_is_refused_before_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        std::fs::write(&path, vec![0u8; STATE_LIMIT + 1]).unwrap();
        let args = |allow_large| SetArgs {
            entity: "q".into(),
            session: "s".into(),
            value: None,
            file: Some(path.clone()),
            allow_large,
            confirm: ConfirmArgs::default(),
        };
        assert!(
            new_state(&args(false))
                .unwrap_err()
                .to_string()
                .contains("--allow-large")
        );
        assert_eq!(new_state(&args(true)).unwrap().len(), STATE_LIMIT + 1);
    }

    #[test]
    fn writing_state_does_not_replace_a_file_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        write_state(&path, b"one", false).unwrap();
        assert!(write_state(&path, b"two", false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        write_state(&path, b"two", true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
    }
}
