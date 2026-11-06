//! Tool Gateway 的核心承诺：授权边界与幂等。

use else_gateway::{ExecContext, GatewayError, ToolGateway};
use else_ledger::Ledger;
use else_protocol::effect::{SideEffectKind, ToolCall, ToolStatus};
use else_protocol::event::{Actor, EventPayload};
use else_protocol::ids::{RunId, ScopeId, ThreadId, ToolCallId};
use else_protocol::resource::{ResourceClass, ResourceLease};
use else_protocol::thread::{AttentionPolicy, Authority, ThreadSpec};
use serde_json::json;

fn ctx(spec_id: &ThreadId) -> ExecContext {
    ExecContext {
        spec: ThreadSpec {
            id: spec_id.clone(),
            revision: 1,
            outcome: "测试".into(),
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
            opaque_harness_policy: else_protocol::thread::OpaqueHarnessPolicy::default(),
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

fn grant_lease(ledger: &Ledger, ctx: &ExecContext) {
    ctx.record(
        ledger,
        Actor::System,
        EventPayload::LeaseGranted {
            lease: ResourceLease {
                id: else_protocol::ids::LeaseId::new(),
                scope_id: ctx.scope_id.clone(),
                run_id: Some(ctx.run_id.clone()),
                class: ResourceClass::Sandbox,
                quantity: 1,
                unit: "seconds".into(),
                generation: ctx.generation,
                granted_at: Some(chrono::Utc::now()),
                expires_at: chrono::Utc::now() + chrono::Duration::seconds(60),
            },
        },
    )
    .unwrap();
}

fn call(tool: &str, effect: SideEffectKind, input: serde_json::Value, key: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(),
        tool: tool.into(),
        effect,
        input,
        idempotency_key: key.into(),
    }
}

#[test]
fn idempotent_call_is_not_executed_twice() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let gateway = ToolGateway::new(&ledger, workspace.clone());
    let ctx = ctx(&ThreadId::new());
    grant_lease(&ledger, &ctx);

    let first = gateway
        .execute(
            &ctx,
            call(
                "fs.write",
                SideEffectKind::Reversible,
                json!({"path": "a.txt", "content": "first"}),
                "k1",
            ),
        )
        .unwrap();
    // 相同幂等键、不同调用 ID、不同内容：必须直接返回首次结果
    let second = gateway
        .execute(
            &ctx,
            call(
                "fs.write",
                SideEffectKind::Reversible,
                json!({"path": "a.txt", "content": "second"}),
                "k1",
            ),
        )
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(
        std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
        "first"
    );
}

#[test]
fn excessive_side_effect_requires_human_approval() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let gateway = ToolGateway::new(&ledger, dir.path().to_path_buf());
    let ctx = ctx(&ThreadId::new());
    grant_lease(&ledger, &ctx);

    let err = gateway
        .execute(
            &ctx,
            call(
                "shell",
                SideEffectKind::Unknown,
                json!({"command": "echo hi"}),
                "k2",
            ),
        )
        .unwrap_err();
    assert!(matches!(err, GatewayError::NeedsApproval(_)));

    // 已提交 AttentionRequest，而不是静默拒绝
    let events = ledger.read_thread(&ctx.spec.id).unwrap();
    assert!(events.iter().any(|event| matches!(
        event.envelope.payload,
        EventPayload::AttentionRequested { .. }
    )));
    assert!(events.iter().any(|event| matches!(
        &event.envelope.payload,
        EventPayload::AttentionRuled {
            ruling: else_protocol::attention::Ruling::Interrupt,
            ..
        }
    )));
}

#[test]
fn path_escaping_workspace_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let gateway = ToolGateway::new(&ledger, dir.path().to_path_buf());
    let ctx = ctx(&ThreadId::new());
    grant_lease(&ledger, &ctx);

    let outcome = gateway
        .execute(
            &ctx,
            call(
                "fs.write",
                SideEffectKind::Reversible,
                json!({"path": "../evil.txt", "content": "x"}),
                "k3",
            ),
        )
        .unwrap();
    assert_eq!(outcome.status, ToolStatus::Failed);
}

#[test]
fn shell_is_executed_by_native_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    if else_environment::Sandbox::new(else_environment::SandboxProfile::new(&workspace)).is_err() {
        return;
    }
    let ledger = Ledger::open_in_memory().unwrap();
    let gateway = ToolGateway::new(&ledger, workspace.clone());
    let mut ctx = ctx(&ThreadId::new());
    ctx.spec.authority.max_autonomous_effect = SideEffectKind::Unknown;
    grant_lease(&ledger, &ctx);

    let outcome = gateway
        .execute(
            &ctx,
            call(
                "shell",
                SideEffectKind::Reversible,
                json!({"command": "set -e; printf isolated > shell.txt; if cat /etc/passwd >/dev/null 2>&1; then exit 41; fi; echo gateway-sandbox"}),
                "sandboxed-shell",
            ),
        )
        .unwrap();
    assert_eq!(outcome.status, ToolStatus::Succeeded);
    assert_eq!(outcome.output["code"], 0);
    assert!(outcome.output["sandbox"].as_str().is_some());
    assert!(outcome.output["stdout"]
        .as_str()
        .unwrap()
        .contains("gateway-sandbox"));
    assert_eq!(
        std::fs::read_to_string(workspace.join("shell.txt")).unwrap(),
        "isolated"
    );
}

#[cfg(unix)]
#[test]
fn symlink_escaping_workspace_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, "unchanged").unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("dangling-outside.txt"),
        workspace.join("dangling.txt"),
    )
    .unwrap();
    let ledger = Ledger::open_in_memory().unwrap();
    let gateway = ToolGateway::new(&ledger, workspace);
    let ctx = ctx(&ThreadId::new());
    grant_lease(&ledger, &ctx);

    for (index, (tool, input)) in [
        ("fs.read", json!({"path": "link.txt"})),
        (
            "fs.write",
            json!({"path": "link.txt", "content": "overwritten"}),
        ),
        (
            "fs.write",
            json!({"path": "dangling.txt", "content": "created"}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let outcome = gateway
            .execute(
                &ctx,
                call(
                    tool,
                    SideEffectKind::Reversible,
                    input,
                    &format!("symlink-{index}"),
                ),
            )
            .unwrap();
        assert_eq!(outcome.status, ToolStatus::Failed);
    }
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "unchanged");
    assert!(!dir.path().join("dangling-outside.txt").exists());
}
