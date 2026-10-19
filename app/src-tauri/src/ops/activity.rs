//! The shared activity log (`activity.db`): every change made by the app, the CLI or an agent.
//! The app lists it, follows it for changes made elsewhere, and exports it.

use crate::error::CommandError;
use crate::jobs::Jobs;
use dispatched_core::activity::{ActivityEntry, ActivityLog, ActivityQuery, Outcome};
use dispatched_core::export::ExportFormat;
use horizon_azure_kit::safety::Origin;
use serde::Deserialize;

/// Which entries to show. `outcome` is applied to the page the log returns.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ActivityQueryForm {
    pub limit: Option<u32>,
    pub origin: Option<Origin>,
    pub profile: Option<String>,
    pub outcome: Option<Outcome>,
}

impl ActivityQueryForm {
    fn query(&self) -> ActivityQuery {
        ActivityQuery {
            limit: self.limit.unwrap_or(500),
            origin: self.origin,
            profile: self.profile.clone(),
            ..ActivityQuery::default()
        }
    }
}

fn unavailable() -> CommandError {
    CommandError::new(
        "activity_unavailable",
        "The activity log could not be opened. Restart Dispatched.",
    )
}

pub fn log(state: &crate::state::AppState) -> Result<&ActivityLog, CommandError> {
    state.activity.as_deref().ok_or_else(unavailable)
}

/// Entries matching `form`, newest first.
pub fn list(
    log: &ActivityLog,
    form: &ActivityQueryForm,
) -> Result<Vec<ActivityEntry>, CommandError> {
    let mut entries = log.list(&form.query())?;
    if let Some(o) = form.outcome {
        entries.retain(|e| e.outcome == o);
    }
    Ok(entries)
}

/// Entries written after row `after_id` by anyone, oldest first: what the app follows to see
/// changes made by the CLI or an agent. `None` returns only the newest id (as a one-entry list)
/// so the caller can start following from now.
pub fn since(log: &ActivityLog, after_id: Option<i64>) -> Result<Vec<ActivityEntry>, CommandError> {
    let q = ActivityQuery {
        limit: if after_id.is_some() { 1_000 } else { 1 },
        after_id,
        ..ActivityQuery::default()
    };
    let mut entries = log.list(&q)?;
    entries.reverse();
    Ok(entries)
}

/// Write the entries matching `form` to the picked path (the save dialog asked "replace?").
pub fn export(
    log: &ActivityLog,
    jobs: &Jobs,
    form: &ActivityQueryForm,
    pick: &str,
    format: &str,
) -> Result<u64, CommandError> {
    let path = jobs
        .take_pick(pick)
        .ok_or_else(|| CommandError::new("invalid", "Choose where to save the log."))?;
    let format = if format.eq_ignore_ascii_case("ndjson") {
        ExportFormat::Ndjson
    } else {
        ExportFormat::Json
    };
    Ok(log.export(&form.query(), &path, format)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::activity::{Audit, Phase};
    use dispatched_core::safety::{Mutation, OpKind};
    use time::OffsetDateTime;

    fn entry(origin: Origin, outcome: Outcome) -> ActivityEntry {
        ActivityEntry {
            id: 0,
            op_id: format!("op-{}", uuid::Uuid::new_v4().simple()),
            at: OffsetDateTime::now_utc(),
            origin,
            profile: "dev".into(),
            entity: Some("orders".into()),
            action: "delete".into(),
            kind: OpKind::Mutating(Mutation::Delete),
            phase: Phase::Finished,
            outcome,
            count: Some(2),
            succeeded: Some(2),
            failed: Some(0),
            skipped: None,
            summary: "Delete 2 messages".into(),
            error: None,
            note: None,
        }
    }

    #[test]
    fn activity_list_filters_by_origin_and_outcome() {
        let log = ActivityLog::open_in_memory().unwrap();
        log.record(&entry(Origin::Gui, Outcome::Succeeded)).unwrap();
        log.record(&entry(Origin::Cli, Outcome::Succeeded)).unwrap();
        log.record(&entry(Origin::Cli, Outcome::Failed)).unwrap();
        let cli = list(
            &log,
            &ActivityQueryForm {
                origin: Some(Origin::Cli),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cli.len(), 2);
        let failed = list(
            &log,
            &ActivityQueryForm {
                outcome: Some(Outcome::Failed),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].origin, Origin::Cli);
    }

    #[test]
    fn activity_since_returns_only_newer_oldest_first() {
        let log = ActivityLog::open_in_memory().unwrap();
        let a = log.record(&entry(Origin::Gui, Outcome::Succeeded)).unwrap();
        let head = since(&log, None).unwrap();
        assert_eq!(head.iter().map(|e| e.id).collect::<Vec<_>>(), [a]);
        let b = log.record(&entry(Origin::Cli, Outcome::Succeeded)).unwrap();
        let c = log.record(&entry(Origin::Cli, Outcome::Failed)).unwrap();
        let newer = since(&log, Some(a)).unwrap();
        assert_eq!(newer.iter().map(|e| e.id).collect::<Vec<_>>(), [b, c]);
        assert!(since(&log, Some(c)).unwrap().is_empty());
    }

    #[test]
    fn export_writes_to_the_pick_once() {
        let log = ActivityLog::open_in_memory().unwrap();
        log.record(&entry(Origin::Gui, Outcome::Succeeded)).unwrap();
        let jobs = Jobs::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.json");
        let pick = jobs.pick(path.clone());
        assert_eq!(
            export(&log, &jobs, &Default::default(), &pick, "json").unwrap(),
            1
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Delete 2 messages")
        );
        assert!(export(&log, &jobs, &Default::default(), &pick, "json").is_err());
    }
}
