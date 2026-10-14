//! `dispatched ls` and `dispatched peek`.

use crate::Format;
use crate::context::Ctx;
use crate::output::{ellipsize, table};
use crate::resolve::{Selection, needs_sign_in, resolve};
use dispatched_core::{
    EntityCounts, EntityPath, Explorer, MessageBody, Namespace, PeekOptions, PeekView,
    PeekedMessage, SubQueue,
};
use time::format_description::well_known::Rfc3339;

/// The namespace the user selected. The sign-in (a keychain read) is only
/// looked up when the selection uses Entra.
fn namespace(ctx: &Ctx, sel: &Selection<'_>) -> anyhow::Result<Namespace> {
    let sign_in = if needs_sign_in(sel, &ctx.store)? {
        ctx.current_sign_in()?
    } else {
        None
    };
    resolve(
        sel,
        &ctx.store,
        ctx.secrets.as_ref(),
        &ctx.session,
        sign_in.as_ref(),
    )
}

/// A count cell: `?` when unknown, `~N` when counted by peeking, `~N+` when
/// that count hit its cap.
fn count_cell(counts: Option<&EntityCounts>, pick: fn(&EntityCounts) -> u64) -> String {
    match counts {
        None => "?".into(),
        Some(c) => {
            let n = pick(c);
            match (c.approximate, c.truncated) {
                (_, true) => format!("~{n}+"),
                (true, false) => format!("~{n}"),
                (false, false) => n.to_string(),
            }
        }
    }
}

fn body_cell(body: &MessageBody) -> String {
    match body.as_text() {
        Some(text) => ellipsize(&text, 60),
        None => format!("<{} bytes>", body.len()),
    }
}

fn print_messages(messages: &[PeekedMessage], format: Format, dlq: bool) -> anyhow::Result<()> {
    if let Format::Json = format {
        println!("{}", serde_json::to_string_pretty(messages)?);
        return Ok(());
    }
    let mut headers = vec!["SEQ", "ENQUEUED", "MESSAGE ID", "DELIV", "SUBJECT"];
    if dlq {
        headers.push("REASON");
    }
    headers.push("BODY");
    let rows: Vec<Vec<String>> = messages
        .iter()
        .map(|m| {
            let mut row = vec![
                m.sequence_number.to_string(),
                m.enqueued_time
                    .and_then(|t| t.format(&Rfc3339).ok())
                    .unwrap_or_default(),
                ellipsize(m.message_id.as_deref().unwrap_or(""), 36),
                m.delivery_count.to_string(),
                ellipsize(m.subject.as_deref().unwrap_or(""), 24),
            ];
            if dlq {
                row.push(ellipsize(m.dead_letter_reason.as_deref().unwrap_or(""), 32));
            }
            row.push(body_cell(&m.body));
            row
        })
        .collect();
    println!("{}", table(&headers, &rows));
    if messages.is_empty() {
        eprintln!("(no messages)");
    }
    Ok(())
}

pub async fn ls(
    ctx: &Ctx,
    sel: &Selection<'_>,
    topics: bool,
    format: Format,
) -> anyhow::Result<()> {
    let explorer = Explorer::new(namespace(ctx, sel)?)?;
    let result = list(&explorer, topics, format).await;
    explorer.close().await;
    result
}

async fn list(explorer: &Explorer, topics: bool, format: Format) -> anyhow::Result<()> {
    let queues = explorer.list_queues().await?;
    if let Format::Json = format {
        let mut value = serde_json::json!({ "queues": queues });
        if topics {
            let mut list = Vec::new();
            for t in explorer.list_topics().await? {
                let subs = explorer.list_subscriptions(&t.name).await?;
                list.push(serde_json::json!({ "topic": t, "subscriptions": subs }));
            }
            value["topics"] = serde_json::Value::Array(list);
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = queues
        .iter()
        .map(|q| {
            vec![
                q.name.clone(),
                count_cell(q.counts.as_ref(), |c| c.active),
                count_cell(q.counts.as_ref(), |c| c.dead_letter),
                count_cell(q.counts.as_ref(), |c| c.scheduled),
                if q.requires_session {
                    "yes".into()
                } else {
                    String::new()
                },
                q.status.clone().unwrap_or_default(),
            ]
        })
        .collect();
    println!(
        "{}",
        table(
            &["QUEUE", "ACTIVE", "DLQ", "SCHEDULED", "SESSIONS", "STATUS"],
            &rows
        )
    );
    if topics {
        let mut rows = Vec::new();
        for t in explorer.list_topics().await? {
            for s in explorer.list_subscriptions(&t.name).await? {
                rows.push(vec![
                    format!("{}/subscriptions/{}", t.name, s.name),
                    count_cell(s.counts.as_ref(), |c| c.active),
                    count_cell(s.counts.as_ref(), |c| c.dead_letter),
                    s.status.clone().unwrap_or_default(),
                ]);
            }
        }
        println!(
            "\n{}",
            table(&["SUBSCRIPTION", "ACTIVE", "DLQ", "STATUS"], &rows)
        );
    }
    Ok(())
}

pub async fn peek(
    ctx: &Ctx,
    sel: &Selection<'_>,
    entity: &str,
    view: PeekView,
    top: u32,
    from_seq: i64,
    format: Format,
) -> anyhow::Result<()> {
    let ns = namespace(ctx, sel)?;
    let explorer = Explorer::new(ns)?;
    let path = EntityPath::parse(entity)?;
    let page = explorer
        .peek(&path, view, &PeekOptions::new(top).from_sequence(from_seq))
        .await;
    explorer.close().await;
    let page = page?;
    print_messages(&page.messages, format, view.sub_queue() != SubQueue::Active)
}
