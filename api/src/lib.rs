//! else Runtime API：所有表面（CLI/Web/桌面/App/IM）共用的薄接口。
//!
//! 表面只能读投影、提交 Intent 与 Decision，不能绕过 Gateway 触碰执行层。
//! W2 以库接口形式提供（进程内调用）；daemon 与远程 IPC 复用同一接口。

use std::path::{Path, PathBuf};

use else_gateway::{
    ExecContext, GatewayError, MockProvider, ModelGateway, OpenAiCompatProvider, ToolGateway,
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

    /// 提交一个抽象 Intent（Intent Resolution 的最小实现，W3 补全假设与验收）。
    pub fn create_intent(&self, outcome: &str) -> Result<IntentSpec> {
        let spec = IntentSpec {
            id: IntentId::new(),
            revision: 1,
            outcome: outcome.to_string(),
            done_when: vec![],
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
            assumptions: vec![],
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
    pub fn run(&self, intent_id: &IntentId) -> Result<IntentProjection> {
        let spec = self
            .projection(intent_id)?
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
        let model = ModelGateway::new(&self.ledger, self.provider());
        let result = else_harness::run(&ctx, &self.ledger, &tools, &model);

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

    /// 本人决策：落 DecisionResolved 事件，Intent 回到 Active。
    /// W3 在此接续自动重规划与续跑。
    pub fn decide(
        &self,
        intent_id: &IntentId,
        request_id: &AttentionId,
        choice: DecisionChoice,
    ) -> Result<()> {
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
        Ok(())
    }

    pub fn events(&self, intent_id: &IntentId) -> Result<Vec<StoredEvent>> {
        Ok(self.ledger.read_intent(intent_id)?)
    }

    /// Provider 选择：配置了 OpenAI 兼容端点则直连，否则确定性 Mock。
    fn provider(&self) -> Box<dyn else_gateway::ModelProvider> {
        match (
            std::env::var("ELSE_MODEL_BASE_URL"),
            std::env::var("ELSE_MODEL_API_KEY"),
        ) {
            (Ok(base_url), Ok(api_key)) => Box::new(OpenAiCompatProvider { base_url, api_key }),
            _ => Box::new(MockProvider),
        }
    }
}
