//! Manual DoneWhen 必须经过 Decision，不得在 Harness 自报完成时自动通过。

use else_api::{KernelAction, Runtime};
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::attention::{AttentionKind, DecisionChoice};
use else_protocol::event::{EventPayload, ScopeState};
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::intent::{Criterion, IntentState};

struct CompletedParser;

impl LineParser for CompletedParser {
    fn parse(&self, _line: &str) -> Vec<BridgeEvent> {
        vec![BridgeEvent::Completed {
            summary: "done".into(),
        }]
    }
}

fn completed_harness() -> ProcessHarness<CompletedParser> {
    ProcessHarness {
        name: "manual-verification",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Observable,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        timeout: None,
        prepare: Box::new(|_, _, _| {
            Ok(HarnessPrep {
                command: "printf '%s\\n' completed".into(),
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
fn approving_manual_verification_completes_without_rerunning_harness() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_intent_with_done_when(
            "人工验收",
            None,
            vec![Criterion::manual("业务结论符合预期")],
        )
        .unwrap();
    let waiting = runtime.run_adapter(&spec.id, &completed_harness()).unwrap();
    assert_eq!(waiting.state, Some(IntentState::NeedsDecision));
    let actionable = waiting.actionable_requests(chrono::Utc::now());
    let request = actionable.first().unwrap();
    assert_eq!(request.kind, AttentionKind::Verification);
    let request_id = request.id.clone();
    assert!(!runtime
        .events(&spec.id)
        .unwrap()
        .iter()
        .any(|event| { matches!(event.envelope.payload, EventPayload::OutcomeVerified { .. }) }));

    let completed = runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    assert_eq!(completed.state, Some(IntentState::Completed));
    let root = completed.root_scope_id.as_ref().unwrap();
    assert_eq!(completed.scopes[root].state, ScopeState::Completed);
    let events = runtime.events(&spec.id).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.envelope.payload, EventPayload::RunStarted { .. }))
            .count(),
        1,
        "确认验收不能重新执行 Harness"
    );
    assert!(events
        .iter()
        .any(|event| matches!(event.envelope.payload, EventPayload::OutcomeVerified { .. })));
}

#[test]
fn rejecting_manual_verification_leaves_kernel_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_intent_with_done_when(
            "人工验收",
            None,
            vec![Criterion::manual("业务结论符合预期")],
        )
        .unwrap();
    let waiting = runtime.run_adapter(&spec.id, &completed_harness()).unwrap();
    let request = waiting
        .actionable_requests(chrono::Utc::now())
        .first()
        .unwrap()
        .id
        .clone();

    let rejected = runtime
        .decide(&spec.id, &request, DecisionChoice::Option("reject".into()))
        .unwrap();
    assert_eq!(rejected.state, Some(IntentState::Active));
    assert!(!runtime
        .events(&spec.id)
        .unwrap()
        .iter()
        .any(|event| { matches!(event.envelope.payload, EventPayload::OutcomeVerified { .. }) }));
    assert!(matches!(
        runtime.kernel_step(&spec.id, Some("scripted")).unwrap(),
        KernelAction::Run { generation: 2, .. }
    ));
}
