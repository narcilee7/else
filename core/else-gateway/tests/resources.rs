//! M1-W3：Gateway 的 Lease 与预算必须失败关闭。

use else_gateway::{ExecContext, GatewayError, MockProvider, ModelGateway, ToolGateway};
use else_ledger::Ledger;
use else_protocol::effect::{SideEffectKind, ToolCall};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::{IntentId, RunId, ScopeId, ToolCallId};
use else_protocol::intent::{AttentionPolicy, Authority, IntentSpec};
use else_protocol::resource::{ResourceBudget, ResourceClass, ResourceLease};
use serde_json::json;

fn ctx() -> ExecContext {
    ExecContext {
        spec: IntentSpec {
            id: IntentId::new(),
            revision: 1,
            outcome: "资源测试".into(),
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
            project: None,
        },
        scope_id: ScopeId::new(),
        run_id: RunId::new(),
        generation: 1,
        snapshot_id: None,
    }
}

fn call() -> ToolCall {
    ToolCall {
        id: ToolCallId::new(),
        tool: "fs.write".into(),
        effect: SideEffectKind::Reversible,
        input: json!({"path": "blocked.txt", "content": "x"}),
        idempotency_key: "resource-test".into(),
    }
}

fn grant_lease(ledger: &Ledger, ctx: &ExecContext) -> ResourceLease {
    let lease = ResourceLease {
        id: else_protocol::ids::LeaseId::new(),
        scope_id: ctx.scope_id.clone(),
        run_id: Some(ctx.run_id.clone()),
        class: ResourceClass::Sandbox,
        quantity: 1,
        unit: "seconds".into(),
        generation: ctx.generation,
        granted_at: Some(chrono::Utc::now()),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(60),
    };
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::LeaseGranted {
            lease: lease.clone(),
        },
    )
    .unwrap();
    lease
}

fn set_budget(ledger: &Ledger, ctx: &ExecContext, budget: ResourceBudget) {
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::ResourceBudgetSet { budget },
    )
    .unwrap();
}

#[test]
fn missing_or_revoked_lease_blocks_tool_and_model_calls() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let tools = ToolGateway::new(&ledger, dir.path().to_path_buf());
    let model = ModelGateway::new(&ledger, Box::new(MockProvider));
    let ctx = ctx();

    assert!(matches!(
        tools.execute(&ctx, call()),
        Err(GatewayError::LeaseRequired(_))
    ));
    assert!(matches!(
        model.complete(&ctx, "planning", "mock-model", "test"),
        Err(GatewayError::LeaseRequired(_))
    ));

    let lease = grant_lease(&ledger, &ctx);
    ctx.record(
        &ledger,
        Actor::System,
        EventPayload::LeaseRevoked { lease_id: lease.id },
    )
    .unwrap();
    assert!(matches!(
        tools.execute(&ctx, call()),
        Err(GatewayError::LeaseRequired(_))
    ));
    assert!(!dir.path().join("blocked.txt").exists());
}

#[test]
fn model_estimate_over_budget_blocks_provider_call() {
    let ledger = Ledger::open_in_memory().unwrap();
    let model = ModelGateway::new(&ledger, Box::new(MockProvider));
    let ctx = ctx();
    grant_lease(&ledger, &ctx);
    set_budget(
        &ledger,
        &ctx,
        ResourceBudget {
            max_model_spend_usd_micros: 1_000,
            ..Default::default()
        },
    );

    assert!(matches!(
        model.complete(&ctx, "planning", "mock-model", "test"),
        Err(GatewayError::ResourceExhausted { .. })
    ));
    assert!(!ledger
        .read_intent(&ctx.spec.id)
        .unwrap()
        .iter()
        .any(|event| matches!(event.envelope.payload, EventPayload::ModelCalled { .. })));
}

#[test]
fn tool_budget_is_checked_before_tool_called_or_side_effect() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let tools = ToolGateway::new(&ledger, dir.path().to_path_buf());
    let ctx = ctx();
    grant_lease(&ledger, &ctx);
    set_budget(
        &ledger,
        &ctx,
        ResourceBudget {
            max_tool_calls: 0,
            ..Default::default()
        },
    );

    assert!(matches!(
        tools.execute(&ctx, call()),
        Err(GatewayError::ResourceExhausted { .. })
    ));
    assert!(!dir.path().join("blocked.txt").exists());
    assert!(!ledger
        .read_intent(&ctx.spec.id)
        .unwrap()
        .iter()
        .any(|event| matches!(event.envelope.payload, EventPayload::ToolCalled { .. })));
}

struct ActualUsageProvider;

impl else_gateway::ModelProvider for ActualUsageProvider {
    fn complete(
        &self,
        _request: &else_gateway::ModelRequest,
    ) -> std::result::Result<else_gateway::ModelResponse, String> {
        Ok(else_gateway::ModelResponse {
            text: "done".into(),
            usage: else_protocol::resource::ModelUsage {
                input_tokens: 10,
                output_tokens: 10,
                cost_usd_micros: 2_000,
                basis: else_protocol::resource::UsageBasis::Reported,
            },
        })
    }
}

#[test]
fn actual_model_usage_is_rechecked_before_accounting() {
    let ledger = Ledger::open_in_memory().unwrap();
    let model = ModelGateway::new(&ledger, Box::new(ActualUsageProvider));
    let ctx = ctx();
    grant_lease(&ledger, &ctx);
    set_budget(
        &ledger,
        &ctx,
        ResourceBudget {
            max_model_spend_usd_micros: 1_500,
            ..Default::default()
        },
    );

    assert!(matches!(
        model.complete(&ctx, "planning", "mock-model", "test"),
        Err(GatewayError::ResourceExhausted { .. })
    ));
    let events = ledger.read_intent(&ctx.spec.id).unwrap();
    assert!(events
        .iter()
        .any(|event| matches!(event.envelope.payload, EventPayload::ModelCalled { .. })));
    assert!(!events
        .iter()
        .any(|event| matches!(event.envelope.payload, EventPayload::ModelCompleted { .. })));
}
