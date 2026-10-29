//! else Runtime API：所有表面（CLI/Web/桌面/App/IM）共用的薄接口。
//!
//! 表面只能读投影、提交 Intent 与 Decision，不能绕过 Gateway 触碰执行层。
//! W2 以库接口形式提供（进程内调用）；daemon 与远程 IPC 复用同一接口。

use std::path::{Path, PathBuf};

use else_gateway::{
    ExecContext, GatewayError, GenAiProvider, MockProvider, ModelGateway, ToolGateway,
};
use else_ledger::{IntentProjection, Ledger, Projection, StoredEvent};
use else_protocol::attention::{Decision, DecisionChoice};
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
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ApiError>;

pub struct Runtime {
    ledger: Ledger,
    workspace: PathBuf,
}

impl Runtime {
    /// 打开本地 Runtime：`dir/else.db`（事件库）+ `dir/workspace/`（工作区）。
    pub fn open(dir: &Path) -> Result<Self> {
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace)?;
        Ok(Self {
            ledger: Ledger::open(dir.join("else.db"))?,
            workspace,
        })
    }

    /// 提交一个抽象 Intent（Intent Resolution 最小实现）：
    /// 系统补全默认验收标准与显式假设，补全内容可修正，不是隐蔽事实。
    pub fn create_intent(&self, outcome: &str) -> Result<IntentSpec> {
        let spec = IntentSpec {
            id: IntentId::new(),
            revision: 1,
            outcome: outcome.to_string(),
            done_when: vec![else_protocol::intent::Criterion {
                description: "计划产物写入工作区且校验通过".into(),
            }],
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
            assumptions: vec![else_protocol::intent::Assumption {
                text: "假设：在当前工作区推进该目标（系统补全，可修正）".into(),
                confirmed: false,
            }],
        };
        self.ledger.append(&EventEnvelope::new(
            spec.id.clone(),
            ScopeId::new(),
            1,
            1,
            Actor::User,
            EventPayload::IntentCreated { spec: spec.clone() },
        ))?;
        Ok(spec)
    }

    /// 启动一次执行（一个 Harness、一次 Run）。返回最新投影。
    /// 终态幂等：Completed/Failed/Cancelled 不重复执行。
    pub fn run(&self, intent_id: &IntentId) -> Result<IntentProjection> {
        let projection = self.projection(intent_id)?;
        if matches!(
            projection.state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ) {
            return Ok(projection);
        }
        let spec = projection
            .spec
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let ctx = ExecContext {
            spec,
            scope_id: ScopeId::new(),
            run_id: RunId::new(),
            generation: 1,
        };
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunStarted {
                run_id: ctx.run_id.clone(),
                harness: else_harness::SCRIPTED.into(),
            },
        )?;

        let tools = ToolGateway::new(&self.ledger, self.workspace.clone());
        let model = ModelGateway::new(&self.ledger, self.provider()?);
        let result = else_harness::run(&ctx, &self.ledger, &tools, &model, &self.model_name());

        let (success, error) = match result {
            Ok(()) => (true, None),
            // 等待本人决策是正常终态，不是执行失败
            Err(GatewayError::NeedsApproval(_)) => (true, None),
            Err(e) => (false, Some(e)),
        };
        ctx.record(
            &self.ledger,
            Actor::System,
            EventPayload::RunFinished {
                run_id: ctx.run_id.clone(),
                success,
            },
        )?;
        if let Some(e) = error {
            return Err(e.into());
        }
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
        let envelope = EventEnvelope::new(
            intent_id.clone(),
            ScopeId::new(),
            1,
            projection.spec.map(|s| s.revision).unwrap_or(1),
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
            ScopeId::new(),
            1,
            1,
            Actor::System,
            EventPayload::IntentStateChanged {
                state: IntentState::Active,
            },
        ))?;
        // 自动续跑：从事件恢复进度；approve 原路径继续，reject 自动重规划
        self.run(intent_id)
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
