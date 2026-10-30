//! 依赖失败：有限重试、单次替换、下游重接线或显式 Superseded。

use else_api::{IntentRevision, KernelAction, Runtime};
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::ScopeState;
use else_protocol::failure::FailureClass;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};

struct EmptyParser;

impl LineParser for EmptyParser {
    fn parse(&self, _line: &str) -> Vec<BridgeEvent> {
        vec![]
    }
}

struct CompletedParser;

impl LineParser for CompletedParser {
    fn parse(&self, _line: &str) -> Vec<BridgeEvent> {
        vec![BridgeEvent::Completed {
            summary: "done".into(),
        }]
    }
}

fn failing_harness() -> ProcessHarness<EmptyParser> {
    ProcessHarness {
        name: "dependency-failure",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Observable,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        egress_allowlist: &[],
        timeout: None,
        prepare: Box::new(|_, _, _| {
            Ok(HarnessPrep {
                command: "exit 9".into(),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: EmptyParser,
    }
}

fn successful_harness() -> ProcessHarness<CompletedParser> {
    ProcessHarness {
        name: "dependency-success",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Observable,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        egress_allowlist: &[],
        timeout: None,
        prepare: Box::new(|_, _, _| {
            Ok(HarnessPrep {
                command: "touch plan.md; printf '%s\\n' completed".into(),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: CompletedParser,
    }
}

#[test]
fn terminal_dependency_failure_gets_one_replacement_and_rewires_downstream() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("依赖自动收敛", None).unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let failed = runtime
        .spawn_subagent(&spec.id, &root, "失败上游", SideEffectKind::ReadOnly)
        .unwrap();
    let downstream = runtime
        .spawn_subagent_with_dependencies(
            &spec.id,
            &root,
            "依赖上游的下游",
            SideEffectKind::ReadOnly,
            vec![failed.clone()],
        )
        .unwrap();

    for attempt in 1..=3 {
        assert!(runtime
            .dispatch_scope_adapter(&spec.id, &failed, &failing_harness())
            .is_err());
        let projection = runtime.projection(&spec.id).unwrap();
        assert_eq!(projection.scopes[&failed].failure_count, attempt);
        assert_eq!(
            projection.scopes[&failed]
                .last_failure
                .as_ref()
                .unwrap()
                .class,
            FailureClass::Retryable
        );
        assert_eq!(
            projection.scopes[&failed].state,
            if attempt < 3 {
                ScopeState::Running
            } else {
                ScopeState::Failed
            }
        );
    }

    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::ConvergeDependency { scope_id, dependencies }
            if scope_id == downstream && dependencies == vec![failed.clone()]
    ));
    let projection = runtime.advance(&spec.id, None).unwrap();
    let replacement = projection.scope_replacements[&failed].clone();
    assert_eq!(projection.scopes[&replacement].state, ScopeState::Running);
    assert!(projection.scopes[&downstream]
        .depends_on
        .contains(&replacement));
    assert!(!projection.scopes[&downstream].depends_on.contains(&failed));
    assert!(projection
        .current_plan()
        .unwrap()
        .steps
        .iter()
        .any(|step| step.scope_id.as_ref() == Some(&replacement)));

    runtime
        .dispatch_scope_adapter(&spec.id, &replacement, &successful_harness())
        .unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.scopes[&replacement].state, ScopeState::Completed);
    assert_ne!(projection.scopes[&downstream].state, ScopeState::Superseded);
    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    let replayed = reopened.projection(&spec.id).unwrap();
    assert_eq!(replayed.scope_replacements[&failed], replacement);
    assert!(replayed.scopes[&downstream]
        .depends_on
        .contains(&replacement));
}

#[test]
fn user_cancelled_dependency_supersedes_downstream_without_retrying_it() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("用户取消上游", None).unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let cancelled = runtime
        .spawn_subagent(&spec.id, &root, "将取消的上游", SideEffectKind::ReadOnly)
        .unwrap();
    let downstream = runtime
        .spawn_subagent_with_dependencies(
            &spec.id,
            &root,
            "保留但依赖失效的下游",
            SideEffectKind::ReadOnly,
            vec![cancelled.clone()],
        )
        .unwrap();

    runtime
        .revise_intent(
            &spec.id,
            IntentRevision {
                reason: "只取消上游".into(),
                ..IntentRevision::default()
            },
            std::slice::from_ref(&downstream),
        )
        .unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.scopes[&cancelled].state, ScopeState::Cancelled);
    assert_eq!(projection.scopes[&downstream].state, ScopeState::Running);
    let projection = runtime.advance(&spec.id, None).unwrap();
    assert_eq!(projection.scopes[&downstream].state, ScopeState::Superseded);
    assert!(!projection.scope_replacements.contains_key(&cancelled));
}

#[test]
fn failed_replacement_is_not_replaced_again_and_downstream_is_cut() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("替换链只允许一层", None).unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let failed = runtime
        .spawn_subagent(&spec.id, &root, "原失败节点", SideEffectKind::ReadOnly)
        .unwrap();
    let downstream = runtime
        .spawn_subagent_with_dependencies(
            &spec.id,
            &root,
            "替换后再失败的下游",
            SideEffectKind::ReadOnly,
            vec![failed.clone()],
        )
        .unwrap();
    for _ in 0..3 {
        let _ = runtime.dispatch_scope_adapter(&spec.id, &failed, &failing_harness());
    }
    let projection = runtime.advance(&spec.id, None).unwrap();
    let replacement = projection.scope_replacements[&failed].clone();

    for _ in 0..3 {
        let _ = runtime.dispatch_scope_adapter(&spec.id, &replacement, &failing_harness());
    }
    let projection = runtime.advance(&spec.id, None).unwrap();
    assert_eq!(projection.scopes[&replacement].state, ScopeState::Failed);
    assert_eq!(projection.scopes[&downstream].state, ScopeState::Superseded);
    assert_eq!(projection.scope_replacements.len(), 1);
}
