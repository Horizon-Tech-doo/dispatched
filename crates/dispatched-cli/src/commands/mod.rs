//! Subcommand handlers.

pub mod activity;
pub mod auth;
pub mod browse;
pub mod inspect;
pub mod profile;
pub mod receive;
pub mod repair;
pub mod send;
pub mod state;

use crate::context::Ctx;
use crate::resolve::Selection;
use crate::safety::{MutateArgs, Planned, cli_gate, preflight};
use dispatched_core::{EntityPath, Explorer, Operator};
use horizon_azure_kit::safety::CancellationToken;
use std::sync::Arc;

/// Everything a mutating command does before its [`Operator`] call, in order: resolve the
/// target, run [`preflight`] (report path, production flags; no I/O), build the gate, then the
/// explorer. Returns the operator and the job's cancel token (armed for Ctrl-C once the gate
/// confirmed). Callers close `operator.explorer()` when the call returns.
pub(crate) fn operator_for(
    ctx: &Ctx,
    sel: &Selection<'_>,
    args: &MutateArgs,
    planned: Planned,
    entities: &[EntityPath],
) -> anyhow::Result<(Operator, CancellationToken)> {
    let resolved = browse::mutating_target(ctx, sel)?;
    preflight(ctx, &resolved, args, planned, entities)?;
    let (gate, cancel) = cli_gate(ctx, &resolved, args)?;
    let explorer = Arc::new(Explorer::new(resolved.namespace)?);
    Ok((Operator::new(explorer, gate), cancel))
}
