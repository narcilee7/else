//! else Runtime API：所有表面（CLI/Web/桌面/App/IM）共用的薄接口。
//!
//! 表面只能读投影、提交 Intent 与 Decision，不能绕过 Gateway 触碰执行层。
//! W2 以库接口形式提供（进程内调用）；daemon 与远程 IPC 复用同一接口。

mod abort;
mod action;
mod attention;
mod binding;
mod capability;
mod connector;
mod context;
mod data_export;
mod data_lifecycle;
mod dependency;
mod diagnostics;
mod dispatch;
mod failure;
mod kernel;
mod opaque;
mod outcome;
mod plan;
mod resources;
mod runtime_settings;
mod scope;
mod service;
mod tool_gateway;
mod wake;
mod world;

pub use connector::ConnectorRepairReport;
pub use context::IntentRevision;
pub use data_export::RuntimeDataExport;
pub use data_lifecycle::IntentDeletionReport;
pub use diagnostics::RuntimeDiagnosticReport;
pub use kernel::{KernelAction, KernelDrive, KernelDriveStop};
pub use resources::LeaseCleanupReport;
pub use runtime_settings::RuntimeSettings;
pub use service::{RuntimeHandle, RuntimeService};

use std::path::{Path, PathBuf};

use else_gateway::{
    ExecContext, GatewayError, GenAiProvider, MockProvider, ModelGateway, ToolGateway,
};
use else_ledger::{IntentProjection, Ledger, Projection, StoredEvent};
use else_protocol::attention::{AttentionKind, Decision, DecisionChoice};
use else_protocol::connector::{IntentAcceptedBy, IntentOrigin, IntentOriginKind};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{AttentionId, IntentId, RunId, ScopeId};
use else_protocol::intent::{AttentionPolicy, Authority, IntentSpec, IntentState};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("ledger: {0}")]
    Ledger(#[from] else_ledger::LedgerError),
    #[error("gateway: {0}")]
    Gateway(#[from] GatewayError),
    #[error("intent not found: {0}")]
    IntentNotFound(IntentId),
    #[error("workspace: {0}")]
    Workspace(#[from] else_workspace::WorkspaceError),
    #[error("credential: {0}")]
    Credential(#[from] else_secrets::SecretError),
    #[error("connector: {0}")]
    Connector(#[from] else_connector::ConnectorError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("runtime worker stopped")]
    WorkerStopped,
    #[error("subagent authority exceeds parent: {0}")]
    PermissionEscalation(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ApiError>;

pub struct Runtime {
    ledger: Ledger,
    workspace: PathBuf,
    harness_private: PathBuf,
    store: else_workspace::Store,
    branches: PathBuf,
    settings_path: PathBuf,
    default_budget: else_protocol::resource::ResourceBudget,
    connector_adapters: else_connector::AdapterRegistry,
    signal_interpreter: else_connector::SignalInterpreter,
    credential_store: else_secrets::CredentialStore,
}

pub(crate) struct IntentDraft {
    pub outcome: String,
    pub project: Option<String>,
    pub done_when: Vec<else_protocol::intent::Criterion>,
    pub constraints: Vec<String>,
    pub authority: Authority,
    pub attention_policy: AttentionPolicy,
    pub deadline: Option<chrono::DateTime<chrono::Utc>>,
    pub origin: Option<IntentOrigin>,
}

impl Runtime {
    /// 打开本地 Runtime：`dir/else.db`（事件库）+ `dir/workspace/`（工作区）
    /// + `dir/store/`（CAS 工件存储）+ `dir/branches/`（分支工作区）。
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with_connector_runtime(
            dir,
            else_connector::AdapterRegistry::builtin(),
            else_connector::SignalInterpreter::builtin(),
        )
    }

    /// 使用外部注入的 Adapter Registry / Interpreter 打开 Runtime。
    /// 新 Provider 可通过组合根注入，无需修改 Intent Kernel。
    pub fn open_with_connector_runtime(
        dir: &Path,
        connector_adapters: else_connector::AdapterRegistry,
        signal_interpreter: else_connector::SignalInterpreter,
    ) -> Result<Self> {
        let credential_store = else_secrets::CredentialStore::open(dir)?;
        Self::open_with_connector_runtime_and_store(
            dir,
            connector_adapters,
            signal_interpreter,
            credential_store,
        )
    }

    /// 测试与部署组合根可显式选择凭据后端，避免依赖进程级环境变量。
    pub fn open_with_connector_runtime_and_store(
        dir: &Path,
        connector_adapters: else_connector::AdapterRegistry,
        signal_interpreter: else_connector::SignalInterpreter,
        credential_store: else_secrets::CredentialStore,
    ) -> Result<Self> {
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace)?;
        let harness_private = dir.join("harness-private");
        std::fs::create_dir_all(&harness_private)?;
        let runtime = Self {
            ledger: Ledger::open(dir.join("else.db"))?,
            workspace,
            harness_private,
            store: else_workspace::Store::new(dir.join("store"))?,
            branches: dir.join("branches"),
            settings_path: dir.join("runtime-settings.json"),
            default_budget: resources::budget_from_env(),
            connector_adapters,
            signal_interpreter,
            credential_store,
        };
        // 上次进程可能死在 Abort 的任意事件边界；重开后先完成安全收敛。
        runtime.recover_aborts()?;
        Ok(runtime)
    }

    /// 提交一个抽象 Intent（Intent Resolution 最小实现）：
    /// 系统补全默认验收标准与显式假设，补全内容可修正，不是隐蔽事实。
    /// `project` 决定 Episodic/Semantic 记忆的项目召回作用域。
    pub fn create_intent(&self, outcome: &str, project: Option<String>) -> Result<IntentSpec> {
        self.create_intent_with_done_when(
            outcome,
            project,
            vec![else_protocol::intent::Criterion::file_exists(
                "计划产物写入工作区且校验通过",
                "plan.md",
            )],
        )
    }

    /// 使用显式 DoneWhen 创建 Intent。机器检查与人工确认标准可混合。
    pub fn create_intent_with_done_when(
        &self,
        outcome: &str,
        project: Option<String>,
        done_when: Vec<else_protocol::intent::Criterion>,
    ) -> Result<IntentSpec> {
        self.create_intent_from_parts(IntentDraft {
            outcome: outcome.into(),
            project,
            done_when,
            constraints: vec![],
            authority: Authority {
                max_autonomous_effect: SideEffectKind::Reversible,
                forbidden: vec![],
            },
            attention_policy: AttentionPolicy {
                allow_interrupt: true,
                batch_interval_minutes: 30,
            },
            deadline: None,
            origin: Some(IntentOrigin {
                kind: IntentOriginKind::UserDirect,
                connector_ids: vec![],
                signal_ids: vec![],
                candidate_id: None,
                accepted_by: IntentAcceptedBy::User,
                accepted_at: chrono::Utc::now(),
            }),
        })
    }

    pub(crate) fn create_intent_from_parts(&self, draft: IntentDraft) -> Result<IntentSpec> {
        let root_scope_id = ScopeId::new();
        let spec = IntentSpec {
            id: IntentId::new(),
            revision: 1,
            outcome: draft.outcome,
            done_when: draft.done_when,
            constraints: draft.constraints,
            authority: draft.authority,
            attention_policy: draft.attention_policy,
            opaque_harness_policy: else_protocol::intent::OpaqueHarnessPolicy::default(),
            deadline: draft.deadline,
            assumptions: vec![else_protocol::intent::Assumption {
                text: "假设：在当前工作区推进该目标（系统补全，可修正）".into(),
                confirmed: false,
            }],
            project: draft.project,
            origin: draft.origin,
            connector_policy: else_protocol::connector::ConnectorPolicy::default(),
        };
        self.ledger.append(&EventEnvelope::new(
            spec.id.clone(),
            root_scope_id.clone(),
            1,
            1,
            Actor::User,
            EventPayload::IntentCreated { spec: spec.clone() },
        ))?;
        self.ledger.append(&EventEnvelope::new(
            spec.id.clone(),
            root_scope_id,
            1,
            spec.revision,
            Actor::System,
            EventPayload::ResourceBudgetSet {
                budget: self.default_budget,
            },
        ))?;
        Ok(spec)
    }

    /// 启动一次执行（一个 Harness、一次 Run）。返回最新投影。
    /// 终态幂等：Completed/Failed/Cancelled 不重复执行。
    pub fn run(&self, intent_id: &IntentId) -> Result<IntentProjection> {
        let Some((ctx, _snapshot)) = self.prepare_root_run(intent_id, false)? else {
            return self.projection(intent_id);
        };
        let lease = self.admit_run_with_failure(&ctx)?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunStarted {
                run_id: ctx.run_id.clone(),
                harness: else_harness::SCRIPTED.into(),
                capabilities: None,
            },
        )?;
        let sandbox_started = self.start_sandbox(&ctx, &lease)?;

        let tools = ToolGateway::new(&self.ledger, self.workspace.clone());
        let model = ModelGateway::new(&self.ledger, self.provider()?);
        let result = else_harness::run(
            &ctx,
            &self.ledger,
            &tools,
            &model,
            &self.model_name(),
            else_harness::RunOptions::default(),
        );
        self.finish_run(intent_id, &ctx, &lease, sandbox_started, result)
    }

    /// 用真实 Harness 适配器执行（MVP-2）：能力随 RunStarted 落账，
    /// Harness 输出桥接为 else 事件；Harness 会话不是权威状态。
    pub fn run_adapter(
        &self,
        intent_id: &IntentId,
        adapter: &dyn else_harness::adapter::HarnessAdapter,
    ) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        let spec = projection
            .spec
            .as_ref()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        self.ensure_harness_allowed(spec, adapter)?;
        let egress_proxy = self.start_egress_proxy(adapter)?;
        let Some((ctx, snapshot)) = self.prepare_root_run(intent_id, true)? else {
            return self.projection(intent_id);
        };
        let tool_gateway = self.start_tool_gateway(&ctx.spec, &ctx, &self.workspace, None)?;
        let tool_gateway_endpoint = tool_gateway.as_ref().map(|gateway| {
            let endpoint = gateway.endpoint();
            else_harness::adapter::ToolGatewayEndpoint {
                host: endpoint.host.clone(),
                port: endpoint.port,
                token: endpoint.token.clone(),
            }
        });
        let lease = self.admit_run_with_failure(&ctx)?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunStarted {
                run_id: ctx.run_id.clone(),
                harness: adapter.name().into(),
                capabilities: Some(adapter.capabilities()),
            },
        )?;
        let sandbox_started = self.start_sandbox(&ctx, &lease)?;
        let request = else_harness::adapter::HarnessRequest {
            ctx: &ctx,
            ledger: &self.ledger,
            workspace: &self.workspace,
            private_dir: &self.harness_private,
            snapshot: &snapshot,
            egress_proxy: egress_proxy.as_ref().map(|proxy| proxy.endpoint()),
            tool_gateway: tool_gateway_endpoint.as_ref(),
            subagent: false,
        };
        let result = adapter.execute(&request);
        self.record_egress_decisions(&ctx, egress_proxy.as_ref())?;
        self.finish_run(intent_id, &ctx, &lease, sandbox_started, result)
    }

    /// Run 前奏：终态幂等检查、编译 Planner 快照、构造执行上下文。
    /// 返回 None 表示 Intent 已在终态，本次 Run 不产生任何事件。
    fn prepare_root_run(
        &self,
        intent_id: &IntentId,
        ensure_plan: bool,
    ) -> Result<Option<(ExecContext, else_protocol::memory::ContextSnapshot)>> {
        let projection = self.projection(intent_id)?;
        if matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ) {
            return Ok(None);
        }
        let spec = projection
            .spec
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let scope_id = projection
            .root_scope_id
            .clone()
            .or_else(|| projection.scopes.keys().next().cloned())
            .unwrap_or_else(ScopeId::new);
        self.ensure_slot_available()?;
        self.ensure_scope_not_active(intent_id, &scope_id)?;
        let generation = self.next_scope_generation(intent_id, &scope_id)?;
        if ensure_plan {
            self.ensure_plan_for_scope(intent_id, &spec, &scope_id)?;
        }
        // 执行前编译 Planner 快照；本 Run 后续事件都标记该上下文版本。
        let snapshot =
            self.compile_snapshot(intent_id, else_protocol::memory::SnapshotAudience::Planner)?;
        Ok(Some((
            ExecContext {
                spec,
                scope_id,
                run_id: RunId::new(),
                generation,
                snapshot_id: Some(snapshot.id.clone()),
            },
            snapshot,
        )))
    }

    /// Run 收尾：RunFinished、回收 Lease、终态写 Episodic 记忆。
    fn finish_run(
        &self,
        intent_id: &IntentId,
        ctx: &ExecContext,
        lease: &else_protocol::resource::ResourceLease,
        sandbox_started: chrono::DateTime<chrono::Utc>,
        result: else_gateway::Result<()>,
    ) -> Result<IntentProjection> {
        let (success, error, termination, basis) = match result {
            Ok(()) => (
                true,
                None,
                else_protocol::resource::SandboxTerminationReason::Completed,
                else_protocol::resource::SandboxUsageBasis::Reported,
            ),
            // 等待本人确认或授权是正常等待态，不是执行失败。
            Err(GatewayError::NeedsApproval(_)) => (
                true,
                None,
                else_protocol::resource::SandboxTerminationReason::PausedForDecision,
                else_protocol::resource::SandboxUsageBasis::Reported,
            ),
            // 故障注入代表进程已死：不得再写 RunFinished，直接丢弃 Runtime 重开。
            Err(e @ GatewayError::Ledger(else_ledger::LedgerError::CrashInjected)) => {
                return Err(e.into());
            }
            Err(e @ GatewayError::Timeout(_)) => (
                false,
                Some(e),
                else_protocol::resource::SandboxTerminationReason::TimedOut,
                else_protocol::resource::SandboxUsageBasis::Reported,
            ),
            Err(e) => (
                false,
                Some(e),
                else_protocol::resource::SandboxTerminationReason::Failed,
                else_protocol::resource::SandboxUsageBasis::Reported,
            ),
        };
        self.finish_sandbox(ctx, lease, sandbox_started, termination, basis)?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunFinished {
                run_id: ctx.run_id.clone(),
                success,
            },
        )?;
        if let Some(error) = &error {
            self.record_scope_failure(ctx, error)?;
        }
        self.revoke_lease(ctx, lease)?;
        if let Some(e) = error {
            return Err(e.into());
        }
        self.record_episode_if_terminal(intent_id)?;
        self.projection(intent_id)
    }

    /// 读投影（局部重放）。
    pub fn projection(&self, intent_id: &IntentId) -> Result<IntentProjection> {
        let mut projection = IntentProjection::default();
        for event in self.ledger.read_intent(intent_id)? {
            projection.apply(&event);
        }
        Ok(projection)
    }

    /// 列出全部 Intent（以最新 revision 为准）。
    pub fn list_intents(&self) -> Result<Vec<IntentSpec>> {
        let mut specs = std::collections::BTreeMap::new();
        for event in self.ledger.read_all()? {
            match event.envelope.payload {
                EventPayload::IntentCreated { spec } | EventPayload::IntentRevised { spec, .. } => {
                    specs.insert(spec.id.clone(), spec);
                }
                _ => {}
            }
        }
        Ok(specs.into_values().collect())
    }

    /// 本人决策：落 DecisionResolved 事件，Intent 回到 Active，
    /// 随后自动续跑——批准则原路径继续，否决则自动重规划。
    /// 返回续跑后的最新投影。
    pub fn decide(
        &self,
        intent_id: &IntentId,
        request_id: &AttentionId,
        choice: DecisionChoice,
    ) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        if !projection.open_requests.contains_key(request_id) {
            return Err(ApiError::IntentNotFound(intent_id.clone()));
        }
        let request_event = self
            .ledger
            .read_intent(intent_id)?
            .into_iter()
            .rev()
            .find(|event| {
                matches!(
                    &event.envelope.payload,
                    EventPayload::AttentionRequested { request } if &request.id == request_id
                )
            })
            .ok_or_else(|| ApiError::NotFound(format!("attention {request_id}")))?;
        let scope_id = request_event.envelope.scope_id;
        let generation = request_event.envelope.generation;
        let run_id = request_event.envelope.run_id.clone().unwrap_or_default();
        let request_kind = match &request_event.envelope.payload {
            EventPayload::AttentionRequested { request } => request.kind,
            _ => unreachable!("request event selected above"),
        };
        let approved = matches!(
            &choice,
            DecisionChoice::Option(id) if id == "approve"
        );
        let revision = projection.spec.map(|s| s.revision).unwrap_or(1);
        let envelope = EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            generation,
            revision,
            Actor::User,
            EventPayload::DecisionResolved {
                decision: Decision {
                    request_id: request_id.clone(),
                    choice,
                    rationale: None,
                    decided_by: Actor::User,
                },
            },
        );
        self.ledger.append(&envelope)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            generation,
            revision,
            Actor::System,
            EventPayload::IntentStateChanged {
                state: IntentState::Active,
            },
        ))?;
        self.close_empty_attention_windows(intent_id)?;
        if request_kind == AttentionKind::Verification {
            return if approved {
                self.confirm_manual_verification(intent_id, &scope_id, generation, run_id)
            } else {
                self.projection(intent_id)
            };
        }
        // 在发起请求的同一 Scope 续跑：一次性授权不会跨委托泄漏。
        self.run_scope(intent_id, &scope_id)
    }

    /// 创建检查点：事件水位 + 工作区快照 + 在途副作用结论，三者缺一不可。
    pub fn checkpoint(&self, intent_id: &IntentId) -> Result<else_protocol::artifact::Checkpoint> {
        let projection = self.projection(intent_id)?;
        let spec = projection
            .spec
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let artifact = self.store.snapshot(&self.workspace, &RunId::new())?;
        let cursor = self.ledger.cursor()?;
        let ctx = ExecContext {
            spec: spec.clone(),
            scope_id: ScopeId::new(),
            run_id: RunId::new(),
            generation: 1,
            snapshot_id: None,
        };
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::ArtifactProduced {
                artifact: artifact.clone(),
            },
        )?;
        let checkpoint = else_protocol::artifact::Checkpoint {
            id: else_protocol::ids::CheckpointId::new(),
            scope_id: ctx.scope_id.clone(),
            generation: 1,
            intent_revision: spec.revision,
            event_cursor: cursor,
            artifact_root: Some(artifact.id.clone()),
            inflight: self.inflight(intent_id)?,
            created_at: chrono::Utc::now(),
        };
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::CheckpointCreated {
                checkpoint: checkpoint.clone(),
            },
        )?;
        Ok(checkpoint)
    }

    /// 在途副作用：已调用但尚未完成的工具调用。
    fn inflight(
        &self,
        intent_id: &IntentId,
    ) -> Result<Vec<else_protocol::artifact::InFlightEffect>> {
        let events = self.ledger.read_intent(intent_id)?;
        let mut called = std::collections::BTreeMap::new();
        let mut done = std::collections::BTreeSet::new();
        for event in events {
            match event.envelope.payload {
                EventPayload::ToolCalled { call } => {
                    called.insert(call.id.clone(), call);
                }
                EventPayload::ToolCompleted { call_id, .. } => {
                    done.insert(call_id);
                }
                _ => {}
            }
        }
        Ok(called
            .into_values()
            .filter(|call| !done.contains(&call.id))
            .map(|call| else_protocol::artifact::InFlightEffect {
                call_id: call.id,
                conclusion: else_protocol::effect::EffectConclusion::Unknown,
                note: format!("{} 尚未有结论", call.tool),
            })
            .collect())
    }

    /// 从检查点派生分支并重跑（Fork → Rerun）：
    /// 恢复快照到独立分支工作区，Generation 递增与主线隔离；
    /// `alternative` 强制走不依赖 shell 的替代路径。返回 (分支 Scope, Generation)。
    pub fn rerun(
        &self,
        intent_id: &IntentId,
        checkpoint_id: &else_protocol::ids::CheckpointId,
        alternative: bool,
    ) -> Result<(ScopeId, u64)> {
        let events = self.ledger.read_intent(intent_id)?;
        let checkpoint = events
            .iter()
            .rev()
            .find_map(|e| match &e.envelope.payload {
                EventPayload::CheckpointCreated { checkpoint }
                    if &checkpoint.id == checkpoint_id =>
                {
                    Some(checkpoint.clone())
                }
                _ => None,
            })
            .ok_or_else(|| ApiError::NotFound(format!("checkpoint {checkpoint_id}")))?;
        let artifact_root = checkpoint
            .artifact_root
            .clone()
            .ok_or_else(|| ApiError::NotFound("checkpoint artifact root".into()))?;
        let manifest = events
            .iter()
            .rev()
            .find_map(|e| match &e.envelope.payload {
                EventPayload::ArtifactProduced { artifact } if artifact.id == artifact_root => {
                    artifact.manifest.clone()
                }
                _ => None,
            })
            .ok_or_else(|| ApiError::NotFound("snapshot manifest".into()))?;
        let spec = events.iter().rev().find_map(|e| match &e.envelope.payload {
            EventPayload::IntentCreated { spec } | EventPayload::IntentRevised { spec, .. } => {
                Some(spec.clone())
            }
            _ => None,
        });
        let spec = spec.ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let generation = events
            .iter()
            .map(|e| e.envelope.generation)
            .max()
            .unwrap_or(1)
            + 1;
        let scope_id = ScopeId::new();
        let branch_ws = self.branches.join(scope_id.to_string());
        self.store.restore(&manifest, &branch_ws)?;

        let ctx = ExecContext {
            spec,
            scope_id: scope_id.clone(),
            run_id: RunId::new(),
            generation,
            snapshot_id: None,
        };
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::ScopeSpawned {
                child: scope_id.clone(),
            },
        )?;
        let lease = self.admit_run_with_failure(&ctx)?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunStarted {
                run_id: ctx.run_id.clone(),
                harness: else_harness::SCRIPTED.into(),
                capabilities: None,
            },
        )?;
        let sandbox_started = self.start_sandbox(&ctx, &lease)?;
        let tools = ToolGateway::new(&self.ledger, branch_ws);
        let model = ModelGateway::new(&self.ledger, self.provider()?);
        let result = else_harness::run(
            &ctx,
            &self.ledger,
            &tools,
            &model,
            &self.model_name(),
            else_harness::RunOptions {
                cursor: Some(checkpoint.event_cursor),
                force_replan: true,
                alternative,
                subagent: false,
            },
        );
        let (success, error, termination) = match result {
            Ok(()) => (
                true,
                None,
                else_protocol::resource::SandboxTerminationReason::Completed,
            ),
            Err(GatewayError::NeedsApproval(_)) => (
                true,
                None,
                else_protocol::resource::SandboxTerminationReason::PausedForDecision,
            ),
            Err(e @ GatewayError::Ledger(else_ledger::LedgerError::CrashInjected)) => {
                return Err(e.into());
            }
            Err(e @ GatewayError::Timeout(_)) => (
                false,
                Some(e),
                else_protocol::resource::SandboxTerminationReason::TimedOut,
            ),
            Err(e) => (
                false,
                Some(e),
                else_protocol::resource::SandboxTerminationReason::Failed,
            ),
        };
        self.finish_sandbox(
            &ctx,
            &lease,
            sandbox_started,
            termination,
            else_protocol::resource::SandboxUsageBasis::Reported,
        )?;
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunFinished {
                run_id: ctx.run_id.clone(),
                success,
            },
        )?;
        self.revoke_lease(&ctx, &lease)?;
        if let Some(e) = error {
            return Err(e.into());
        }
        Ok((scope_id, generation))
    }

    /// 三维度 Compare（按 Generation）：Outcome Evidence、模型与副作用账、工件差异。
    pub fn compare(&self, intent_id: &IntentId) -> Result<String> {
        let events = self.ledger.read_intent(intent_id)?;
        let mut tool_names = std::collections::HashMap::new();
        for event in &events {
            match &event.envelope.payload {
                EventPayload::ToolCalled { call }
                | EventPayload::OpaqueToolObserved { call, .. } => {
                    tool_names.insert(call.id.clone(), call.tool.clone());
                }
                _ => {}
            }
        }
        let mut by_generation: std::collections::BTreeMap<u64, Vec<&StoredEvent>> =
            Default::default();
        for event in &events {
            by_generation
                .entry(event.envelope.generation)
                .or_default()
                .push(event);
        }
        let mut out = String::new();
        let mut plan_hashes = Vec::new();
        for (generation, events) in by_generation {
            let mut input = 0;
            let mut output = 0;
            let mut cost = 0;
            let mut verified = false;
            let mut tools = Vec::new();
            let mut sandbox_reserved_seconds = 0_u64;
            let mut sandbox_reported_seconds = 0_u64;
            let mut sandbox_estimated_seconds = 0_u64;
            let mut sandbox_unknown_seconds = 0_u64;
            let mut sandbox_actual_millis = 0_u64;
            let mut plan_path: Option<(ScopeId, String)> = None;
            for event in events {
                match &event.envelope.payload {
                    EventPayload::ModelCompleted { usage, .. } => {
                        input += usage.input_tokens;
                        output += usage.output_tokens;
                        cost += usage.cost_usd_micros;
                    }
                    EventPayload::OutcomeVerified { .. } => verified = true,
                    EventPayload::ToolCompleted { call_id, outcome } => {
                        let name = tool_names
                            .get(call_id)
                            .cloned()
                            .unwrap_or_else(|| "?".into());
                        tools.push(format!("{name}({:?})", outcome.effect));
                    }
                    EventPayload::OpaqueToolObserved { call, outcome, .. } => {
                        tools.push(format!("observed:{} ({:?})", call.tool, outcome.effect));
                    }
                    EventPayload::LeaseGranted { lease }
                        if lease.class == else_protocol::resource::ResourceClass::Sandbox =>
                    {
                        sandbox_reserved_seconds =
                            sandbox_reserved_seconds.saturating_add(lease.quantity);
                    }
                    EventPayload::SandboxFinished { usage } => {
                        let seconds = usage
                            .elapsed_millis
                            .saturating_add(999)
                            .saturating_div(1000)
                            .max(1);
                        sandbox_actual_millis =
                            sandbox_actual_millis.saturating_add(usage.elapsed_millis);
                        match usage.basis {
                            else_protocol::resource::SandboxUsageBasis::Reported => {
                                sandbox_reported_seconds =
                                    sandbox_reported_seconds.saturating_add(seconds);
                            }
                            else_protocol::resource::SandboxUsageBasis::Estimated => {
                                sandbox_estimated_seconds =
                                    sandbox_estimated_seconds.saturating_add(seconds);
                            }
                            else_protocol::resource::SandboxUsageBasis::Unknown => {
                                sandbox_unknown_seconds =
                                    sandbox_unknown_seconds.saturating_add(seconds);
                            }
                        }
                    }
                    EventPayload::ArtifactProduced { artifact }
                        if artifact.locator.ends_with("plan.md") =>
                    {
                        plan_path =
                            Some((event.envelope.scope_id.clone(), artifact.locator.clone()));
                    }
                    _ => {}
                }
            }
            out.push_str(&format!(
                "generation {generation}:\n  outcome_verified: {verified}\n  model: input={input} output={output} cost_usd_micros={cost}\n  sandbox: reserved_seconds={sandbox_reserved_seconds} actual_millis={sandbox_actual_millis} reported={sandbox_reported_seconds}s estimated={sandbox_estimated_seconds}s unknown={sandbox_unknown_seconds}s\n  tools: {}\n",
                tools.join(", ")
            ));
            if let Some((scope_id, path)) = plan_path {
                let relative = std::path::Path::new(&path);
                let path = if relative.is_absolute() {
                    std::path::PathBuf::from(relative)
                } else {
                    let branch = self.branches.join(scope_id.to_string()).join(relative);
                    let scoped = self
                        .workspace
                        .join("scopes")
                        .join(scope_id.to_string())
                        .join(relative);
                    if branch.exists() {
                        branch
                    } else if scoped.exists() {
                        scoped
                    } else {
                        self.workspace.join(relative)
                    }
                };
                match else_workspace::Store::hash_file(&path) {
                    Ok(hash) => {
                        plan_hashes.push((generation, hash.clone()));
                        out.push_str(&format!("  plan.md hash: {}\n", &hash[..16]));
                    }
                    Err(_) => out.push_str("  plan.md: <missing>\n"),
                }
            }
        }
        if plan_hashes.len() >= 2 {
            let same = plan_hashes.windows(2).all(|w| w[0].1 == w[1].1);
            out.push_str(if same {
                "compare: 各分支 plan.md 内容一致\n"
            } else {
                "compare: 各分支 plan.md 内容不同（产物可并排查看）\n"
            });
        }
        Ok(out)
    }

    pub fn events(&self, intent_id: &IntentId) -> Result<Vec<StoredEvent>> {
        Ok(self.ledger.read_intent(intent_id)?)
    }

    /// 默认模型：环境变量 ELSE_MODEL（genai 模型名，如 `gpt-4o-mini`、
    /// `claude-sonnet-4-5`、`ollama::llama3.2`、`moonshot::moonshot-v1-8k`）。
    /// 缺省 `mock-model` 走确定性 Mock，不产生真实外部调用。
    fn model_name(&self) -> String {
        std::env::var("ELSE_MODEL").unwrap_or_else(|_| "mock-model".into())
    }

    /// Provider 选择：数据面复用 genai，密钥走各 Provider 标准环境变量。
    fn provider(&self) -> Result<Box<dyn else_gateway::ModelProvider>> {
        if self.model_name() == "mock-model" {
            return Ok(Box::new(MockProvider));
        }
        let provider = GenAiProvider::new().map_err(GatewayError::Model)?;
        Ok(Box::new(provider))
    }
}
