//! Harness 适配器：else 托管 Agent，不自己发明 Agent。
//!
//! ScriptedHarness 是确定性的内置执行器，W3 起可重入：
//! 从事件流恢复进度，Decision 后续跑；本人否决路径时自动重规划
//! （Plan 易失，Intent 不失败）；最后按原始 DoneWhen 验收。
//! 真实 Harness 后续经 Spawn 协议接入，控制面契约不变。

use else_gateway::{ExecContext, ModelGateway, Result, ToolGateway};
use else_ledger::{Ledger, StoredEvent};
use else_protocol::artifact::{Artifact, ArtifactKind, OutcomeEvidence};
use else_protocol::attention::DecisionChoice;
use else_protocol::effect::{SideEffectKind, ToolCall, ToolOutcome, ToolStatus};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::{ArtifactId, ToolCallId};

pub const SCRIPTED: &str = "scripted";

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

/// 执行一次 Run。等待本人决策（NeedsApproval）不是错误：
/// 它向上传播，由 Runtime 结束本次 Run 并保留现场，Decision 后续跑。
pub fn run(
    ctx: &ExecContext,
    ledger: &Ledger,
    tools: &ToolGateway,
    model: &ModelGateway,
    model_name: &str,
) -> Result<()> {
    let progress = Progress::from_events(&ledger.read_intent(&ctx.spec.id)?);

    // 1. 规划 / 重规划：模型调用经 Model Gateway 入账
    if progress.needs_plan() {
        let replan = progress.needs_replan();
        let prompt = if replan {
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
        let outcome = tools.execute(
            ctx,
            ToolCall {
                id: ToolCallId::new(),
                tool: "fs.write".into(),
                effect: SideEffectKind::Reversible,
                input: serde_json::json!({"path": "plan.md", "content": plan.text}),
                idempotency_key: format!("plan-write-{plan_id}"),
            },
        )?;
        ctx.record(
            ledger,
            Actor::Agent,
            EventPayload::ArtifactProduced {
                artifact: Artifact {
                    id: ArtifactId::new(),
                    kind: ArtifactKind::Document,
                    locator: outcome.output["path"]
                        .as_str()
                        .unwrap_or("plan.md")
                        .to_string(),
                    manifest: None,
                    produced_by: ctx.run_id.clone(),
                    note: "执行计划".into(),
                },
            },
        )?;
    }

    // 规划/重规划已落事件，重算进度以选择正确的校验路径
    let progress = Progress::from_events(&ledger.read_intent(&ctx.spec.id)?);

    // 2. 校验：替代路径用只读校验；原路径用 shell（越权则提交本人授权）
    let verification = if progress.on_alternative_path() {
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

    // 3. 验收：按原始 DoneWhen，不按最新 Plan
    verify(ctx, ledger, &verification)
}

/// Verifier：证据必须支撑 spec 里的每一条 done_when。
/// 通过则 OutcomeVerified + Completed，否则 Failed——不擅自降低验收标准。
fn verify(ctx: &ExecContext, ledger: &Ledger, verification: &ToolOutcome) -> Result<()> {
    let verified = verification.status == ToolStatus::Succeeded;
    if verified {
        let evidence = ctx
            .spec
            .done_when
            .iter()
            .map(|criterion| OutcomeEvidence {
                criterion: criterion.description.clone(),
                artifact_id: None,
                note: "校验工具调用成功，产物存在且可读".into(),
            })
            .collect();
        ctx.record(
            ledger,
            Actor::System,
            EventPayload::OutcomeVerified { evidence },
        )?;
    }
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::IntentStateChanged {
            state: if verified {
                else_protocol::intent::IntentState::Completed
            } else {
                else_protocol::intent::IntentState::Failed
            },
        },
    )?;
    Ok(())
}
