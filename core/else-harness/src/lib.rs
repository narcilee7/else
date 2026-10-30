//! Harness 适配器：else 托管 Agent，不自己发明 Agent。
//!
//! ScriptedHarness 是确定性的内置执行器，W3 起可重入：
//! 从事件流恢复进度，Decision 后续跑；本人否决路径时自动重规划
//! （Plan 易失，Intent 不失败）；最后按原始 DoneWhen 验收。
//! 真实 Harness 经 MVP-2 适配契约（adapter 模块）接入，控制面契约不变。

pub mod adapter;
pub mod real;
pub mod verify;

use else_gateway::{
    ensure_not_fenced, ExecContext, GatewayError, ModelGateway, Result, ToolGateway,
};
use else_ledger::{Ledger, StoredEvent};
use else_protocol::artifact::{Artifact, ArtifactKind, OutcomeEvidence};
use else_protocol::attention::{
    AttentionKind, AttentionRequest, DecisionChoice, DecisionOption, ExpectedLoss,
};
use else_protocol::effect::{SideEffectKind, ToolCall};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::{ArtifactId, AttentionId, ToolCallId};
use else_protocol::intent::{Criterion, IntentState};

pub const SCRIPTED: &str = "scripted";

/// 内置真实 Harness 适配器注册表（MVP-2：Codex 与 Claude Code）。
pub fn builtin_adapters() -> Vec<Box<dyn adapter::HarnessAdapter>> {
    vec![Box::new(real::codex()), Box::new(real::claude_code())]
}

/// 按名解析适配器。
pub fn find_adapter(name: &str) -> Option<Box<dyn adapter::HarnessAdapter>> {
    builtin_adapters()
        .into_iter()
        .find(|adapter| adapter.name() == name)
}

/// Harness 可用性：二进制在 PATH 或常见位置可解析。
/// scripted 是内置执行器，永远可用。
pub fn adapter_available(name: &str) -> bool {
    if name == SCRIPTED {
        return true;
    }
    let binary = match name {
        real::CODEX => "codex",
        real::CLAUDE_CODE => "claude",
        _ => return false,
    };
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(binary).is_file()))
        .unwrap_or(false)
}

/// 规则式 Harness 选择（MVP-3）：显式指定 > 任务类型 > 可用性 > 内置兜底。
/// 不学习、不路由，规则可解释、可复现。
pub fn select_harness(task: &str, hint: Option<&str>) -> &'static str {
    if let Some(hint) = hint {
        if hint != "auto" && adapter_available(hint) {
            return match hint {
                real::CODEX => real::CODEX,
                real::CLAUDE_CODE => real::CLAUDE_CODE,
                _ => SCRIPTED,
            };
        }
    }
    let coding_signals = [
        "代码", "code", "coding", "修复", "bug", "实现", "refactor", "重构", "函数", "测试",
    ];
    let looks_coding = coding_signals
        .iter()
        .any(|signal| task.to_lowercase().contains(signal));
    if looks_coding {
        // 编码任务优先真实 Harness；不可用则降级下一个，最后 scripted 兜底。
        for candidate in [real::CODEX, real::CLAUDE_CODE] {
            if adapter_available(candidate) {
                return candidate;
            }
        }
    }
    SCRIPTED
}

/// 从事件流恢复的进度：Plan 易失，进度判断只依赖事件，不依赖进程内存。
struct Progress {
    last_plan_seq: Option<u64>,
    last_reject_seq: Option<u64>,
}

impl Progress {
    fn from_events(events: &[StoredEvent]) -> Self {
        let mut progress = Self {
            last_plan_seq: None,
            last_reject_seq: None,
        };
        for event in events {
            match &event.envelope.payload {
                EventPayload::PlanProposed { .. } | EventPayload::PlanReplaced { .. } => {
                    progress.last_plan_seq = Some(event.seq);
                }
                EventPayload::DecisionResolved { decision }
                    if matches!(
                        decision.choice,
                        DecisionChoice::Option(ref id) if id == "reject"
                    ) || matches!(decision.choice, DecisionChoice::Dismissed) =>
                {
                    progress.last_reject_seq = Some(event.seq);
                }
                _ => {}
            }
        }
        progress
    }

    fn needs_plan(&self) -> bool {
        self.last_plan_seq.is_none() || self.needs_replan()
    }

    /// 最后一次否决发生在最新计划之后 → 路径失效，需要重规划。
    fn needs_replan(&self) -> bool {
        match (self.last_plan_seq, self.last_reject_seq) {
            (Some(plan), Some(reject)) => reject > plan,
            (None, Some(_)) => true,
            _ => false,
        }
    }

    /// 已按本人否决完成重规划 → 走不依赖 shell 的替代校验路径。
    fn on_alternative_path(&self) -> bool {
        match (self.last_plan_seq, self.last_reject_seq) {
            (Some(plan), Some(reject)) => plan > reject,
            _ => false,
        }
    }
}

/// 运行选项：分支重跑时的可见水位与路径选择。
#[derive(Debug, Default, Clone, Copy)]
pub struct RunOptions {
    /// 检查点事件水位：只有水位之前或当前 Generation 的事件对本次运行可见。
    pub cursor: Option<u64>,
    /// 分支重跑：无视既有计划，重新规划（Rerun 不是续跑）。
    pub force_replan: bool,
    /// 强制走不依赖 shell 的替代路径（换路径重跑）。
    pub alternative: bool,
    /// Subagent 验收只结束自身 Scope，不直接把根 Intent 标记为完成。
    pub subagent: bool,
}

/// 执行一次 Run。等待本人决策（NeedsApproval）不是错误：
/// 它向上传播，由 Runtime 结束本次 Run 并保留现场，Decision 后续跑。
pub fn run(
    ctx: &ExecContext,
    ledger: &Ledger,
    tools: &ToolGateway,
    model: &ModelGateway,
    model_name: &str,
    options: RunOptions,
) -> Result<()> {
    ensure_not_fenced(ledger, ctx)?;
    // 事件可见性：Subagent 只看自身 Scope；分支看检查点水位之前 + 当前 Generation。
    let visible_events = |ledger: &Ledger| -> Result<Vec<StoredEvent>> {
        Ok(ledger
            .read_intent(&ctx.spec.id)?
            .into_iter()
            .filter(|e| {
                if options.subagent {
                    e.envelope.scope_id == ctx.scope_id
                } else {
                    options.cursor.is_none_or(|cursor| e.seq <= cursor)
                        || e.envelope.generation == ctx.generation
                }
            })
            .collect())
    };
    let progress = Progress::from_events(&visible_events(ledger)?);

    // 1. 规划 / 重规划：模型调用经 Model Gateway 入账
    if options.force_replan || progress.needs_plan() {
        let replan = !options.force_replan && progress.needs_replan();
        let prompt = if options.alternative || replan {
            format!(
                "此前执行路径被本人否决。给出不依赖 shell 的替代方案：以只读方式校验工作区产物。目标：{}",
                ctx.spec.outcome
            )
        } else {
            ctx.spec.outcome.clone()
        };
        let plan = model.complete(ctx, "planning", model_name, &prompt)?;
        let plan_id = uuid::Uuid::new_v4().to_string();
        if replan {
            ctx.record(
                ledger,
                Actor::Agent,
                EventPayload::PlanReplaced {
                    plan_id: plan_id.clone(),
                    reason: "本人否决原路径，自动重规划（计划失败不等于 Intent 失败）".into(),
                },
            )?;
        } else {
            ctx.record(
                ledger,
                Actor::Agent,
                EventPayload::PlanProposed {
                    plan_id: plan_id.clone(),
                    summary: plan.text.clone(),
                },
            )?;
        }

        // 写计划产物：幂等键随计划版本，重规划生成新产物
        let _outcome = tools.execute(
            ctx,
            ToolCall {
                id: ToolCallId::new(),
                tool: "fs.write".into(),
                effect: SideEffectKind::Reversible,
                input: serde_json::json!({"path": "plan.md", "content": plan.text}),
                idempotency_key: format!("plan-write-{plan_id}"),
            },
        )?;
        ensure_not_fenced(ledger, ctx)?;
        ctx.record(
            ledger,
            Actor::Agent,
            EventPayload::ArtifactProduced {
                artifact: Artifact {
                    id: ArtifactId::new(),
                    kind: ArtifactKind::Document,
                    // 跨 Scope 的协议只传 Workspace 相对路径，不暴露宿主绝对路径。
                    locator: "plan.md".into(),
                    manifest: None,
                    produced_by: ctx.run_id.clone(),
                    note: "执行计划".into(),
                },
            },
        )?;
    }

    // 规划/重规划已落事件，重算进度以选择正确的校验路径
    let progress = Progress::from_events(&visible_events(ledger)?);

    // 2. 校验：替代路径用只读校验；原路径用 shell（越权则提交本人授权）
    let _verification = if options.alternative || progress.on_alternative_path() {
        tools.execute(
            ctx,
            ToolCall {
                id: ToolCallId::new(),
                tool: "fs.read".into(),
                effect: SideEffectKind::ReadOnly,
                input: serde_json::json!({"path": "plan.md"}),
                idempotency_key: format!("verify-read-{}", ctx.run_id),
            },
        )?
    } else {
        tools.execute(
            ctx,
            ToolCall {
                id: ToolCallId::new(),
                tool: "shell".into(),
                effect: SideEffectKind::Unknown,
                input: serde_json::json!({"command": "test -f plan.md && echo verified"}),
                idempotency_key: "verify-shell".into(),
            },
        )?
    };

    // 3. 验收：按原始 DoneWhen 独立检查产物，不信任校验工具或 Plan 自报结果。
    let verified = crate::verify::evaluate_outcome(tools.workspace(), &ctx.spec.done_when)
        .map_err(|error| GatewayError::Model(format!("outcome verification failed: {error}")))?;
    match verified {
        crate::verify::OutcomeVerification::Verified(evidence) => {
            verify_with_evidence(ctx, ledger, evidence, options.subagent)
        }
        crate::verify::OutcomeVerification::NeedsConfirmation { criteria, .. } => {
            request_manual_verification(ctx, ledger, &criteria)
        }
    }
}

pub(crate) fn request_manual_verification(
    ctx: &ExecContext,
    ledger: &Ledger,
    criteria: &[Criterion],
) -> Result<()> {
    let summary = criteria
        .iter()
        .map(|criterion| criterion.description.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let attention = AttentionRequest {
        id: AttentionId::new(),
        kind: AttentionKind::Verification,
        summary: format!("请确认验收结果：{summary}"),
        blocked_reason: "以下 DoneWhen 无法由 Runtime 机器验证，需要本人确认".into(),
        expected_loss: ExpectedLoss::High,
        blocking: true,
        reversible: false,
        decide_by: None,
        options: vec![
            DecisionOption {
                id: "approve".into(),
                label: "确认结果成立".into(),
                recommended: true,
            },
            DecisionOption {
                id: "reject".into(),
                label: "结果不成立，重新执行".into(),
                recommended: false,
            },
        ],
        default_option: Some("reject".into()),
        related_call_key: Some("manual-outcome-verification".into()),
    };
    let request_id = attention.id.clone();
    let ruling = else_attention::AttentionKernel.rule(
        &attention,
        &ctx.spec.attention_policy,
        chrono::Utc::now(),
    );
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::AttentionRequested { request: attention },
    )?;
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::AttentionRuled {
            request_id: request_id.clone(),
            ruling,
        },
    )?;
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::IntentStateChanged {
            state: IntentState::NeedsDecision,
        },
    )?;
    Err(GatewayError::NeedsApproval(request_id))
}

/// 记录独立 Verifier 已产出的逐项证据。
pub(crate) fn verify_with_evidence(
    ctx: &ExecContext,
    ledger: &Ledger,
    evidence: Vec<OutcomeEvidence>,
    subagent: bool,
) -> Result<()> {
    ensure_not_fenced(ledger, ctx)?;
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::OutcomeVerified { evidence },
    )?;
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::ScopeStateChanged {
            state: else_protocol::event::ScopeState::Completed,
        },
    )?;
    if !subagent {
        ctx.record(
            ledger,
            Actor::System,
            EventPayload::IntentStateChanged {
                state: else_protocol::intent::IntentState::Completed,
            },
        )?;
    }
    Ok(())
}
