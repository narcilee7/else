//! Capability Registry 与双向 Agent Binding 的重放回归。

use else_api::Runtime;
use else_protocol::capability::{AgentTransport, CapabilityKind};
use else_protocol::ids::AgentId;
#[cfg(target_os = "macos")]
use else_protocol::thread::OpaqueHarnessPolicy;

#[test]
fn skill_registration_and_agent_binding_are_replayable() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_thread("Capability registry", None).unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let skill = runtime
        .register_skill(
            &spec.id,
            "release-checklist",
            "发布检查清单",
            "检查测试、版本号和发布说明。",
            vec!["docs/release.md".into()],
            vec![],
        )
        .unwrap();
    assert_eq!(skill.manifest.kind, CapabilityKind::Skill);
    let capabilities = runtime.capabilities(&spec.id).unwrap();
    assert!(capabilities
        .iter()
        .any(|capability| capability.manifest.name == "release-checklist"));
    assert!(capabilities
        .iter()
        .any(|capability| capability.manifest.name == "fs.write"));

    let binding = runtime
        .bind_agent(
            &spec.id,
            AgentId::new(),
            "release-agent",
            root,
            None,
            AgentTransport::LocalProcess {
                command: "release-agent".into(),
            },
            vec![skill.manifest.id.clone()],
            60,
        )
        .unwrap();
    assert!(
        runtime
            .agent_bindings(&spec.id)
            .unwrap()
            .iter()
            .any(|active| active.id == binding.id
                && active.inbound_session == binding.inbound_session)
    );
    runtime
        .revoke_agent_binding(&spec.id, &binding.id, "test cleanup")
        .unwrap();
    assert!(runtime.agent_bindings(&spec.id).unwrap().is_empty());

    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    let projection = reopened.projection(&spec.id).unwrap();
    assert!(projection.capabilities.contains_key(&skill.manifest.id));
    assert!(!projection.agent_bindings[&binding.id].active);
}

#[test]
#[cfg(target_os = "macos")]
fn local_process_binding_dispatches_outbound_agent_and_verifies_result() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_thread("Bound agent outcome", None).unwrap();
    runtime
        .configure_opaque_harness(
            &spec.id,
            OpaqueHarnessPolicy {
                enabled: true,
                allow_native_provider_network: false,
                use_tool_gateway: true,
                max_observed_effect: else_protocol::effect::SideEffectKind::Reversible,
            },
        )
        .unwrap();
    std::fs::write(
        dir.path().join("workspace/agent.sh"),
        "printf '%s\\n' '{\"type\":\"output\",\"text\":\"bound-agent-output\"}'\nprintf '%s\\n' '{\"type\":\"completed\",\"summary\":\"done\"}'\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("workspace/plan.md"), "bound result").unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let binding = runtime
        .bind_agent(
            &spec.id,
            AgentId::new(),
            "local-agent",
            root,
            None,
            AgentTransport::LocalProcess {
                command: "sh agent.sh".into(),
            },
            vec![],
            60,
        )
        .unwrap();
    let projection = runtime
        .dispatch_agent_binding(&spec.id, &binding.id)
        .unwrap();
    assert_eq!(
        projection.state,
        Some(else_protocol::thread::ThreadState::Completed)
    );
    assert!(runtime
        .events(&spec.id)
        .unwrap()
        .iter()
        .any(|event| matches!(
            &event.envelope.payload,
            else_protocol::event::EventPayload::HarnessOutput { text }
                if text == "bound-agent-output"
        )));
}

#[test]
fn mcp_discovery_registers_tools_with_manifest_effects() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_thread("MCP discovery", None).unwrap();
    std::fs::write(
        dir.path().join("workspace/mcp.sh"),
        r#"while IFS= read -r line; do
case "$line" in
  *'"id":1'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26"}}' ;;
  *'"id":2'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"search docs","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}},{"name":"send","description":"send message","inputSchema":{"type":"object"},"annotations":{"destructiveHint":true}}]}}' ;;
esac
done
"#,
    )
    .unwrap();
    let discovered = runtime
        .discover_mcp_tools(&spec.id, "docs", "sh mcp.sh")
        .unwrap();
    assert_eq!(discovered.len(), 2);
    assert_eq!(
        discovered[0].manifest.effect,
        else_protocol::effect::SideEffectKind::ReadOnly
    );
    assert_eq!(
        discovered[1].manifest.effect,
        else_protocol::effect::SideEffectKind::Irreversible
    );
    assert_eq!(runtime.capabilities(&spec.id).unwrap().len(), 5);
    drop(runtime);
    else_gateway::mcp_session::shutdown_all();
    let runtime = Runtime::open(dir.path()).unwrap();
    runtime.restore_mcp_sessions().unwrap();
}

#[test]
#[cfg(target_os = "macos")]
fn mcp_stdio_binding_dispatches_task_through_selected_tool() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_thread("MCP binding outcome", None).unwrap();
    runtime
        .configure_opaque_harness(
            &spec.id,
            OpaqueHarnessPolicy {
                enabled: true,
                allow_native_provider_network: false,
                use_tool_gateway: true,
                max_observed_effect: else_protocol::effect::SideEffectKind::Reversible,
            },
        )
        .unwrap();
    std::fs::write(
        dir.path().join("workspace/mcp-agent.sh"),
        r#"while IFS= read -r line; do
case "$line" in
  *'"id":1'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26"}}' ;;
  *'"id":2'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":"agent accepted task"}}' ;;
esac
done
"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("workspace/plan.md"), "mcp result").unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let binding = runtime
        .bind_agent(
            &spec.id,
            AgentId::new(),
            "mcp-agent",
            root,
            None,
            AgentTransport::McpStdio {
                command: "sh mcp-agent.sh".into(),
                tool_name: Some("run".into()),
            },
            vec![],
            60,
        )
        .unwrap();
    let projection = runtime
        .dispatch_agent_binding(&spec.id, &binding.id)
        .unwrap();
    assert_eq!(
        projection.state,
        Some(else_protocol::thread::ThreadState::Completed)
    );
}
