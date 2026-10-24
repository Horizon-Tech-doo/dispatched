//! `dispatched mcp`: an MCP server over stdio, for agents.
//!
//! Reads go straight to the core's explorer (peeks never lock, receive or settle). Every change
//! is the same core `Operation` the CLI builds, through a gate whose policy asks the human in
//! the agent's client ([`policy::McpPolicy`]) and whose activity entries say `mcp`. Production
//! is decided by the saved profiles, never by the agent. `--read-only` leaves the write tools
//! out of the tool list.

mod diagnose;
mod policy;
mod read;
mod write;

#[cfg(test)]
mod tests;

use crate::context::Ctx;
use crate::resolve::{OwnedSelection, Selection, resolve_selected};
use anyhow::Context as _;
use dispatched_core::{Explorer, ProfileRef};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool_handler};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What a tool returns: a result the agent reads (an error included), or invalid arguments.
pub(crate) type ToolResult = Result<CallToolResult, ErrorData>;

/// Invalid arguments: a protocol error, so the agent fixes the call.
pub(crate) fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}

/// The text the CLI prints for `e` (what to do, secrets redacted).
pub(crate) fn error_text(e: &anyhow::Error) -> String {
    crate::format_error(e, crate::Hints::default())
}

/// A tool's outcome as a result the agent can read: the JSON, or `is_error` with the CLI's
/// message for the failure.
pub(crate) fn reply(result: anyhow::Result<serde_json::Value>) -> ToolResult {
    Ok(match result {
        Ok(value) => CallToolResult::structured(value),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(error_text(&e))]),
    })
}

/// A namespace the server talks to, with the connection it keeps open.
pub(crate) struct Target {
    pub explorer: Arc<Explorer>,
    /// The profile name, or the host for a connection string or `--namespace`.
    pub name: String,
    /// The namespace host.
    pub host: String,
    /// Resolved from a saved profile.
    pub from_profile: bool,
}

impl Target {
    fn new(explorer: Explorer, name: String, from_profile: bool) -> Self {
        let host = explorer.namespace().host().to_string();
        Self {
            explorer: Arc::new(explorer),
            name,
            host,
            from_profile,
        }
    }
}

struct Inner {
    ctx: Ctx,
    /// The namespace of the CLI's selection, resolved once at start; the error text when it
    /// could not be (tools then need a `profile`).
    default: Result<Arc<Target>, String>,
    /// Targets of the `profile` argument, by lower-case name.
    profiles: Mutex<HashMap<String, Arc<Target>>>,
}

/// The MCP server.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
    tool_router: ToolRouter<Server>,
}

impl Server {
    /// A server over `ctx` whose default namespace is the selection `sel` (flags, environment,
    /// then the default profile, as for every other command). Opens no connection.
    pub fn new(ctx: Ctx, sel: &Selection<'_>, read_only: bool) -> Self {
        let from_profile = sel.connection_string.is_none() && sel.namespace.is_none();
        let default = resolve_selected(&ctx, sel)
            .and_then(|r| {
                let explorer = Explorer::new(r.namespace)?;
                Ok(Arc::new(Target::new(
                    explorer,
                    r.profile.name,
                    from_profile,
                )))
            })
            .map_err(|e| error_text(&e));
        let mut tool_router = Self::read_router() + Self::diagnose_router();
        if !read_only {
            tool_router += Self::write_router();
        }
        Self {
            inner: Arc::new(Inner {
                ctx,
                default,
                profiles: Mutex::new(HashMap::new()),
            }),
            tool_router,
        }
    }

    pub(crate) fn ctx(&self) -> &Ctx {
        &self.inner.ctx
    }

    /// The target of a call: the saved profile `profile`, or the server's default. A profile is
    /// resolved on first use and kept, with its connection, for the server's life.
    pub(crate) fn target(&self, profile: Option<&str>) -> anyhow::Result<Arc<Target>> {
        let Some(name) = profile.map(str::trim).filter(|p| !p.is_empty()) else {
            return self.inner.default.clone().map_err(|e| {
                anyhow::anyhow!("{e}; pass `profile` with a saved profile name (see list_profiles)")
            });
        };
        let key = name.to_lowercase();
        if let Some(t) = self.lock_profiles().get(&key) {
            return Ok(t.clone());
        }
        let sel = Selection {
            connection_string: None,
            namespace: None,
            profile: Some(name),
        };
        let resolved = resolve_selected(self.ctx(), &sel)?;
        let explorer = Explorer::new(resolved.namespace)?;
        let target = Arc::new(Target::new(explorer, resolved.profile.name, true));
        // A concurrent first use keeps the first one.
        Ok(self.lock_profiles().entry(key).or_insert(target).clone())
    }

    fn lock_profiles(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Target>>> {
        self.inner
            .profiles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// The profile a change on `target` is gated and logged under, with production read from
    /// the saved profiles now (a profile marked production while the server runs counts at
    /// once): the target's own profile, or any saved production profile on the same host. If
    /// the profiles cannot be read, nothing runs.
    pub(crate) fn profile_ref(&self, target: &Target) -> anyhow::Result<ProfileRef> {
        let check = || -> anyhow::Result<bool> {
            Ok(self.ctx().store()?.list()?.iter().any(|p| {
                p.production
                    && (p.host.eq_ignore_ascii_case(&target.host)
                        || (target.from_profile && p.name.eq_ignore_ascii_case(&target.name)))
            }))
        };
        let production = check().with_context(|| {
            format!(
                "cannot check whether {} is saved as a production profile; nothing was changed",
                target.host
            )
        })?;
        Ok(ProfileRef {
            name: target.name.clone(),
            production,
        })
    }

    /// Close every connection the server opened.
    pub async fn close(&self) {
        if let Ok(t) = &self.inner.default {
            t.explorer.close().await;
        }
        let targets: Vec<Arc<Target>> = self.lock_profiles().values().cloned().collect();
        for t in targets {
            t.explorer.close().await;
        }
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "dispatched",
    instructions = "Dispatched explores and repairs Azure Service Bus namespaces. Read tools peek \
        and never lock, receive or settle a message. Every change asks the human to confirm in \
        this client (on production profiles by typing the entity name) and is written to \
        Dispatched's activity log. Entities are named `queue`, `topic` or \
        `topic/subscriptions/subscription`. Pass `profile` to use another saved profile than the \
        server's default (see list_profiles)."
)]
impl ServerHandler for Server {}

/// `dispatched mcp`: serve on stdin and stdout until the client disconnects.
pub async fn serve(ctx: Ctx, sel: &OwnedSelection, read_only: bool) -> anyhow::Result<()> {
    let server = Server::new(ctx, &sel.borrow(), read_only);
    if let Err(e) = &server.inner.default {
        // stdout is the protocol; notes go to stderr, which clients keep as the server log.
        eprintln!("dispatched mcp: no default namespace ({e}); tools need a `profile` argument");
    }
    let running = server
        .clone()
        .serve(rmcp::transport::stdio())
        .await
        .context("starting the MCP server")?;
    let ended = running.waiting().await;
    server.close().await;
    ended.context("the MCP server stopped")?;
    Ok(())
}
