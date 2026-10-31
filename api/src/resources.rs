//! M1-W3：Admission + Lease + Accounting。

use chrono::{DateTime, Utc};
use else_gateway::{resource::ensure_capacity, ExecContext};
use else_ledger::StoredEvent;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::IntentId;
use else_protocol::resource::{
    ResourceBudget, ResourceClass, ResourceLease, ResourceUsage, SandboxTerminationReason,
    SandboxUsage, SandboxUsageBasis,
};
use serde::Serialize;

use crate::{Result, Runtime};

const RUN_MODEL_TOKENS: u64 = 128;
const RUN_MODEL_SPEND_MICROS: u64 = 1_500;
const RUN_TOOL_CALLS: u64 = 2;
/// 与真实 Harness 的 15 分钟墙钟超时一致；Admission 按完整最坏时长预留。
const RUN_SANDBOX_SECONDS: u64 = 900;
const LEASE_TTL_SECONDS: i64 = 900;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceReport {
    pub budget: ResourceBudget,
    pub usage: ResourceUsage,
    pub sandboxes: Vec<SandboxUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeaseCleanupReport {
    pub scanned_intents: usize,
    pub orphan_leases: usize,
    pub revoked_leases: usize,
    pub sandbox_finish_records: usize,
    pub skipped_active_leases: usize,
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
            sandboxes: projection.sandbox_usages.into_values().collect(),
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
            usage.sandbox_committed_seconds(),
            "seconds",
        )?;
        let now = Utc::now();
        let lease = ResourceLease {
            id: else_protocol::ids::LeaseId::new(),
            scope_id: ctx.scope_id.clone(),
            run_id: Some(ctx.run_id.clone()),
            class: ResourceClass::Sandbox,
            quantity: RUN_SANDBOX_SECONDS,
            unit: "seconds".into(),
            generation: ctx.generation,
            granted_at: Some(now),
            expires_at: now + chrono::Duration::seconds(LEASE_TTL_SECONDS),
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

    /// 外部写动作没有 Sandbox，但仍需要 API Rate Limit Lease 与 Tool 预算。
    pub(crate) fn admit_external_action(&self, ctx: &ExecContext) -> Result<ResourceLease> {
        let projection = self.projection(&ctx.spec.id)?;
        ensure_capacity(
            "tool_calls",
            1,
            projection.resource_budget.max_tool_calls,
            projection.resource_usage.tool_calls,
            "calls",
        )?;
        let now = Utc::now();
        let lease = ResourceLease {
            id: else_protocol::ids::LeaseId::new(),
            scope_id: ctx.scope_id.clone(),
            run_id: Some(ctx.run_id.clone()),
            class: ResourceClass::ApiRateLimit,
            quantity: 1,
            unit: "call".into(),
            generation: ctx.generation,
            granted_at: Some(now),
            expires_at: now + chrono::Duration::seconds(60),
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

    /// Sandbox 真正开始接管执行时落账；开始后崩溃会留下可恢复证据。
    pub(crate) fn start_sandbox(
        &self,
        ctx: &ExecContext,
        lease: &ResourceLease,
    ) -> Result<DateTime<Utc>> {
        let started_at = Utc::now();
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::SandboxStarted {
                lease_id: lease.id.clone(),
                backend: sandbox_backend_name().into(),
                started_at,
            },
        )?;
        Ok(started_at)
    }

    /// Normal Run 结束由 Runtime 实测墙钟，证据等级为 Reported。
    pub(crate) fn finish_sandbox(
        &self,
        ctx: &ExecContext,
        lease: &ResourceLease,
        started_at: DateTime<Utc>,
        termination: SandboxTerminationReason,
        basis: SandboxUsageBasis,
    ) -> Result<()> {
        if self
            .projection(&ctx.spec.id)?
            .sandbox_usages
            .contains_key(&lease.id)
        {
            return Ok(());
        }
        let finished_at = Utc::now();
        let elapsed_millis = finished_at
            .signed_duration_since(started_at)
            .num_milliseconds()
            .max(0) as u64;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::SandboxFinished {
                usage: SandboxUsage {
                    lease_id: lease.id.clone(),
                    backend: sandbox_backend_name().into(),
                    started_at,
                    finished_at,
                    elapsed_millis,
                    basis,
                    termination,
                },
            },
        )?;
        Ok(())
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
                let started_at = events
                    .iter()
                    .find_map(|event| match &event.envelope.payload {
                        EventPayload::SandboxStarted {
                            lease_id,
                            started_at,
                            ..
                        } if lease_id == &lease.id => Some(*started_at),
                        _ => None,
                    });
                let (started_at, basis) = started_at.map_or(
                    (
                        lease.granted_at.unwrap_or(event.envelope.occurred_at),
                        SandboxUsageBasis::Estimated,
                    ),
                    |started_at| (started_at, SandboxUsageBasis::Unknown),
                );
                self.finish_sandbox(
                    ctx,
                    lease,
                    started_at,
                    SandboxTerminationReason::Aborted,
                    basis,
                )?;
                self.revoke_lease(ctx, lease)?;
            }
        }
        Ok(())
    }

    pub fn cleanup_orphan_leases(&self) -> Result<LeaseCleanupReport> {
        let specs = self.list_intents()?;
        let mut report = LeaseCleanupReport {
            scanned_intents: specs.len(),
            orphan_leases: 0,
            revoked_leases: 0,
            sandbox_finish_records: 0,
            skipped_active_leases: 0,
        };
        for spec in specs {
            let events = self.events(&spec.id)?;
            let projection = self.projection(&spec.id)?;
            let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
            let revoked: std::collections::BTreeSet<_> = events
                .iter()
                .filter_map(|event| match &event.envelope.payload {
                    EventPayload::LeaseRevoked { lease_id } => Some(lease_id.clone()),
                    _ => None,
                })
                .collect();
            let finished_runs: std::collections::BTreeSet<_> = events
                .iter()
                .filter_map(|event| match &event.envelope.payload {
                    EventPayload::RunFinished { run_id, .. } => Some(run_id.clone()),
                    _ => None,
                })
                .collect();
            for event in &events {
                let EventPayload::LeaseGranted { lease } = &event.envelope.payload else {
                    continue;
                };
                if revoked.contains(&lease.id) {
                    continue;
                }
                let scope = projection.scopes.get(&lease.scope_id);
                let scope_terminal = scope.is_some_and(|scope| {
                    matches!(
                        scope.state,
                        else_protocol::event::ScopeState::Completed
                            | else_protocol::event::ScopeState::Failed
                            | else_protocol::event::ScopeState::Cancelled
                            | else_protocol::event::ScopeState::Superseded
                    )
                });
                let generation_stale =
                    scope.is_some_and(|scope| scope.generation > lease.generation);
                let run_finished = lease
                    .run_id
                    .as_ref()
                    .is_some_and(|run_id| finished_runs.contains(run_id));
                if !scope_terminal && !generation_stale && !run_finished {
                    report.skipped_active_leases += 1;
                    continue;
                }
                report.orphan_leases += 1;
                if lease.class == ResourceClass::Sandbox
                    && !projection.sandbox_usages.contains_key(&lease.id)
                {
                    let started_at =
                        events
                            .iter()
                            .find_map(|candidate| match &candidate.envelope.payload {
                                EventPayload::SandboxStarted {
                                    lease_id,
                                    started_at,
                                    ..
                                } if lease_id == &lease.id => Some(*started_at),
                                _ => None,
                            });
                    let (started_at, basis) = started_at.map_or(
                        (
                            lease.granted_at.unwrap_or(event.envelope.occurred_at),
                            SandboxUsageBasis::Estimated,
                        ),
                        |started_at| (started_at, SandboxUsageBasis::Unknown),
                    );
                    let finished_at = Utc::now();
                    let usage = SandboxUsage {
                        lease_id: lease.id.clone(),
                        backend: sandbox_backend_name().into(),
                        started_at,
                        finished_at,
                        elapsed_millis: finished_at
                            .signed_duration_since(started_at)
                            .num_milliseconds()
                            .max(0) as u64,
                        basis,
                        termination: SandboxTerminationReason::Crashed,
                    };
                    self.ledger.append(&EventEnvelope::new(
                        spec.id.clone(),
                        lease.scope_id.clone(),
                        lease.generation,
                        revision,
                        Actor::System,
                        EventPayload::SandboxFinished { usage },
                    ))?;
                    report.sandbox_finish_records += 1;
                }
                self.ledger.append(&EventEnvelope::new(
                    spec.id.clone(),
                    lease.scope_id.clone(),
                    lease.generation,
                    revision,
                    Actor::System,
                    EventPayload::LeaseRevoked {
                        lease_id: lease.id.clone(),
                    },
                ))?;
                report.revoked_leases += 1;
            }
        }
        Ok(report)
    }
}

fn sandbox_backend_name() -> &'static str {
    #[cfg(target_os = "macos")]
    return "seatbelt";
    #[cfg(target_os = "linux")]
    return "bubblewrap";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return "unsupported";
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

#[cfg(test)]
mod tests {
    use super::*;
    use else_protocol::ids::RunId;

    #[test]
    fn orphan_lease_cleanup_skips_active_runs_and_revokes_finished_runs() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime.create_intent("lease cleanup", None).unwrap();
        let projection = runtime.projection(&spec.id).unwrap();
        let scope_id = projection.root_scope_id.unwrap();
        let revision = projection.spec.unwrap().revision;
        let active_run = RunId::new();
        let orphan_run = RunId::new();
        for (run_id, orphan) in [(active_run.clone(), false), (orphan_run.clone(), true)] {
            runtime
                .ledger
                .append(&EventEnvelope::new(
                    spec.id.clone(),
                    scope_id.clone(),
                    1,
                    revision,
                    Actor::System,
                    EventPayload::RunStarted {
                        run_id: run_id.clone(),
                        harness: "test".into(),
                        capabilities: None,
                    },
                ))
                .unwrap();
            let now = Utc::now();
            runtime
                .ledger
                .append(&EventEnvelope::new(
                    spec.id.clone(),
                    scope_id.clone(),
                    1,
                    revision,
                    Actor::System,
                    EventPayload::LeaseGranted {
                        lease: ResourceLease {
                            id: else_protocol::ids::LeaseId::new(),
                            scope_id: scope_id.clone(),
                            run_id: Some(run_id.clone()),
                            class: ResourceClass::Sandbox,
                            quantity: 900,
                            unit: "seconds".into(),
                            generation: 1,
                            granted_at: Some(now),
                            expires_at: now + chrono::Duration::minutes(15),
                        },
                    },
                ))
                .unwrap();
            if orphan {
                runtime
                    .ledger
                    .append(&EventEnvelope::new(
                        spec.id.clone(),
                        scope_id.clone(),
                        1,
                        revision,
                        Actor::System,
                        EventPayload::RunFinished {
                            run_id,
                            success: false,
                        },
                    ))
                    .unwrap();
            }
        }

        let report = runtime.cleanup_orphan_leases().unwrap();
        assert_eq!(report.orphan_leases, 1);
        assert_eq!(report.revoked_leases, 1);
        assert_eq!(report.sandbox_finish_records, 1);
        assert_eq!(report.skipped_active_leases, 1);
        assert_eq!(
            runtime
                .projection(&spec.id)
                .unwrap()
                .resource_usage
                .sandbox_reserved_seconds,
            900
        );
    }
}
