//! M1 Generation Fencing：取消与旧世代的调用、回调都不能再落账。

use else_gateway::{ExecContext, GatewayError, MockProvider, ModelGateway, ToolGateway};
use else_ledger::Ledger;
use else_protocol::effect::{EffectConclusion, SideEffectKind, ToolCall, ToolOutcome, ToolStatus};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::{IntentId, RunId, ScopeId, ToolCallId};
use else_protocol::intent::{AttentionPolicy, Authority, IntentSpec};
use serde_json::json;

fn ctx(intent_id: &IntentId, generation: u64) -> ExecContext {
    ExecContext {
        spec: IntentSpec {
            id: intent_id.clone(),
            revision: 1,
            outcome: "围栏测试".into(),
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
            opaque_harness_policy: else_protocol::intent::OpaqueHarnessPolicy::default(),
            deadline: None,
            assumptions: vec![],
            project: None,
            origin: None,
            connector_policy: Default::default(),
        },
        scope_id: ScopeId::new(),
        run_id: RunId::new(),
        generation,
        snapshot_id: None,
    }
}

fn write_call() -> ToolCall {
    ToolCall {
        id: ToolCallId::new(),
        tool: "fs.write".into(),
        effect: SideEffectKind::Reversible,
        input: json!({"path": "forbidden.txt", "content": "must not exist"}),
        idempotency_key: "fenced-write".into(),
    }
}

#[test]
fn aborted_generation_rejects_new_work_model_calls_and_late_callbacks() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let tools = ToolGateway::new(&ledger, dir.path().to_path_buf());
    let model = ModelGateway::new(&ledger, Box::new(MockProvider));
    let ctx = ctx(&IntentId::new(), 1);
    let pending = write_call();

    // 模拟一个已经发出、尚未完成的旧调用，随后 Abort。
    ctx.record(
        &ledger,
        Actor::Agent,
        EventPayload::ToolCalled {
            call: pending.clone(),
        },
    )
    .unwrap();
    ctx.record(
        &ledger,
        Actor::User,
        EventPayload::ScopeAbortRequested {
            reason: "停止旧执行".into(),
            scope_only: None,
        },
    )
    .unwrap();

    let late = tools.complete(
        &ctx,
        pending.id,
        ToolOutcome {
            status: ToolStatus::Succeeded,
            output: json!({}),
            effect: EffectConclusion::Applied,
        },
    );
    assert!(matches!(late, Err(GatewayError::GenerationFenced { .. })));
    assert!(matches!(
        tools.execute(&ctx, write_call()),
        Err(GatewayError::GenerationFenced { .. })
    ));
    assert!(matches!(
        model.complete(&ctx, "planning", "mock-model", "旧模型调用"),
        Err(GatewayError::GenerationFenced { .. })
    ));
    assert!(!dir.path().join("forbidden.txt").exists());

    let events = ledger.read_intent(&ctx.spec.id).unwrap();
    assert!(!events.iter().any(|event| matches!(
        event.envelope.payload,
        EventPayload::ToolCompleted { .. } | EventPayload::ModelCalled { .. }
    )));
}

#[test]
fn newer_generation_fences_old_execution_without_an_abort_event() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let tools = ToolGateway::new(&ledger, dir.path().to_path_buf());
    let intent_id = IntentId::new();
    let old = ctx(&intent_id, 1);
    let new = ctx(&intent_id, 2);
    new.record(
        &ledger,
        Actor::System,
        EventPayload::RunStarted {
            run_id: new.run_id.clone(),
            harness: "scripted".into(),
            capabilities: None,
        },
    )
    .unwrap();

    assert!(matches!(
        tools.execute(&old, write_call()),
        Err(GatewayError::GenerationFenced { generation: 1 })
    ));
    assert!(!dir.path().join("forbidden.txt").exists());
}
