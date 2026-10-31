//! Capability Registry 与双向 Agent Binding 的重放回归。

use else_api::Runtime;
use else_protocol::capability::{AgentTransport, CapabilityKind};
use else_protocol::ids::AgentId;

#[test]
fn skill_registration_and_agent_binding_are_replayable() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("Capability registry", None).unwrap();
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
