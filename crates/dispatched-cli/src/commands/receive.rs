//! `dispatched receive` and `deferred ls|receive`.

use super::browse::{ViewArgs, messages_table, peek};
use super::operator_for;
use crate::Format;
use crate::context::Ctx;
use crate::jobs::{JobExit, StderrProgress, finish_job, finish_job_with_messages};
use crate::resolve::Selection;
use crate::safety::{MutateArgs, Planned};
use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use dispatched_core::messaging::{DEFAULT_SCAN_LIMIT, Settlement};
use dispatched_core::ops::receive::MAX_RECEIVE;
use dispatched_core::{EntityPath, Mutation, PeekView, ReceiveAction, ReceiveRequest, SubQueue};
use std::time::Duration;

/// The longest `--wait`, in seconds.
pub const MAX_WAIT_SECS: u64 = 3_600;

/// How to settle each message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Then {
    /// Remove it.
    Complete,
    /// Unlock it (its delivery count goes up).
    Abandon,
    /// Set it aside; receive it later by sequence number.
    Defer,
    /// Move it to the dead-letter queue (--reason, --description).
    DeadLetter,
}

/// `receive`. Exactly one of `--then` and `--delete` is required: there is no default action
/// (spec §5).
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("action").required(true).args(["then", "delete"])))]
pub struct ReceiveArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Receive from the dead-letter queue.
    #[arg(long, conflicts_with = "transfer_dlq")]
    pub dlq: bool,
    /// Receive from the transfer dead-letter queue.
    #[arg(long)]
    pub transfer_dlq: bool,
    /// Accept this session (session-enabled entities).
    #[arg(long)]
    pub session: Option<String>,
    /// At most this many messages (1 to 5,000).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=MAX_RECEIVE as i64))]
    pub top: u32,
    /// Stop after this many seconds pass without a message (an idle timeout, not a total).
    #[arg(long, value_name = "SECS", default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=MAX_WAIT_SECS))]
    pub wait: u64,
    /// Peek-lock each message, then settle it this way.
    #[arg(long, value_enum)]
    pub then: Option<Then>,
    /// With --then dead-letter: the DeadLetterReason.
    #[arg(long)]
    pub reason: Option<String>,
    /// With --then dead-letter: the DeadLetterErrorDescription.
    #[arg(long)]
    pub description: Option<String>,
    /// Receive and delete: each message is removed as it is delivered.
    #[arg(long)]
    pub delete: bool,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// `deferred`: deferred messages.
#[derive(Subcommand, Debug)]
pub enum DeferredCmd {
    /// List deferred messages (`peek --view deferred`).
    Ls(DeferredLsArgs),
    /// Receive deferred messages by sequence number and settle them.
    Receive(DeferredReceiveArgs),
}

/// `deferred ls`.
#[derive(Args, Debug)]
pub struct DeferredLsArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// Maximum messages.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..))]
    pub top: u32,
}

/// `deferred receive`.
#[derive(Args, Debug)]
pub struct DeferredReceiveArgs {
    /// `queue` or `topic/subscriptions/subscription`.
    pub entity: String,
    /// A sequence number (repeat, or separate with commas).
    #[arg(long, value_name = "N", value_delimiter = ',', required = true)]
    pub seq: Vec<i64>,
    /// How to settle each message.
    #[arg(long, value_enum)]
    pub then: Then,
    /// With --then dead-letter: the DeadLetterReason.
    #[arg(long)]
    pub reason: Option<String>,
    /// With --then dead-letter: the DeadLetterErrorDescription.
    #[arg(long)]
    pub description: Option<String>,
    /// The session the messages are in (session-enabled entities).
    #[arg(long)]
    pub session: Option<String>,
    /// The messages are deferred in the dead-letter queue.
    #[arg(long)]
    pub dlq: bool,
    #[command(flatten)]
    pub mutate: MutateArgs,
}

/// The settlement `then` names; `--reason` and `--description` belong to dead-letter only.
fn settlement(
    then: Then,
    reason: &Option<String>,
    description: &Option<String>,
) -> anyhow::Result<Settlement> {
    if then != Then::DeadLetter && (reason.is_some() || description.is_some()) {
        anyhow::bail!("--reason and --description go with --then dead-letter");
    }
    Ok(match then {
        Then::Complete => Settlement::Complete,
        Then::Abandon => Settlement::Abandon,
        Then::Defer => Settlement::Defer,
        Then::DeadLetter => Settlement::DeadLetter {
            // Not given: not set (an empty reason would read as a reason that says nothing).
            reason: reason.clone(),
            description: description.clone(),
        },
    })
}

/// The receive request the flags describe (no I/O).
pub fn receive_request_from(a: &ReceiveArgs) -> anyhow::Result<ReceiveRequest> {
    let action = match (a.then, a.delete) {
        (Some(then), false) => ReceiveAction::Settle(settlement(then, &a.reason, &a.description)?),
        (None, true) => {
            if a.reason.is_some() || a.description.is_some() {
                anyhow::bail!("--reason and --description go with --then dead-letter");
            }
            ReceiveAction::Delete
        }
        // clap requires exactly one; kept so the function is total.
        _ => anyhow::bail!("pass exactly one of --then and --delete"),
    };
    Ok(ReceiveRequest {
        entity: EntityPath::parse(&a.entity)?,
        sub_queue: match (a.dlq, a.transfer_dlq) {
            (true, _) => SubQueue::DeadLetter,
            (_, true) => SubQueue::TransferDeadLetter,
            _ => SubQueue::Active,
        },
        session: a.session.clone(),
        max: a.top,
        wait: Duration::from_secs(a.wait),
        action,
    })
}

/// The planned kind and verb of a settlement (as [`dispatched_core::Operation`] has them).
fn planned_settle(how: &Settlement) -> Planned {
    Planned {
        kind: Mutation::Settle,
        verb: match how {
            Settlement::Complete => "complete",
            Settlement::Abandon => "abandon",
            Settlement::Defer => "defer",
            Settlement::DeadLetter { .. } => "dead-letter",
        },
    }
}

/// Receive-and-delete.
pub const RECEIVE_AND_DELETE: Planned = Planned {
    kind: Mutation::Delete,
    verb: "receive-and-delete",
};

/// The planned kind and verb of a receive.
pub fn planned_receive(action: &ReceiveAction) -> Planned {
    match action {
        ReceiveAction::Delete => RECEIVE_AND_DELETE,
        ReceiveAction::Settle(how) => planned_settle(how),
    }
}

/// The browse view whose table shows messages of `sub_queue`.
fn view_of(sub_queue: SubQueue) -> PeekView {
    match sub_queue {
        SubQueue::Active => PeekView::Active,
        SubQueue::DeadLetter => PeekView::DeadLetter,
        SubQueue::TransferDeadLetter => PeekView::TransferDeadLetter,
    }
}

pub async fn receive(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &ReceiveArgs,
    format: Format,
) -> anyhow::Result<JobExit> {
    let req = receive_request_from(args)?;
    let planned = planned_receive(&req.action);
    let view = view_of(req.sub_queue);
    let (operator, cancel) = operator_for(
        ctx,
        sel,
        &args.mutate,
        planned,
        std::slice::from_ref(&req.entity),
    )?;
    let result = operator.receive(req, &StderrProgress::new(), &cancel).await;
    operator.explorer().close().await;
    let outcome = result?;
    if let Format::Table = format {
        println!("{}", messages_table(&outcome.messages, view));
        if outcome.messages.is_empty() {
            eprintln!("(no messages)");
        } else if let Some(note) = shortened_note(&args.mutate, args.delete) {
            eprintln!("{note}");
        }
    }
    // JSON: one document with the messages in the `peek --format json` shape; the --report
    // file holds them in full in both modes.
    Ok(finish_job_with_messages(
        &outcome.report,
        format,
        &args.mutate,
        &outcome.messages,
    ))
}

/// The table shortens bodies: a receive-and-delete without --report says how to keep full
/// copies of what it removed.
pub const SHORTENED_NOTE: &str =
    "bodies are shortened above; use --format json or --report FILE to keep full copies";

/// [`SHORTENED_NOTE`] for a table-format `receive --delete` with no `--report`.
fn shortened_note(mutate: &MutateArgs, delete: bool) -> Option<&'static str> {
    (delete && mutate.report.is_none()).then_some(SHORTENED_NOTE)
}

pub async fn deferred(
    ctx: &Ctx,
    sel: &Selection<'_>,
    cmd: &DeferredCmd,
    format: Format,
) -> anyhow::Result<JobExit> {
    match cmd {
        DeferredCmd::Ls(a) => {
            let view = ViewArgs {
                entity: Some(a.entity.clone()),
                view: Some("deferred".into()),
                scan_limit: DEFAULT_SCAN_LIMIT,
                ..ViewArgs::default()
            };
            peek(ctx, sel, &view, a.top, false, format).await?;
            Ok(JobExit::Ok)
        }
        DeferredCmd::Receive(a) => {
            let how = settlement(a.then, &a.reason, &a.description)?;
            let entity = EntityPath::parse(&a.entity)?;
            let (operator, cancel) = operator_for(
                ctx,
                sel,
                &a.mutate,
                planned_settle(&how),
                std::slice::from_ref(&entity),
            )?;
            let result = operator
                .settle_deferred(
                    &entity,
                    if a.dlq {
                        SubQueue::DeadLetter
                    } else {
                        SubQueue::Active
                    },
                    &a.seq,
                    a.session.as_deref(),
                    how,
                    &StderrProgress::new(),
                    &cancel,
                )
                .await;
            operator.explorer().close().await;
            Ok(finish_job(&result?, format, &a.mutate))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn try_parse<const N: usize>(args: [&str; N]) -> Result<crate::Cli, clap::Error> {
        crate::Cli::try_parse_from(std::iter::once("dispatched").chain(args))
    }

    fn parse<const N: usize>(args: [&str; N]) -> ReceiveArgs {
        match try_parse(args).unwrap().command {
            crate::Command::Receive(a) => a,
            _ => panic!("not receive"),
        }
    }

    #[test]
    fn receive_requires_an_explicit_action() {
        assert!(try_parse(["receive", "q"]).is_err(), "no default action");
        assert!(try_parse(["receive", "q", "--then", "complete", "--delete"]).is_err());
        let r = receive_request_from(&parse([
            "receive",
            "q",
            "--dlq",
            "--top",
            "5",
            "--then",
            "dead-letter",
            "--reason",
            "Manual",
        ]))
        .unwrap();
        assert_eq!(r.sub_queue, SubQueue::DeadLetter);
        assert!(
            matches!(r.action, ReceiveAction::Settle(Settlement::DeadLetter { ref reason, .. }) if reason.as_deref() == Some("Manual"))
        );
        assert!(matches!(
            receive_request_from(&parse(["receive", "q", "--delete"]))
                .unwrap()
                .action,
            ReceiveAction::Delete
        ));
        assert!(try_parse(["receive", "q", "--delete", "--top", "6000"]).is_err());
    }

    #[test]
    fn planned_kinds_match_the_operations() {
        use dispatched_core::{Operation, SettleVerb, Target};
        let t = || Target::new(EntityPath::queue("q"), SubQueue::Active);
        let dl = Settlement::DeadLetter {
            reason: None,
            description: None,
        };
        for (planned, op) in [
            (
                planned_receive(&ReceiveAction::Delete),
                Operation::receive_and_delete(t(), 1),
            ),
            (
                planned_receive(&ReceiveAction::Settle(Settlement::Complete)),
                Operation::receive_and_settle(t(), 1, SettleVerb::Complete),
            ),
            (
                planned_settle(&Settlement::Abandon),
                Operation::settle_deferred(t(), 1, SettleVerb::Abandon),
            ),
            (
                planned_settle(&Settlement::Defer),
                Operation::settle_deferred(t(), 1, SettleVerb::Defer),
            ),
            (
                planned_settle(&dl),
                Operation::settle_deferred(t(), 1, SettleVerb::DeadLetter),
            ),
        ] {
            assert_eq!((planned.kind, planned.verb), (op.mutation, op.verb()));
        }
    }

    #[test]
    fn reason_goes_with_dead_letter_only_and_wait_is_bounded() {
        let e = receive_request_from(&parse([
            "receive", "q", "--then", "complete", "--reason", "r",
        ]))
        .unwrap_err()
        .to_string();
        assert!(e.contains("--then dead-letter"), "{e}");
        assert!(
            receive_request_from(&parse(["receive", "q", "--delete", "--reason", "r"])).is_err()
        );
        let r = receive_request_from(&parse([
            "receive",
            "events/subscriptions/all",
            "--transfer-dlq",
            "--session",
            "s1",
            "--wait",
            "30",
            "--then",
            "abandon",
        ]))
        .unwrap();
        assert_eq!(r.sub_queue, SubQueue::TransferDeadLetter);
        assert_eq!(r.entity, EntityPath::subscription("events", "all"));
        assert_eq!(r.session.as_deref(), Some("s1"));
        assert_eq!(r.wait, Duration::from_secs(30));
        assert_eq!(r.max, 1);
        assert!(try_parse(["receive", "q", "--delete", "--wait", "0"]).is_err());
        assert!(try_parse(["receive", "q", "--delete", "--dlq", "--transfer-dlq"]).is_err());
    }

    #[test]
    fn deferred_commands_parse() {
        let cli = try_parse([
            "deferred", "receive", "q", "--seq", "5,6", "--then", "complete",
        ])
        .unwrap();
        let crate::Command::Deferred {
            cmd: DeferredCmd::Receive(a),
        } = cli.command
        else {
            panic!("not deferred receive")
        };
        assert_eq!(a.seq, [5, 6]);
        assert!(
            try_parse(["deferred", "receive", "q", "--then", "complete"]).is_err(),
            "--seq needed"
        );
        assert!(
            try_parse(["deferred", "receive", "q", "--seq", "5"]).is_err(),
            "--then needed"
        );
        assert!(try_parse(["deferred", "ls", "q", "--top", "3"]).is_ok());
        assert!(try_parse(["deferred", "delete", "q"]).is_err(), "Task 20");
    }

    #[tokio::test]
    async fn receive_on_production_without_flags_never_connects() {
        let ctx = crate::context::Ctx::for_test_with_profiles(&[(
            "prod-weu",
            "prod.servicebus.invalid",
            true,
        )]);
        let cs = "Endpoint=sb://prod.servicebus.invalid/;SharedAccessKeyName=k;SharedAccessKey=topsecret";
        for (args, verb, kind) in [
            (
                vec!["receive", "orders", "--delete"],
                "receive-and-delete",
                Mutation::Delete,
            ),
            (
                vec!["receive", "orders", "--dlq", "--then", "complete"],
                "complete",
                Mutation::Settle,
            ),
            (
                vec![
                    "deferred", "receive", "orders", "--seq", "3", "--then", "defer",
                ],
                "defer",
                Mutation::Settle,
            ),
        ] {
            let mut argv = vec!["dispatched", "--connection-string", cs];
            argv.extend(args);
            let cli = crate::Cli::try_parse_from(argv).unwrap();
            let list = || {
                ctx.activity()
                    .unwrap()
                    .list(&dispatched_core::ActivityQuery::default())
                    .unwrap()
            };
            let before = list().len();
            let e = crate::run_command(&ctx, cli).await.unwrap_err().to_string();
            assert!(e.contains("--confirm-prod orders"), "{e}");
            assert!(!e.contains("topsecret"));
            let log = list();
            assert_eq!(log.len(), before + 1, "one entry: the precheck's");
            assert_eq!(log[0].action, verb);
            assert_eq!(log[0].kind, dispatched_core::OpKind::Mutating(kind));
            assert_eq!(log[0].phase, dispatched_core::Phase::Finished);
            // The precheck's own marker: the gate's denial would carry the effect sentence.
            assert_eq!(
                log[0].summary,
                format!("{verb} on orders refused before connecting")
            );
            assert_eq!(log[0].error.as_deref(), Some(e.as_str()));
            assert!(!format!("{:?}", log[0]).contains("topsecret"));
        }
    }

    #[test]
    fn a_table_receive_and_delete_without_a_report_says_how_to_keep_copies() {
        let none = MutateArgs::default();
        assert_eq!(shortened_note(&none, true), Some(SHORTENED_NOTE));
        assert_eq!(shortened_note(&none, false), None, "settled messages stay");
        let with_report = MutateArgs {
            report: Some("r.json".into()),
            ..MutateArgs::default()
        };
        assert_eq!(
            shortened_note(&with_report, true),
            None,
            "the report keeps them"
        );
    }

    #[test]
    fn dead_letter_reason_is_absent_unless_given() {
        let r = receive_request_from(&parse(["receive", "q", "--then", "dead-letter"])).unwrap();
        assert_eq!(
            r.action,
            ReceiveAction::Settle(Settlement::DeadLetter {
                reason: None,
                description: None
            })
        );
        let cli = try_parse([
            "deferred", "receive", "q", "--seq", "1", "--then", "complete", "--dlq",
        ])
        .unwrap();
        let crate::Command::Deferred {
            cmd: DeferredCmd::Receive(a),
        } = cli.command
        else {
            panic!("not deferred receive")
        };
        assert!(a.dlq);
    }
}
