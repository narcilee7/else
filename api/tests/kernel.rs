//! Kernel 回归：持久 Scope 身份、单步推进、等待决策与失败后 Generation 递增。

use else_api::{KernelAction, KernelDriveStop, Runtime};
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::attention::DecisionChoice;
use else_protocol::effect::SideEffectKind;
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

struct CompletedParser;

impl LineParser for CompletedParser {
    fn parse(&self, _line: &str) -> Vec<BridgeEvent> {
        vec![BridgeEvent::Completed {
            summary: "done".into(),
        }]
    }
}

fn failing_harness() -> ProcessHarness<NullParser> {
    ProcessHarness {
        name: "kernel-failure",
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

fn successful_child_harness() -> ProcessHarness<CompletedParser> {
    ProcessHarness {
        name: "kernel-success",
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

#[test]
fn kernel_respects_scope_dependencies_and_fair_root_order() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();
    runtime
        .run_adapter(&spec.id, &failing_harness())
        .unwrap_err();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let child = runtime
        .spawn_subagent(&spec.id, &root, "整理附录", SideEffectKind::ReadOnly)
        .unwrap();
    let dependent = runtime
        .spawn_subagent_with_dependencies(
            &spec.id,
            &root,
            "汇总附录",
            SideEffectKind::ReadOnly,
            vec![child.clone()],
        )
        .unwrap();

    assert!(matches!(
        runtime.kernel_step(&spec.id, Some("scripted")).unwrap(),
        KernelAction::Run { scope_id, generation: 1, .. } if scope_id == child
    ));
    runtime
        .dispatch_scope_adapter(&spec.id, &child, &successful_child_harness())
        .unwrap();
    let next = runtime.kernel_step(&spec.id, Some("scripted")).unwrap();
    assert!(
        matches!(
            &next,
            KernelAction::Run { scope_id, generation: 1, .. } if scope_id == &dependent
        ),
        "实际下一步：{next:?}"
    );
    runtime
        .dispatch_scope_adapter(&spec.id, &dependent, &successful_child_harness())
        .unwrap();
    assert!(matches!(
        runtime.kernel_step(&spec.id, Some("scripted")).unwrap(),
        KernelAction::Run { scope_id, generation: 2, .. } if scope_id == root
    ));
}

#[test]
fn due_timer_wakes_drive_until_the_next_wait_state() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();
    runtime
        .schedule_timer(
            &spec.id,
            chrono::Utc::now() - chrono::Duration::seconds(1),
            "test wake",
        )
        .unwrap();

    let driven = runtime.drive(&spec.id, Some("scripted"), 8).unwrap();
    assert_eq!(driven.stopped, KernelDriveStop::WaitForUser);
    assert_eq!(driven.steps, 2, "fire timer then run root");
    assert!(driven.projection.timers.is_empty());
    assert_eq!(driven.projection.state, Some(IntentState::NeedsDecision));
}

#[test]
fn future_timer_is_a_durable_wait_condition() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();
    let fire_at = chrono::Utc::now() + chrono::Duration::hours(1);
    runtime
        .schedule_timer(&spec.id, fire_at, "follow up later")
        .unwrap();
    drop(runtime);
    let runtime = Runtime::open(dir.path()).unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.next_timer_at(), Some(fire_at));
    // 可执行 Scope 仍应继续推进；Timer 只在没有 ready work 时成为等待条件。
    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::Run { .. }
    ));
}

#[test]
fn external_event_is_durable_and_can_wake_kernel_progress() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();
    runtime
        .record_external_event(&spec.id, "mail", "合作方已回复")
        .unwrap();
    assert!(runtime.events(&spec.id).unwrap().iter().any(|event| matches!(
        &event.envelope.payload,
        else_protocol::event::EventPayload::ExternalEventReceived { source, .. } if source == "mail"
    )));

    let driven = runtime.drive(&spec.id, Some("scripted"), 4).unwrap();
    assert_eq!(driven.stopped, KernelDriveStop::WaitForUser);
}

#[test]
fn waiting_external_blocks_execution_and_feeds_the_next_context() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();
    runtime
        .wait_for_external(&spec.id, "等待合作方回复")
        .unwrap();
    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::WaitForExternal { reason: Some(reason) } if reason == "等待合作方回复"
    ));

    runtime
        .record_external_event(&spec.id, "mail", "合作方已回复")
        .unwrap();
    let snapshot = runtime
        .compile_snapshot(&spec.id, else_protocol::memory::SnapshotAudience::Planner)
        .unwrap();
    assert_eq!(snapshot.external_events.len(), 1);
    assert_eq!(snapshot.external_events[0].source, "mail");
    assert_eq!(snapshot.external_events[0].summary, "合作方已回复");
    assert!(matches!(
        runtime.kernel_step(&spec.id, None).unwrap(),
        KernelAction::Run { .. }
    ));
}

#[test]
fn due_timer_resumes_an_external_wait() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理会议纪要", None).unwrap();
    runtime.wait_for_external(&spec.id, "等待异步任务").unwrap();
    runtime
        .schedule_timer(
            &spec.id,
            chrono::Utc::now() - chrono::Duration::seconds(1),
            "poll async task",
        )
        .unwrap();

    let driven = runtime.drive(&spec.id, Some("scripted"), 8).unwrap();
    assert_eq!(driven.stopped, KernelDriveStop::WaitForUser);
    assert_eq!(driven.projection.external_wait_reason, None);
    assert_ne!(driven.projection.state, Some(IntentState::WaitingExternal));
}
