//! Kernel 回归：持久 Scope 身份、单步推进、等待决策与失败后 Generation 递增。

use else_api::{KernelAction, Runtime};
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::attention::DecisionChoice;
use else_protocol::event::ScopeState;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::intent::IntentState;

struct NullParser;

impl LineParser for NullParser {
    fn parse(&self, _line: &str) -> Vec<BridgeEvent> {
        vec![]
    }
}

fn failing_harness() -> ProcessHarness<NullParser> {
    ProcessHarness {
        name: "kernel-failure",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Opaque,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        timeout: None,
        prepare: Box::new(|_, _, _| {
            Ok(HarnessPrep {
                command: "exit 7".into(),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: NullParser,
    }
}

#[test]
fn kernel_drives_one_step_and_waits_for_real_decisions() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();

    let initial = runtime.projection(&spec.id).unwrap();
    let root = initial.root_scope_id.expect("root scope");
    assert_eq!(initial.scopes[&root].state, ScopeState::Running);
    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::Run {
            scope_id,
            generation: 1,
            harness: else_harness::SCRIPTED
        } if scope_id == root
    ));

    let waiting = runtime.advance(&spec.id, None).unwrap();
    assert_eq!(waiting.state, Some(IntentState::NeedsDecision));
    let request = waiting
        .actionable_requests(chrono::Utc::now())
        .first()
        .unwrap()
        .id
        .clone();
    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::WaitForUser { requests } if requests == vec![request.clone()]
    ));

    let completed = runtime
        .decide(&spec.id, &request, DecisionChoice::Option("approve".into()))
        .unwrap();
    assert_eq!(completed.state, Some(IntentState::Completed));
    assert_eq!(completed.scopes[&root].state, ScopeState::Completed);
    assert_eq!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::Idle
    );
}

#[test]
fn failed_run_is_retried_with_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();
    runtime
        .run_adapter(&spec.id, &failing_harness())
        .unwrap_err();
    let projection = runtime.projection(&spec.id).unwrap();
    let root = projection.root_scope_id.as_ref().unwrap();
    assert_eq!(projection.scopes[root].generation, 1);

    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::Run { generation: 2, .. }
    ));

    runtime
        .run_adapter(&spec.id, &failing_harness())
        .unwrap_err();
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.scopes[root].generation, 2);
    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::Run { generation: 3, .. }
    ));
}
