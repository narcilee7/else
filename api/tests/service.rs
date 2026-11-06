//! 常驻 Runtime supervisor：创建 Thread 后自动 reconcile，决策后自动收敛。

use std::time::{Duration, Instant};

use else_api::RuntimeService;
use else_protocol::attention::DecisionChoice;
use else_protocol::thread::ThreadState;

#[test]
fn resident_runtime_reconciles_threads_and_decisions() {
    let dir = tempfile::tempdir().unwrap();
    let service = RuntimeService::start(dir.path()).unwrap();
    let handle = service.handle();
    let spec = handle.create_thread("整理会议纪要", None).unwrap();

    let waiting = wait_until(&handle, &spec.id, ThreadState::NeedsDecision);
    let request = waiting
        .actionable_requests(chrono::Utc::now())
        .first()
        .unwrap()
        .id
        .clone();
    let completed = handle
        .decide(&spec.id, &request, DecisionChoice::Option("approve".into()))
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
