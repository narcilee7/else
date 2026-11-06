//! 常驻 Runtime supervisor：创建 Thread 后不替用户抢跑，
//! 用户推进后才自动收敛。

use std::time::{Duration, Instant};

use else_api::RuntimeService;
use else_protocol::attention::DecisionChoice;
use else_protocol::thread::ThreadState;

#[test]
fn resident_runtime_waits_for_the_user_before_running() {
    let dir = tempfile::tempdir().unwrap();
    let service = RuntimeService::start(dir.path()).unwrap();
    let handle = service.handle();
    let spec = handle
        .create_thread("整理会议纪要", None, Default::default())
        .unwrap();

    // 后台 reconcile 只推进调度层：刚创建的 Thread 不应该被它拉起来跑。
    std::thread::sleep(Duration::from_millis(300));
    let projection = handle.projection(&spec.id).unwrap();
    assert!(
        projection.state.is_none(),
        "后台不应替用户启动 Run，实际状态：{:?}",
        projection.state
    );
    assert!(
        !handle
            .events(&spec.id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event.envelope.payload,
                else_protocol::event::EventPayload::RunStarted { .. }
            )),
        "后台不应替用户启动 Run"
    );

    // 用户显式推进后，仍然按原来的节奏收敛。
    handle.advance(&spec.id, None).unwrap();
    let waiting = wait_until(&handle, &spec.id, ThreadState::NeedsDecision);
    let request = waiting
        .actionable_requests(chrono::Utc::now())
        .first()
        .unwrap()
        .id
        .clone();
    // 默认验收是人工确认：批准执行之后还需要确认结果。
    handle
        .decide(&spec.id, &request, DecisionChoice::Option("approve".into()))
        .unwrap();
    let confirming = wait_until(&handle, &spec.id, ThreadState::NeedsDecision);
    let confirm = confirming
        .actionable_requests(chrono::Utc::now())
        .first()
        .map(|request| request.id.clone())
        .expect("verification request");
    let completed = handle
        .decide(&spec.id, &confirm, DecisionChoice::Option("approve".into()))
        .unwrap();
    assert_eq!(completed.state, Some(ThreadState::Completed));
    handle.shutdown().unwrap();
    service.shutdown();
}

fn wait_until(
    handle: &else_api::RuntimeHandle,
    thread_id: &else_protocol::ids::ThreadId,
    expected: ThreadState,
) -> else_ledger::ThreadProjection {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let projection = handle.projection(thread_id).unwrap();
        if projection.state == Some(expected) {
            return projection;
        }
        assert!(
            Instant::now() < deadline,
            "runtime did not reach {expected:?}; current: {:?}",
            projection.state
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
