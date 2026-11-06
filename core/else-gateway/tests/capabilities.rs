use else_gateway::{ExecContext, ToolGateway};
use else_ledger::Ledger;
use else_protocol::capability::{
    CapabilityAdapter, CapabilityKind, CapabilityManifest, CapabilityRegistration, McpToolBinding,
    SkillManifest,
};
use else_protocol::effect::{SideEffectKind, ToolCall};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::{
    CapabilityId, LeaseId, McpServerId, RunId, ScopeId, ThreadId, ToolCallId,
};
use else_protocol::resource::{ResourceClass, ResourceLease};
use else_protocol::thread::{AttentionPolicy, Authority, ThreadSpec};

fn context(thread_id: ThreadId) -> ExecContext {
    ExecContext {
        spec: ThreadSpec {
            id: thread_id,
            revision: 1,
            outcome: "capability test".into(),
            done_when: vec![],
            constraints: vec![],
            authority: Authority {
                max_autonomous_effect: SideEffectKind::ReadOnly,
                forbidden: vec![],
            },
            attention_policy: AttentionPolicy {
                allow_interrupt: true,
                batch_interval_minutes: 30,
            },
            mode: Default::default(),
            opaque_harness_policy: Default::default(),
            deadline: None,
            assumptions: vec![],
            project: None,
            origin: None,
            connector_policy: Default::default(),
        },
        scope_id: ScopeId::new(),
        run_id: RunId::new(),
        generation: 1,
        snapshot_id: None,
    }
}

fn lease(ledger: &Ledger, ctx: &ExecContext) {
    let now = chrono::Utc::now();
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::LeaseGranted {
            lease: ResourceLease {
                id: LeaseId::new(),
                scope_id: ctx.scope_id.clone(),
                run_id: Some(ctx.run_id.clone()),
                class: ResourceClass::Sandbox,
                quantity: 10,
                unit: "seconds".into(),
                generation: 1,
                granted_at: Some(now),
                expires_at: now + chrono::Duration::seconds(60),
            },
        },
    )
    .unwrap();
}

#[test]
fn skill_and_mcp_tools_share_gateway_execution() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let ctx = context(ThreadId::new());
    lease(&ledger, &ctx);
    std::fs::write(
        dir.path().join("mcp.sh"),
        r#"while IFS= read -r line; do
case "$line" in
  *'"id":1'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26"}}' ;;
  *'"id":2'*) printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}'
              printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"mcp-ok"}]}}' ;;
esac
done
"#,
    )
    .unwrap();
    let skill = CapabilityRegistration {
        manifest: CapabilityManifest {
            id: CapabilityId::new(),
            name: "checklist".into(),
            description: "skill".into(),
            kind: CapabilityKind::Skill,
            effect: SideEffectKind::ReadOnly,
            idempotency_required: false,
            input_schema: serde_json::json!({"type":"object"}),
        },
        adapter: CapabilityAdapter::Skill(SkillManifest {
            instructions: "do checks".into(),
            resources: vec![],
            capability_ids: vec![],
        }),
    };
    let mcp = CapabilityRegistration {
        manifest: CapabilityManifest {
            id: CapabilityId::new(),
            name: "mcp-read".into(),
            description: "mcp".into(),
            kind: CapabilityKind::McpTool,
            effect: SideEffectKind::ReadOnly,
            idempotency_required: true,
            input_schema: serde_json::json!({"type":"object"}),
        },
        adapter: CapabilityAdapter::Mcp(McpToolBinding {
            server_id: Some(McpServerId::new()),
            command: "sh mcp.sh".into(),
            tool_name: "read".into(),
        }),
    };
    for capability in [skill.clone(), mcp.clone()] {
        ctx.record(
            &ledger,
            Actor::User,
            EventPayload::CapabilityRegistered { capability },
        )
        .unwrap();
    }
    let gateway = ToolGateway::new(&ledger, dir.path().to_path_buf());
    let skill_outcome = gateway
        .execute(
            &ctx,
            ToolCall {
                id: ToolCallId::new(),
                tool: skill.manifest.id.to_string(),
                effect: SideEffectKind::Unknown,
                input: serde_json::json!({}),
                idempotency_key: "skill-call".into(),
            },
        )
        .unwrap();
    assert_eq!(skill_outcome.output["authority_granted"], false);
    let mcp_outcome = gateway
        .execute(
            &ctx,
            ToolCall {
                id: ToolCallId::new(),
                tool: mcp.manifest.id.to_string(),
                effect: SideEffectKind::Unknown,
                input: serde_json::json!({}),
                idempotency_key: "mcp-call".into(),
            },
        )
        .unwrap();
    assert!(mcp_outcome.output["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("mcp-ok"));
    assert!(ledger
        .read_thread(&ctx.spec.id)
        .unwrap()
        .iter()
        .any(|event| { matches!(event.envelope.payload, EventPayload::McpToolsChanged { .. }) }));
}
