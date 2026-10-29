//! M1-W3：Admission + Lease + Accounting。

use else_gateway::{resource::ensure_capacity, ExecContext};
use else_ledger::StoredEvent;
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::IntentId;
use else_protocol::resource::{ResourceBudget, ResourceClass, ResourceLease, ResourceUsage};

use crate::{Result, Runtime};

const RUN_MODEL_TOKENS: u64 = 128;
const RUN_MODEL_SPEND_MICROS: u64 = 1_500;
const RUN_TOOL_CALLS: u64 = 2;
const RUN_SANDBOX_SECONDS: u64 = 1;
const LEASE_TTL_SECONDS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceReport {
    pub budget: ResourceBudget,
    pub usage: ResourceUsage,
}

impl Runtime {
    /// 只影响之后创建的 Intent；既有 Intent 的预算已从事件持久化。
    pub fn set_default_budget(&mut self, budget: ResourceBudget) {
        self.default_budget = budget;
    }

    pub fn resources(&self, intent_id: &IntentId) -> Result<ResourceReport> {
        let projection = self.projection(intent_id)?;
        Ok(ResourceReport {
            budget: projection.resource_budget,
            usage: projection.resource_usage,
        })
    }

    /// Run 启动前做保守准入；任一资源不足则不签发 Lease、不启动执行。
    pub(crate) fn admit_run(&self, ctx: &ExecContext) -> Result<ResourceLease> {
        let projection = self.projection(&ctx.spec.id)?;
        let budget = projection.resource_budget;
        let usage = projection.resource_usage;
        ensure_capacity(
            "model_tokens",
            RUN_MODEL_TOKENS,
            budget.max_model_tokens,
            usage.model_tokens,
            "tokens",
        )?;
        ensure_capacity(
            "model_spend",
            RUN_MODEL_SPEND_MICROS,
            budget.max_model_spend_usd_micros,
            usage.model_spend_usd_micros,
            "usd_micros",
        )?;
        ensure_capacity(
            "tool_calls",
            RUN_TOOL_CALLS,
            budget.max_tool_calls,
            usage.tool_calls,
            "calls",
        )?;
        ensure_capacity(
            "sandbox_seconds",
            RUN_SANDBOX_SECONDS,
            budget.max_sandbox_seconds,
            usage.sandbox_seconds,
            "seconds",
        )?;
        let lease = ResourceLease {
            id: else_protocol::ids::LeaseId::new(),
            scope_id: ctx.scope_id.clone(),
            run_id: Some(ctx.run_id.clone()),
            class: ResourceClass::Sandbox,
            quantity: RUN_SANDBOX_SECONDS,
            unit: "seconds".into(),
            generation: ctx.generation,
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(LEASE_TTL_SECONDS),
        };
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::LeaseGranted {
                lease: lease.clone(),
            },
        )?;
        Ok(lease)
    }

    pub(crate) fn revoke_lease(&self, ctx: &ExecContext, lease: &ResourceLease) -> Result<()> {
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::LeaseRevoked {
                lease_id: lease.id.clone(),
            },
        )?;
        Ok(())
    }

    /// Abort 时回收该 Scope 所有未回收 Lease；重复恢复不会重复吊销。
    pub(crate) fn revoke_active_leases(
        &self,
        ctx: &ExecContext,
        events: &[StoredEvent],
    ) -> Result<()> {
        let revoked: std::collections::BTreeSet<_> = events
            .iter()
            .filter_map(|event| match &event.envelope.payload {
                EventPayload::LeaseRevoked { lease_id } => Some(lease_id.clone()),
                _ => None,
            })
            .collect();
        for event in events {
            let EventPayload::LeaseGranted { lease } = &event.envelope.payload else {
                continue;
            };
            if lease.scope_id == ctx.scope_id
                && lease.generation == ctx.generation
                && !revoked.contains(&lease.id)
            {
                self.revoke_lease(ctx, lease)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn budget_from_env() -> ResourceBudget {
    let mut budget = ResourceBudget::default();
    budget.max_model_tokens = env_u64("ELSE_MAX_MODEL_TOKENS", budget.max_model_tokens);
    budget.max_model_spend_usd_micros = env_u64(
        "ELSE_MAX_MODEL_SPEND_MICROS",
        budget.max_model_spend_usd_micros,
    );
    budget.max_tool_calls = env_u64("ELSE_MAX_TOOL_CALLS", budget.max_tool_calls);
    budget.max_sandbox_seconds = env_u64("ELSE_MAX_SANDBOX_SECONDS", budget.max_sandbox_seconds);
    budget
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
