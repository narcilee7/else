//! Gateway 资源边界：有效 Lease + 调用前预算校验。

use std::collections::BTreeSet;

use else_ledger::{Ledger, Projection, ThreadProjection};
use else_protocol::event::EventPayload;
use else_protocol::resource::ModelUsage;

use crate::{ExecContext, GatewayError, Result};

pub fn ensure_active_lease(ledger: &Ledger, ctx: &ExecContext) -> Result<()> {
    let events = ledger.read_thread(&ctx.spec.id)?;
    let revoked: BTreeSet<_> = events
        .iter()
        .filter_map(|event| match &event.envelope.payload {
            EventPayload::LeaseRevoked { lease_id } => Some(lease_id.clone()),
            _ => None,
        })
        .collect();
    let valid = events.iter().any(|event| match &event.envelope.payload {
        EventPayload::LeaseGranted { lease } => {
            lease.scope_id == ctx.scope_id
                && lease.run_id.as_ref() == Some(&ctx.run_id)
                && lease.generation == ctx.generation
                && lease.expires_at > chrono::Utc::now()
                && !revoked.contains(&lease.id)
        }
        _ => false,
    });
    if valid {
        Ok(())
    } else {
        Err(GatewayError::LeaseRequired(format!(
            "scope={} run={} generation={}",
            ctx.scope_id, ctx.run_id, ctx.generation
        )))
    }
}

pub fn ensure_model_capacity(
    ledger: &Ledger,
    ctx: &ExecContext,
    estimate: ModelUsage,
) -> Result<()> {
    let projection = projection(ledger, ctx)?;
    let tokens = estimate.input_tokens.saturating_add(estimate.output_tokens);
    ensure_capacity(
        "model_tokens",
        tokens,
        projection.resource_budget.max_model_tokens,
        projection.resource_usage.model_tokens,
        "tokens",
    )?;
    ensure_capacity(
        "model_spend",
        estimate.cost_usd_micros,
        projection.resource_budget.max_model_spend_usd_micros,
        projection.resource_usage.model_spend_usd_micros,
        "usd_micros",
    )
}

pub fn ensure_tool_capacity(ledger: &Ledger, ctx: &ExecContext) -> Result<()> {
    let projection = projection(ledger, ctx)?;
    ensure_capacity(
        "tool_calls",
        1,
        projection.resource_budget.max_tool_calls,
        projection.resource_usage.tool_calls,
        "calls",
    )
}

pub fn ensure_capacity(
    resource: &str,
    requested: u64,
    limit: u64,
    used: u64,
    unit: &'static str,
) -> Result<()> {
    let remaining = limit.saturating_sub(used);
    if requested > remaining {
        return Err(GatewayError::ResourceExhausted {
            resource: resource.to_string(),
            requested,
            remaining,
            unit,
        });
    }
    Ok(())
}

fn projection(ledger: &Ledger, ctx: &ExecContext) -> Result<ThreadProjection> {
    let mut projection = ThreadProjection::default();
    for event in ledger.read_thread(&ctx.spec.id)? {
        projection.apply(&event);
    }
    Ok(projection)
}
