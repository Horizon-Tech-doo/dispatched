//! Operate jobs: every operation that changes messages, run in the background behind the core
//! gate (`crate::safety`). A command checks its form, then [`start_op`] registers the job, builds
//! its operator and returns the job id at once; confirmations, items, notes and the finish
//! arrive as events, and the finished report is kept for the report view and for saving.

use super::operate_types::*;
use crate::error::CommandError;
use crate::events::{ActivityError, Events, JobEvent};
use crate::jobs::Jobs;
use crate::ops::export::JobStarted;
use crate::safety::operator_for;
use crate::state::AppState;
use dispatched_core::EntityKind;
use dispatched_core::export::ExportFormat;
use dispatched_core::jobs::{ItemResult, JobEvent as CoreEvent, JobKind, JobReport, Progress};
use dispatched_core::messaging::{BodySource, MessageSummary, PeekedMessage, read_template};
use dispatched_core::namespace::{EntityPath, SubQueue};
use dispatched_core::ops::{
    ImportOptions, Operator, PurgeScope, ReceiveRequest, ResubmitSpec, SendRequest, default_target,
};
use horizon_azure_kit::safety::CancellationToken;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use time::OffsetDateTime;

/// Session state above this needs "Allow large state" (Premium allows more).
pub const STATE_LIMIT: usize = 256 * 1024;

/// A finished operate job: its report and, for a receive, the messages it took.
pub struct StoredReport {
    pub report: JobReport,
    pub messages: Vec<PeekedMessage>,
}

/// What a job's work returns.
pub enum Done {
    /// A job with per-message results.
    Report(JobReport, Vec<PeekedMessage>),
    /// A single change (session state): one sentence.
    Plain(String),
}

/// A listener that removes messages stops itself at this many held messages…
pub const HELD_MAX_MESSAGES: usize = 10_000;
/// …or at this many bytes of held bodies.
pub const HELD_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// The messages a removing listener took, in arrival order: the report keeps them, and
/// "Send again" reads them. Past either cap the listener is stopped, never a message dropped.
pub struct Held {
    inner: Mutex<(Vec<PeekedMessage>, u64)>,
    max_messages: usize,
    max_bytes: u64,
}

impl Default for Held {
    fn default() -> Self {
        Self::new()
    }
}

impl Held {
    pub fn new() -> Self {
        Self::with_caps(HELD_MAX_MESSAGES, HELD_MAX_BYTES)
    }

    pub fn with_caps(max_messages: usize, max_bytes: u64) -> Self {
        Self {
            inner: Mutex::default(),
            max_messages,
            max_bytes,
        }
    }

    /// Keep `m`; the note to stop with when this one reached a cap.
    fn push(&self, m: PeekedMessage, bytes: u64) -> Option<String> {
        let mut g = self.inner.lock().unwrap();
        let was_full = g.0.len() >= self.max_messages || g.1 >= self.max_bytes;
        g.0.push(m);
        g.1 = g.1.saturating_add(bytes);
        if was_full {
            return None;
        }
        if g.0.len() >= self.max_messages {
            Some(format!(
                "Stopped at {} held messages: save the report, then start again.",
                dispatched_core::safety::count_text(self.max_messages as u64)
            ))
        } else if g.1 >= self.max_bytes {
            Some(format!(
                "Stopped at {} MiB of held messages: save the report, then start again.",
                self.max_bytes / (1024 * 1024)
            ))
        } else {
            None
        }
    }

    /// The held messages, taken out.
    pub fn take(&self) -> Vec<PeekedMessage> {
        std::mem::take(&mut self.inner.lock().unwrap().0)
    }

    /// The held message with sequence number `seq`.
    pub fn find(&self, seq: i64) -> Option<PeekedMessage> {
        self.inner
            .lock()
            .unwrap()
            .0
            .iter()
            .find(|m| m.sequence_number == seq)
            .cloned()
    }
}

type StopFn = Box<dyn Fn(String) + Send + Sync>;

/// Forwards core job events to the webview as desktop job events, keeping the items so a job
/// that ends in an error still has a report of what it did.
pub struct ProgressBridge {
    id: String,
    events: Arc<dyn Events>,
    items: Mutex<Vec<ItemResult>>,
    notes: Mutex<Vec<String>>,
    /// A removing listener's held messages, and how to stop it at a cap.
    held: Mutex<Option<(Arc<Held>, StopFn)>>,
}

impl ProgressBridge {
    pub fn new(id: &str, events: Arc<dyn Events>) -> Self {
        Self {
            id: id.to_string(),
            events,
            items: Mutex::default(),
            notes: Mutex::default(),
            held: Mutex::default(),
        }
    }

    /// The job id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Hold every received message in `held`; at a cap, note why and call `stop`.
    pub fn hold(&self, held: Arc<Held>, stop: StopFn) {
        *self.held.lock().unwrap() = Some((held, stop));
    }

    /// The messages this bridge holds, if it holds them.
    #[cfg(test)]
    pub fn held(&self) -> Option<Arc<Held>> {
        self.held.lock().unwrap().as_ref().map(|(h, _)| h.clone())
    }

    /// The report of a job that ended early: what reached the bridge.
    fn partial(
        &self,
        kind: JobKind,
        entity: &str,
        started_at: OffsetDateTime,
    ) -> Option<JobReport> {
        let items = self.items.lock().unwrap().clone();
        if items.is_empty() {
            return None;
        }
        Some(JobReport {
            op_id: None,
            kind,
            entity: entity.to_string(),
            target: None,
            started_at,
            finished_at: OffsetDateTime::now_utc(),
            cancelled: true,
            items,
            notes: self.notes.lock().unwrap().clone(),
            not_attempted: 0,
        })
    }
}

impl Progress for ProgressBridge {
    fn event(&self, e: &CoreEvent) {
        let id = self.id.clone();
        let out = match e {
            CoreEvent::Started { total, .. } => JobEvent::Progress {
                id,
                done: 0,
                total: *total,
                scanned: 0,
            },
            CoreEvent::Progress { done, total } => JobEvent::Progress {
                id,
                done: *done,
                total: *total,
                scanned: *done,
            },
            CoreEvent::Item(i) => {
                self.items.lock().unwrap().push(i.clone());
                JobEvent::Item {
                    id,
                    item: Box::new(ItemView::from(i)),
                }
            }
            CoreEvent::Received(m) => {
                let summary = MessageSummary::from(&**m);
                let bytes = summary.body_size as u64;
                let out = JobEvent::Received {
                    id: id.clone(),
                    message: Box::new(summary.into()),
                };
                let stop = {
                    let g = self.held.lock().unwrap();
                    g.as_ref().and_then(|(h, _)| h.push((**m).clone(), bytes))
                };
                if let Some(note) = stop {
                    self.events.job(out);
                    self.notes.lock().unwrap().push(note.clone());
                    self.events.job(JobEvent::Note {
                        id,
                        text: note.clone(),
                    });
                    if let Some((_, f)) = self.held.lock().unwrap().as_ref() {
                        f(note);
                    }
                    return;
                }
                out
            }
            CoreEvent::Note(text) => {
                self.notes.lock().unwrap().push(text.clone());
                JobEvent::Note {
                    id,
                    text: text.clone(),
                }
            }
        };
        self.events.job(out);
    }
}

/// What an operate job is: a core job kind, or a session-state change (a single change, no
/// per-message report).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobTag {
    Core(JobKind),
    SessionState,
}

impl JobTag {
    fn name(self) -> String {
        match self {
            JobTag::Core(kind) => serde_json::to_value(kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            JobTag::SessionState => "session-state".into(),
        }
    }
}

impl From<JobKind> for JobTag {
    fn from(kind: JobKind) -> Self {
        JobTag::Core(kind)
    }
}

/// Register an operate job on the connected profile `profile_id`, build its operator and run
/// `work` in the background. Returns the job id once the job is registered; everything else
/// arrives as events.
pub async fn start_op<F, Fut>(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    kind: impl Into<JobTag>,
    entity: String,
    title: String,
    work: F,
) -> Result<JobStarted, CommandError>
where
    F: FnOnce(Operator, Arc<ProgressBridge>, CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = dispatched_core::Result<Done>> + Send + 'static,
{
    // Held while the job is registered, so a connect or disconnect that closes this explorer
    // either comes first (then `active_for` refuses) or stops the job.
    let kind = kind.into();
    let _serial = state.connect_lock.lock().await;
    let (id, cancel) = state.jobs.start();
    let op = match operator_for(state, profile_id, &id, ev.clone(), cancel.clone()).await {
        Ok(op) => op,
        Err(e) => {
            state.jobs.finish(&id, None);
            return Err(e);
        }
    };
    ev.job(JobEvent::Started {
        id: id.clone(),
        kind: kind.name(),
        title,
        total: None,
        file_name: None,
    });
    let bridge = Arc::new(ProgressBridge::new(&id, ev.clone()));
    let (jobs, job_id) = (state.jobs.clone(), id.clone());
    tokio::spawn(async move {
        let started_at = OffsetDateTime::now_utc();
        let result = work(op, bridge.clone(), cancel.clone()).await;
        finish(
            &jobs, &*ev, &job_id, kind, &entity, started_at, &cancel, &bridge, result,
        );
    });
    Ok(JobStarted { id })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn finish(
    jobs: &Jobs,
    ev: &dyn Events,
    id: &str,
    kind: JobTag,
    entity: &str,
    started_at: OffsetDateTime,
    cancel: &CancellationToken,
    bridge: &ProgressBridge,
    result: dispatched_core::Result<Done>,
) {
    let finished = |error, report: Option<&JobReport>, summary| {
        ev.job(JobEvent::Finished {
            id: id.to_string(),
            error,
            report: report.map(|r| Box::new(ReportSummary::from(r))),
            summary,
            output: None,
            written: None,
            missing: None,
            file_name: None,
        })
    };
    match result {
        Ok(Done::Report(report, messages)) => {
            jobs.finish(id, None);
            let line = report.summary_line();
            // A cancelled run ends with a report, not an error: say how it ended all the same. A
            // listener's normal end is the user's stop, so it is cancelled without being an error
            // unless it stopped itself or its connection closed.
            let listener = kind == JobTag::Core(JobKind::Listen);
            let stopped = cancel.is_cancelled()
                && (report.cancelled
                    || jobs.stop_reason(id).is_some()
                    || (listener && jobs.stopped_by_close(id)));
            let error = stopped.then(|| ActivityError {
                kind: stop_kind(jobs, id).into(),
                message: stop_message(jobs, id),
            });
            let stored = Arc::new(StoredReport { report, messages });
            jobs.set_report(id, stored.clone());
            finished(error, Some(&stored.report), Some(line));
        }
        Ok(Done::Plain(line)) => {
            jobs.finish(id, None);
            finished(None, None, Some(line));
        }
        Err(e) => {
            let mut err = CommandError::from(e);
            if cancel.is_cancelled() {
                err = CommandError::new(stop_kind(jobs, id), stop_message(jobs, id));
            }
            jobs.finish(id, None);
            let partial = match kind {
                JobTag::Core(kind) => bridge.partial(kind, entity, started_at),
                JobTag::SessionState => None,
            };
            let summary = partial.as_ref().map(JobReport::summary_line);
            if let Some(report) = partial {
                jobs.set_report(
                    id,
                    Arc::new(StoredReport {
                        report,
                        messages: Vec::new(),
                    }),
                );
            }
            let kept = jobs.report(id);
            finished(
                Some(ActivityError {
                    kind: err.kind.to_string(),
                    message: err.message,
                }),
                kept.as_ref().map(|s| &s.report),
                summary,
            );
        }
    }
}

/// How a job that was asked to stop ended: by the user, by itself (a reason), or because its
/// connection closed.
fn stop_kind(jobs: &Jobs, id: &str) -> &'static str {
    if jobs.stopped_by_close(id) || jobs.stop_reason(id).is_some() {
        "stopped"
    } else {
        "cancelled"
    }
}

fn stop_message(jobs: &Jobs, id: &str) -> String {
    if let Some(r) = jobs.stop_reason(id) {
        r
    } else if jobs.stopped_by_close(id) {
        "Stopped: the connection closed.".into()
    } else {
        "Cancelled.".into()
    }
}

fn opened_files(jobs: &Jobs, pick: &str) -> Result<Vec<PathBuf>, CommandError> {
    jobs.take_open(pick)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| CommandError::new("invalid", "Choose the file again."))
}

pub(crate) fn report(r: dispatched_core::Result<JobReport>) -> dispatched_core::Result<Done> {
    r.map(|r| Done::Report(r, Vec::new()))
}

pub async fn send(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    form: SendForm,
) -> Result<JobStarted, CommandError> {
    let mut template = form.template.message()?;
    let body = match &form.body {
        BodyForm::Text { text } => Some(BodySource::Text(text.clone())),
        BodyForm::Files { pick } => Some(BodySource::Files(opened_files(&state.jobs, pick)?)),
        BodyForm::Template { pick } => {
            let path = opened_files(&state.jobs, pick)?.remove(0);
            template = read_template(&path)?;
            None
        }
    };
    let schedule = form.schedule.as_deref().map(parse_time).transpose()?;
    let schedule = schedule.or(template.scheduled_enqueue_time.take());
    let req = SendRequest {
        target: form.target.clone(),
        template,
        body,
        ids: (&form.ids).into(),
        count: form.count,
        schedule,
        batch: form.batch,
    };
    let verb = if schedule.is_some() {
        "Schedule"
    } else {
        "Send"
    };
    let title = format!(
        "{verb} {} to {}",
        dispatched_core::safety::count_text(form.count),
        form.target
    );
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Send,
        form.target,
        title,
        move |op, b, c| async move { report(op.send(req, &*b, &c).await) },
    )
    .await
}

pub async fn import(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    form: ImportForm,
) -> Result<JobStarted, CommandError> {
    let path = opened_files(&state.jobs, &form.pick)?.remove(0);
    let messages = tokio::task::spawn_blocking(move || dispatched_core::read_messages(&path))
        .await
        .map_err(|e| CommandError::new("io", format!("Reading the file failed: {e}")))??;
    let opts = ImportOptions {
        new_message_id: form.new_message_id,
        keep_sessions: form.keep_sessions,
        drop_dead_letter_properties: !form.keep_dead_letter_properties,
    };
    let title = format!(
        "Import {} into {}",
        dispatched_core::safety::count_text(messages.len() as u64),
        form.target
    );
    let target = form.target.clone();
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Import,
        form.target,
        title,
        move |op, b, c| async move { report(op.import(target, messages, opts, &*b, &c).await) },
    )
    .await
}

pub async fn cancel_scheduled(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    entity: &str,
    sequences: Vec<Seq>,
) -> Result<JobStarted, CommandError> {
    let path = EntityPath::parse(entity)?;
    let seqs = seqs(&sequences);
    let title = format!(
        "Cancel {} scheduled in {entity}",
        dispatched_core::safety::count_text(seqs.len() as u64)
    );
    start_op(
        state,
        ev,
        profile_id,
        JobKind::CancelScheduled,
        entity.into(),
        title,
        move |op, b, c| async move { report(op.cancel_scheduled(&path, &seqs, &*b, &c).await) },
    )
    .await
}

pub async fn receive(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    form: ReceiveForm,
) -> Result<JobStarted, CommandError> {
    let req = ReceiveRequest {
        entity: EntityPath::parse(&form.entity)?,
        sub_queue: form.sub_queue,
        session: form.session.clone().filter(|s| !s.is_empty()),
        max: form.max,
        wait: Duration::from_secs(form.wait_seconds),
        action: (&form.action).into(),
    };
    let how = match form.action {
        ReceiveActionForm::Delete => "and delete",
        ReceiveActionForm::Settle { .. } => "and settle",
    };
    let title = format!("Receive {how} up to {} from {}", form.max, form.entity);
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Receive,
        form.entity,
        title,
        move |op, b, c| async move {
            op.receive(req, &*b, &c)
                .await
                .map(|o| Done::Report(o.report, o.messages))
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn settle_deferred(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    entity: &str,
    sub_queue: SubQueue,
    sequences: Vec<Seq>,
    session: Option<String>,
    how: SettlementForm,
) -> Result<JobStarted, CommandError> {
    let path = EntityPath::parse(entity)?;
    let seqs = seqs(&sequences);
    let title = format!(
        "Settle {} deferred in {entity}",
        dispatched_core::safety::count_text(seqs.len() as u64)
    );
    start_op(
        state,
        ev,
        profile_id,
        JobKind::SettleDeferred,
        entity.into(),
        title,
        move |op, b, c| async move {
            let session = session.filter(|s| !s.is_empty());
            report(
                op.settle_deferred(
                    &path,
                    sub_queue,
                    &seqs,
                    session.as_deref(),
                    (&how).into(),
                    &*b,
                    &c,
                )
                .await,
            )
        },
    )
    .await
}

fn selection_words(form: &SelectForm) -> String {
    match &form.selection {
        SelectionForm::Sequences { sequences } => {
            dispatched_core::safety::count_text(sequences.len() as u64)
        }
        SelectionForm::Matching { .. } => "matching".into(),
    }
}

pub async fn delete(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    form: SelectForm,
) -> Result<JobStarted, CommandError> {
    let spec = form.spec()?;
    let title = format!("Delete {} from {}", selection_words(&form), spec.address());
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Delete,
        form.entity,
        title,
        move |op, b, c| async move { report(op.delete(spec, &*b, &c).await) },
    )
    .await
}

pub async fn move_messages(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    form: SelectForm,
    target: String,
    drop_uncopyable: bool,
) -> Result<JobStarted, CommandError> {
    let spec = form.spec()?;
    let title = format!(
        "Move {} from {} to {target}",
        selection_words(&form),
        spec.address()
    );
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Move,
        form.entity,
        title,
        move |op, b, c| async move {
            report(
                op.move_messages(spec, target, drop_uncopyable, &*b, &c)
                    .await,
            )
        },
    )
    .await
}

pub async fn resubmit(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    form: ResubmitForm,
) -> Result<JobStarted, CommandError> {
    let source = form.source.spec()?;
    let target = match form.target.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => default_target(&source.entity),
    };
    let title = format!(
        "Resubmit {} from {} to {target}",
        selection_words(&form.source),
        source.address()
    );
    let spec = ResubmitSpec {
        source,
        target,
        edits: form.edits.edits()?,
        new_message_id: form.new_message_id,
        remove_original: form.remove_original,
        keep_dead_letter_properties: form.keep_dead_letter_properties,
        drop_uncopyable_properties: form.drop_uncopyable_properties,
    };
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Resubmit,
        form.source.entity,
        title,
        move |op, b, c| async move { report(op.resubmit(spec, &*b, &c).await) },
    )
    .await
}

/// Count every entity's purge (no gate: nothing changes). One failed count fails the whole
/// plan, with nothing changed. The plans wait under one plan id for `op_purge`.
pub async fn purge_plan(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    entities: Vec<String>,
    scope: PurgeScope,
) -> Result<PurgePlans, CommandError> {
    if entities.is_empty() {
        return Err(CommandError::new("invalid", "Choose what to purge."));
    }
    let op = operator_for(
        state,
        profile_id,
        "purge-plan",
        ev,
        CancellationToken::new(),
    )
    .await?;
    let mut plans = Vec::with_capacity(entities.len());
    for e in &entities {
        plans.push(op.plan_purge(&EntityPath::parse(e)?, scope).await?);
    }
    let views = plans.iter().map(PurgePlanView::from).collect();
    Ok(PurgePlans {
        plan_id: state.jobs.plan(profile_id, plans),
        plans: views,
    })
}

pub async fn purge(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    plan_id: &str,
) -> Result<JobStarted, CommandError> {
    let planned = state
        .jobs
        .take_plan(plan_id)
        .filter(|p| p.profile_id == profile_id)
        .ok_or_else(|| {
            CommandError::new(
                "not_found",
                "These counts are no longer current. Count again before purging.",
            )
        })?;
    let plans = planned.plans;
    let entity = match plans.as_slice() {
        [one] => one.entity.to_string(),
        many => format!("{} entities", many.len()),
    };
    let title = format!("Purge {entity}");
    start_op(
        state,
        ev,
        profile_id,
        JobKind::Purge,
        entity,
        title,
        move |op, b, c| async move { report(op.purge(plans, &*b, &c).await) },
    )
    .await
}

pub async fn dlq_summary(
    state: &AppState,
    profile_id: &str,
    entity: &str,
    sub_queue: SubQueue,
) -> Result<DlqSummaryView, CommandError> {
    let active = state.active_for(profile_id).await?;
    let s = active
        .explorer
        .dlq_summary(&EntityPath::parse(entity)?, sub_queue)
        .await?;
    Ok((&s).into())
}

pub async fn session_state_get(
    state: &AppState,
    profile_id: &str,
    entity: &str,
    session: &str,
) -> Result<Option<SessionStateView>, CommandError> {
    let active = state.active_for(profile_id).await?;
    let bytes = active
        .explorer
        .session_state(&EntityPath::parse(entity)?, session)
        .await?;
    Ok(bytes.as_deref().map(SessionStateView::of))
}

/// Set (`Some`, base64) or clear (`None`) the state of `session`.
pub async fn session_state_set(
    state: &AppState,
    ev: Arc<dyn Events>,
    profile_id: &str,
    entity: &str,
    session: &str,
    value: Option<String>,
    allow_large: bool,
) -> Result<JobStarted, CommandError> {
    let path = EntityPath::parse(entity)?;
    let bytes = value.as_deref().map(decode_state).transpose()?;
    if let Some(b) = &bytes
        && b.len() > STATE_LIMIT
        && !allow_large
    {
        return Err(CommandError::new(
            "invalid",
            "The state is over 256 KB. Allow large state (Premium namespaces) to send it.",
        ));
    }
    let session = session.to_string();
    let (title, done) = match &bytes {
        Some(_) => (
            format!("Set the state of session {session} in {entity}"),
            format!("Session {session} in {entity}: state set"),
        ),
        None => (
            format!("Clear the state of session {session} in {entity}"),
            format!("Session {session} in {entity}: state cleared"),
        ),
    };
    start_op(
        state,
        ev,
        profile_id,
        JobTag::SessionState,
        entity.into(),
        title,
        move |op, _b, _c| async move {
            op.set_session_state(&path, &session, bytes)
                .await
                .map(|()| Done::Plain(done))
        },
    )
    .await
}

pub async fn resolve_kind(
    state: &AppState,
    profile_id: &str,
    entity: &str,
) -> Result<Option<EntityKind>, CommandError> {
    let active = state.active_for(profile_id).await?;
    Ok(active
        .explorer
        .resolve_kind(&EntityPath::parse(entity)?)
        .await?)
}

pub fn job_report(
    jobs: &Jobs,
    id: &str,
    offset: u64,
    limit: u64,
    only_problems: bool,
) -> Option<ReportPage> {
    jobs.report(id)
        .map(|s| report_page(&s.report, offset, limit, only_problems))
}

/// Write job `id`'s report to the path the save dialog picked (it asked "replace?").
pub fn save_report(
    jobs: &Jobs,
    id: &str,
    pick: &str,
    format: &str,
) -> Result<String, CommandError> {
    let stored = jobs
        .report(id)
        .ok_or_else(|| CommandError::new("not_found", "That job has no report."))?;
    let path = jobs
        .take_pick(pick)
        .ok_or_else(|| CommandError::new("invalid", "Choose where to save the report."))?;
    let format = if format.eq_ignore_ascii_case("ndjson") {
        ExportFormat::Ndjson
    } else {
        ExportFormat::Json
    };
    stored
        .report
        .write_with_messages(&path, format, true, &stored.messages)?;
    // A listener's held messages (up to 256 MiB each) are on disk now: they leave memory, so restarting a listener
    // after its report was saved does not pile them up.
    if stored.report.kind == JobKind::Listen && !stored.messages.is_empty() {
        let mut report = stored.report.clone();
        report
            .notes
            .push("The removed messages were saved to the report file and released.".into());
        jobs.set_report(
            id,
            Arc::new(StoredReport {
                report,
                messages: Vec::new(),
            }),
        );
    }
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::{CS, state};
    use dispatched_core::jobs::ItemOutcome;

    fn item(seq: i64) -> ItemResult {
        ItemResult {
            sequence_number: Some(seq),
            message_id: None,
            session_id: None,
            entity: "orders".into(),
            new_sequence_number: None,
            outcome: ItemOutcome::Succeeded { detail: None },
        }
    }

    fn sample_report(items: Vec<ItemResult>) -> JobReport {
        let now = OffsetDateTime::now_utc();
        JobReport {
            op_id: Some("op-1".into()),
            kind: JobKind::Delete,
            entity: "orders".into(),
            target: None,
            started_at: now,
            finished_at: now,
            cancelled: false,
            items,
            notes: vec![],
            not_attempted: 0,
        }
    }

    async fn connected() -> (AppState, Arc<Recorder>, String) {
        let s = state();
        let ev = Arc::new(Recorder::default());
        let p = s
            .store
            .add_connection_string("Logistics", CS, s.secrets.as_ref(), false)
            .unwrap();
        crate::ops::connection::connect(&s, &*ev, &p.id)
            .await
            .unwrap();
        (s, ev, p.id)
    }

    async fn finished(ev: &Recorder) -> JobEvent {
        for _ in 0..200 {
            if let Some(f) = ev
                .jobs
                .lock()
                .unwrap()
                .iter()
                .find(|j| matches!(j, JobEvent::Finished { .. }))
            {
                return f.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the job never finished");
    }

    #[tokio::test]
    async fn runner_streams_items_and_stores_the_report() {
        let (s, ev, profile) = connected().await;
        let started = start_op(
            &s,
            ev.clone(),
            &profile,
            JobKind::Delete,
            "orders".into(),
            "Delete 2".into(),
            move |_op, b, _c| async move {
                b.event(&CoreEvent::Item(item(9_007_199_254_740_993)));
                Ok(Done::Report(
                    sample_report(vec![item(9_007_199_254_740_993), item(3)]),
                    vec![],
                ))
            },
        )
        .await
        .unwrap();
        let JobEvent::Finished { error, report, .. } = finished(&ev).await else {
            unreachable!()
        };
        assert!(error.is_none());
        assert_eq!(report.unwrap().succeeded, 2);
        let jobs = ev.jobs.lock().unwrap().clone();
        assert!(matches!(&jobs[0], JobEvent::Started { kind, .. } if kind == "delete"));
        assert!(matches!(&jobs[1], JobEvent::Item { item, .. }
            if item.sequence_number.as_deref() == Some("9007199254740993")));
        let page = job_report(&s.jobs, &started.id, 0, 10, false).unwrap();
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn close_running_keeps_partial_report() {
        let (s, ev, profile) = connected().await;
        let gate = Arc::new(tokio::sync::Notify::new());
        let g = gate.clone();
        let started = start_op(
            &s,
            ev.clone(),
            &profile,
            JobKind::Delete,
            "orders".into(),
            "Delete".into(),
            move |_op, b, c| async move {
                b.event(&CoreEvent::Item(item(1)));
                g.notified().await;
                assert!(c.is_cancelled());
                Err(dispatched_core::Error::Cancelled)
            },
        )
        .await
        .unwrap();
        assert_eq!(s.jobs.close_running(), 1);
        gate.notify_one();
        let JobEvent::Finished { error, report, .. } = finished(&ev).await else {
            unreachable!()
        };
        assert_eq!(error.unwrap().kind, "stopped");
        assert_eq!(report.unwrap().succeeded, 1);
        assert_eq!(
            job_report(&s.jobs, &started.id, 0, 10, false)
                .unwrap()
                .total,
            1
        );
    }

    #[tokio::test]
    async fn a_cancelled_report_ends_stopped_with_its_report() {
        let (s, ev, profile) = connected().await;
        let gate = Arc::new(tokio::sync::Notify::new());
        let g = gate.clone();
        start_op(
            &s,
            ev.clone(),
            &profile,
            JobKind::Delete,
            "orders".into(),
            "Delete".into(),
            move |_op, _b, c| async move {
                g.notified().await;
                assert!(c.is_cancelled());
                // Real operations end a cancelled run with a report, not an error.
                let mut r = sample_report(vec![item(1)]);
                r.cancelled = true;
                Ok(Done::Report(r, vec![]))
            },
        )
        .await
        .unwrap();
        s.jobs.close_running();
        gate.notify_one();
        let JobEvent::Finished { error, report, .. } = finished(&ev).await else {
            unreachable!()
        };
        assert_eq!(error.unwrap().kind, "stopped");
        assert_eq!(report.unwrap().succeeded, 1);
    }

    #[tokio::test]
    async fn a_user_cancel_ends_cancelled() {
        let (s, ev, profile) = connected().await;
        let started = start_op(
            &s,
            ev.clone(),
            &profile,
            JobKind::Purge,
            "orders".into(),
            "Purge".into(),
            move |_op, _b, c| async move {
                c.cancelled().await;
                Err(dispatched_core::Error::Cancelled)
            },
        )
        .await
        .unwrap();
        assert!(s.jobs.cancel(&started.id));
        let JobEvent::Finished { error, report, .. } = finished(&ev).await else {
            unreachable!()
        };
        assert_eq!(error.unwrap().kind, "cancelled");
        assert!(report.is_none());
    }

    #[tokio::test]
    async fn a_job_needs_the_profile_to_be_connected() {
        let s = state();
        let ev: Arc<dyn Events> = Arc::new(Recorder::default());
        let e = start_op(
            &s,
            ev,
            "nope",
            JobKind::Send,
            "q".into(),
            "Send".into(),
            move |_op, _b, _c| async move { Ok(Done::Plain("never".into())) },
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, "not_connected");
        assert!(!s.jobs.any_running());
    }

    #[tokio::test]
    async fn purge_runs_stored_plan_once() {
        let (s, ev, profile) = connected().await;
        let plan = dispatched_core::ops::PurgePlan {
            entity: EntityPath::queue("orders"),
            scope: PurgeScope::Active,
            per_queue: vec![(
                SubQueue::Active,
                dispatched_core::safety::CountEstimate::Exact(0),
            )],
            deferred: 0,
            deferred_truncated: false,
            total: dispatched_core::safety::CountEstimate::Exact(0),
            requires_session: Some(false),
        };
        let id = s.jobs.plan(&profile, vec![plan.clone()]);
        assert_eq!(
            purge(&s, ev.clone(), "other-profile", &id)
                .await
                .unwrap_err()
                .kind,
            "not_found",
            "a plan runs on the profile it was counted on"
        );
        let id = s.jobs.plan(&profile, vec![plan]);
        let first = purge(&s, ev.clone(), &profile, &id).await;
        assert!(first.is_ok());
        s.jobs.cancel(&first.unwrap().id);
        assert_eq!(
            purge(&s, ev, &profile, &id).await.unwrap_err().kind,
            "not_found"
        );
    }

    #[tokio::test]
    async fn session_state_over_the_limit_needs_allow_large() {
        let (s, ev, profile) = connected().await;
        use base64::Engine as _;
        let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; STATE_LIMIT + 1]);
        let e = session_state_set(&s, ev, &profile, "orders", "a", Some(big), false)
            .await
            .unwrap_err();
        assert!(e.message.contains("256 KB"));
    }

    #[test]
    fn report_save_writes_json() {
        let jobs = Jobs::default();
        let (id, _) = jobs.start();
        jobs.set_report(
            &id,
            Arc::new(StoredReport {
                report: sample_report(vec![item(5)]),
                messages: vec![],
            }),
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let pick = jobs.pick(path.clone());
        save_report(&jobs, &id, &pick, "json").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"sequence_number\""), "{text}");
        assert_eq!(
            save_report(&jobs, &id, &pick, "json").unwrap_err().kind,
            "invalid",
            "a pick is spent once"
        );
    }
}
