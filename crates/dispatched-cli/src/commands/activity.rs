//! `dispatched activity ls` and `activity export`: the activity log of mutating operations.

use crate::Format;
use crate::commands::browse::{check_output, export_format};
use crate::context::Ctx;
use crate::output::{sanitize, table, thousands};
use clap::{Subcommand, ValueEnum};
use dispatched_core::{ActivityEntry, ActivityQuery};
use horizon_azure_kit::safety::Origin;
use std::io::Write;
use std::path::Path;
use time::format_description::well_known::Rfc3339;

/// Which surface wrote an entry.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OriginArg {
    Cli,
    Gui,
    Mcp,
}

impl From<OriginArg> for Origin {
    fn from(o: OriginArg) -> Self {
        match o {
            OriginArg::Cli => Origin::Cli,
            OriginArg::Gui => Origin::Gui,
            OriginArg::Mcp => Origin::Mcp,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum ActivityCmd {
    /// List recent entries, newest first. The global `--profile NAME` shows only that profile's
    /// entries.
    Ls {
        /// At most this many entries.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=10_000))]
        limit: u32,
        /// Only entries from this surface.
        #[arg(long, value_enum)]
        origin: Option<OriginArg>,
    },
    /// Write every retained entry to a file (JSON, or NDJSON with `--format ndjson`).
    Export {
        /// Output file.
        #[arg(long)]
        out: String,
        /// Replace the output file if it exists.
        #[arg(long)]
        force: bool,
    },
}

/// Run an activity subcommand. `profile` is the global `--profile` flag, used as a filter.
pub fn run(
    ctx: &Ctx,
    cmd: ActivityCmd,
    format: Option<Format>,
    profile: Option<&str>,
) -> anyhow::Result<()> {
    run_to(&mut std::io::stdout().lock(), ctx, cmd, format, profile)
}

fn run_to(
    out: &mut dyn Write,
    ctx: &Ctx,
    cmd: ActivityCmd,
    format: Option<Format>,
    profile: Option<&str>,
) -> anyhow::Result<()> {
    let log = ctx.activity()?;
    // Entries carry the profile's saved name; accept it in any case.
    let profile = profile.map(|name| {
        ctx.store()
            .ok()
            .and_then(|s| s.get(name).ok().flatten())
            .map(|p| p.name)
            .unwrap_or_else(|| name.to_string())
    });
    match cmd {
        ActivityCmd::Ls { limit, origin } => {
            let entries = log.list(&ActivityQuery {
                limit,
                origin: origin.map(Into::into),
                profile,
                ..ActivityQuery::default()
            })?;
            match format.unwrap_or(Format::Table) {
                Format::Json | Format::Ndjson => {
                    writeln!(out, "{}", serde_json::to_string_pretty(&entries)?)?
                }
                Format::Table if entries.is_empty() => writeln!(out, "No activity recorded.")?,
                Format::Table => writeln!(
                    out,
                    "{}",
                    table(
                        &[
                            "TIME", "ORIGIN", "PROFILE", "ACTION", "ENTITY", "OUTCOME", "COUNTS",
                            "SUMMARY",
                        ],
                        &rows(&entries),
                    )
                )?,
            }
        }
        ActivityCmd::Export { out, force } => {
            let fmt = export_format(format)?;
            if out == "-" {
                anyhow::bail!("activity export writes a file; pass a path to --out");
            }
            check_output(&out, force)?;
            let n = log.export(
                &ActivityQuery {
                    profile,
                    ..ActivityQuery::default()
                },
                Path::new(&out),
                fmt,
            )?;
            let noun = if n == 1 { "entry" } else { "entries" };
            eprintln!("Wrote {} {noun} to {}", thousands(n), sanitize(&out));
        }
    }
    Ok(())
}

/// Table rows: time (UTC), origin, profile, action, entity, outcome, counts, summary (with
/// the note and error).
fn rows(entries: &[ActivityEntry]) -> Vec<Vec<String>> {
    entries
        .iter()
        .map(|e| {
            let counts = match (e.succeeded, e.failed, e.skipped) {
                (Some(ok), Some(failed), Some(skipped)) => format!(
                    "{} ok, {} failed, {} skipped",
                    thousands(ok),
                    thousands(failed),
                    thousands(skipped)
                ),
                _ => e.count.map(thousands).unwrap_or_default(),
            };
            let mut summary = e.summary.clone();
            if let Some(note) = &e.note {
                summary.push_str(&format!("; note: {note}"));
            }
            if let Some(error) = &e.error {
                summary.push_str(&format!("; error: {error}"));
            }
            vec![
                time_text(e.at),
                tag(&e.origin),
                e.profile.clone(),
                e.action.clone(),
                e.entity.clone().unwrap_or_default(),
                tag(&e.outcome),
                counts,
                summary,
            ]
        })
        .collect()
}

/// `2026-10-18 09:30:00` (UTC).
fn time_text(at: time::OffsetDateTime) -> String {
    let text = at
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_default();
    text.get(..19).unwrap_or(&text).replacen('T', " ", 1)
}

/// The serialized tag of an enum value (`cli`, `partly-failed`).
fn tag<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::{Mutation, OpKind, Outcome, Phase};

    fn entry() -> ActivityEntry {
        ActivityEntry {
            id: 1,
            op_id: "op-1".into(),
            at: time::OffsetDateTime::UNIX_EPOCH,
            origin: Origin::Cli,
            profile: "prod-weu".into(),
            entity: Some("orders".into()),
            action: "purge".into(),
            kind: OpKind::Mutating(Mutation::Purge),
            phase: Phase::Finished,
            outcome: Outcome::PartlyFailed,
            count: Some(5),
            succeeded: Some(3),
            failed: Some(1),
            skipped: Some(1),
            summary: "Remove 5 messages from orders on prod-weu".into(),
            error: None,
            note: Some("2 messages could not be unlocked".into()),
        }
    }

    #[test]
    fn rows_show_time_origin_profile_action_entity_outcome_counts_and_summary() {
        let [row] = rows(&[entry()]).try_into().unwrap();
        assert_eq!(
            row,
            vec![
                "1970-01-01 00:00:00".to_string(),
                "cli".into(),
                "prod-weu".into(),
                "purge".into(),
                "orders".into(),
                "partly-failed".into(),
                "3 ok, 1 failed, 1 skipped".into(),
                "Remove 5 messages from orders on prod-weu; note: 2 messages could not be unlocked"
                    .into(),
            ]
        );
        let mut denied = entry();
        denied.outcome = Outcome::Denied;
        denied.succeeded = None;
        denied.failed = None;
        denied.skipped = None;
        denied.note = None;
        denied.error = Some("orders is on production profile prod-weu".into());
        let [row] = rows(&[denied]).try_into().unwrap();
        assert_eq!(row[5], "denied");
        assert_eq!(row[6], "5");
        assert!(row[7].ends_with("; error: orders is on production profile prod-weu"));
    }

    #[test]
    fn ls_filters_by_origin_profile_and_limit() {
        use dispatched_core::Audit;
        let ctx = Ctx::for_test_with_profiles(&[("prod-weu", "prod.servicebus.windows.net", true)]);
        let log = ctx.activity().unwrap();
        for (origin, profile) in [
            (Origin::Cli, "dev"),
            (Origin::Gui, "dev"),
            (Origin::Cli, "prod-weu"),
        ] {
            let mut e = entry();
            e.origin = origin;
            e.profile = profile.into();
            log.record(&e).unwrap();
        }
        let ls = |origin, limit, profile: Option<&str>| {
            let mut out = Vec::new();
            run_to(
                &mut out,
                &ctx,
                ActivityCmd::Ls { limit, origin },
                Some(Format::Json),
                profile,
            )
            .unwrap();
            let v: Vec<serde_json::Value> = serde_json::from_slice(&out).unwrap();
            v.iter()
                .map(|e| {
                    format!(
                        "{}:{}",
                        e["origin"].as_str().unwrap(),
                        e["profile"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(ls(Some(OriginArg::Gui), 100, None), ["gui:dev"]);
        assert_eq!(
            ls(None, 100, Some("PROD-WEU")),
            ["cli:prod-weu"],
            "the saved profile name in any case"
        );
        assert_eq!(
            ls(None, 2, None),
            ["cli:prod-weu", "gui:dev"],
            "newest first, limited"
        );
        let mut out = Vec::new();
        run_to(
            &mut out,
            &ctx,
            ActivityCmd::Ls {
                limit: 100,
                origin: Some(OriginArg::Mcp),
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "No activity recorded.\n");
    }
}
