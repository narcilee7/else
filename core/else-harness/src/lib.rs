//! Harness 适配器：else 托管 Agent，不自己发明 Agent。
//!
//! W2 提供 ScriptedHarness——确定性的内置执行器，用来验证控制面：
//! 规划（模型入账）→ 写产物（副作用过 Gateway）→ 遇到越权操作
//! 提交本人决策。真实 Harness（Claude Code、Codex 等）后续经
//! Spawn 协议以子进程接入，控制面契约不变。

use else_gateway::{ExecContext, ModelGateway, Result, ToolGateway};
use else_ledger::Ledger;
use else_protocol::artifact::{Artifact, ArtifactKind};
use else_protocol::effect::{SideEffectKind, ToolCall};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::{ArtifactId, ToolCallId};

pub const SCRIPTED: &str = "scripted";

/// 执行一次 Run。等待本人决策（NeedsApproval）不是错误：
/// 它向上传播，由 Runtime 结束本次 Run 并保留现场。
pub fn run(
    ctx: &ExecContext,
    ledger: &Ledger,
    tools: &ToolGateway,
    model: &ModelGateway,
) -> Result<()> {
    // 1. 规划：模型调用经 Model Gateway，Token/费用入账
    let plan = model.complete(ctx, "planning", "mock-model", &ctx.spec.outcome)?;
    ctx.record(
        ledger,
        Actor::Agent,
        EventPayload::PlanProposed {
            plan_id: uuid::Uuid::new_v4().to_string(),
            summary: plan.text.clone(),
        },
    )?;

    // 2. 写产物：Reversible 副作用，在默认授权范围内
    let outcome = tools.execute(
        ctx,
        ToolCall {
            id: ToolCallId::new(),
            tool: "fs.write".into(),
            effect: SideEffectKind::Reversible,
            input: serde_json::json!({"path": "plan.md", "content": plan.text}),
            idempotency_key: format!("{}-plan", ctx.run_id),
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

    // 3. 校验步骤：shell 声明为 Unknown，默认超出授权 →
    //    触发 AttentionRequest，演示"只有本人能决定什么"。
    tools.execute(
        ctx,
        ToolCall {
            id: ToolCallId::new(),
            tool: "shell".into(),
            effect: SideEffectKind::Unknown,
            input: serde_json::json!({"command": "test -f plan.md && echo verified"}),
            idempotency_key: format!("{}-verify", ctx.run_id),
        },
    )?;

    Ok(())
}
