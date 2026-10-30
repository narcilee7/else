//! MVP-3 本地 Dispatch：把 Scope 映射到 Harness＋隔离 Workspace＋Snapshot＋Lease。
//!
//! 规则式选择（可解释、可复现），固定槽位执行（默认最多 2 个活跃 Run），
//! 工件经 ContextSnapshot 的 artifact_refs 交接；重试由调用方递增 Generation，
//! 根 Abort 沿委托树收敛全部本地执行（复用 M1 机制）。

use else_gateway::{ExecContext, GatewayError};
use else_harness::adapter::{HarnessAdapter, HarnessRequest};
use else_ledger::IntentProjection;
use else_protocol::event::{Actor, EventPayload, ScopeState};
use else_protocol::ids::{IntentId, RunId, ScopeId};
use else_protocol::memory::SnapshotAudience;

use crate::scope::scope_spec;
use crate::{ApiError, Result, Runtime};

/// 本地固定执行槽位：策略而非协议假设，未来可增加槽位与远程后端。
const DEFAULT_RUN_SLOTS: u64 = 2;

impl Runtime {
    /// 规则选择 + 派发（MVP-3 主入口）。`harness_hint` 为 None 或 "auto" 时
    /// 按任务类型与可用性规则选择；显式指定优先。scripted 走内置执行路径。
    pub fn dispatch_scope(
        &self,
        intent_id: &IntentId,
        scope_id: &ScopeId,
        harness_hint: Option<&str>,
    ) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        let spec = scope_spec(&projection, scope_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
        let name = else_harness::select_harness(&spec.outcome, harness_hint);
        if name == else_harness::SCRIPTED {
            return self.run_scope(intent_id, scope_id);
        }
        let adapter = else_harness::find_adapter(name)
            .ok_or_else(|| ApiError::NotFound(format!("harness {name}")))?;
        self.dispatch_scope_adapter(intent_id, scope_id, adapter.as_ref())
    }

    /// 用指定适配器执行一个 Scope：隔离 Workspace、Executor 快照、
    /// 槽位准入、Lease、能力落账；子 Scope 验收不误结束根 Intent。
    pub fn dispatch_scope_adapter(
        &self,
        intent_id: &IntentId,
        scope_id: &ScopeId,
        adapter: &dyn HarnessAdapter,
    ) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        let scope = projection
            .scopes
            .get(scope_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
        // 终态幂等：已完成的 Scope 不重复执行
        if matches!(
            scope.state,
            ScopeState::Completed | ScopeState::Failed | ScopeState::Cancelled
        ) {
            return Ok(projection);
        }
        let spec = scope_spec(&projection, scope_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {scope_id}")))?;
        let generation = scope.generation;

        // 准入先于一切事件：槽位或预算不足时不产生任何新事件。
        self.ensure_slot_available()?;

        // 共享不等于共同可写：每个 Scope 独立 Workspace，工件按引用交接。
        let scope_workspace = self.workspace.join("scopes").join(scope_id.to_string());
        std::fs::create_dir_all(&scope_workspace)?;

        let snapshot = self.compile_snapshot_for(
            intent_id,
            &spec,
            scope_id,
            generation,
            SnapshotAudience::Executor,
        )?;
        let ctx = ExecContext {
            spec,
            scope_id: scope_id.clone(),
            run_id: RunId::new(),
            generation,
            snapshot_id: Some(snapshot.id.clone()),
        };
        let lease = self.admit_run(&ctx)?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunStarted {
                run_id: ctx.run_id.clone(),
                harness: adapter.name().into(),
                capabilities: Some(adapter.capabilities()),
            },
        )?;
        let request = HarnessRequest {
            ctx: &ctx,
            ledger: &self.ledger,
            workspace: &scope_workspace,
            snapshot: &snapshot,
            subagent: true,
        };
        self.finish_run(intent_id, &ctx, &lease, adapter.execute(&request))
    }

    /// 固定槽位准入：活跃 Run（RunStarted 无对应 RunFinished）到顶则拒绝启动。
    /// 槽位是本地策略；额度不足时上层选择等待、降级或请求本人决策。
    fn ensure_slot_available(&self) -> Result<()> {
        let limit = std::env::var("ELSE_MAX_RUN_SLOTS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(DEFAULT_RUN_SLOTS);
        let mut started: std::collections::BTreeSet<String> = Default::default();
        let mut finished: std::collections::BTreeSet<String> = Default::default();
        for event in self.ledger.read_all()? {
            match &event.envelope.payload {
                EventPayload::RunStarted { run_id, .. } => {
                    started.insert(run_id.to_string());
                }
                EventPayload::RunFinished { run_id, .. } => {
                    finished.insert(run_id.to_string());
                }
                _ => {}
            }
        }
        let active = started.difference(&finished).count() as u64;
        if active >= limit {
            return Err(GatewayError::ResourceExhausted {
                resource: "run_slots".into(),
                requested: 1,
                remaining: 0,
                unit: "slots",
            }
            .into());
        }
        Ok(())
    }
}
