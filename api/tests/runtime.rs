//! W3 端到端闭环：
//! approve → 原路径自动续跑到验收；reject → 自动重规划到验收；
//! 终态幂等；重启重放一致。

use else_api::Runtime;
use else_protocol::attention::DecisionChoice;
use else_protocol::ids::ThreadId;
use else_protocol::thread::ThreadState;

fn payloads(runtime: &Runtime, thread_id: &ThreadId) -> Vec<String> {
    runtime
        .events(thread_id)
        .unwrap()
        .iter()
        .map(|e| serde_json::to_string(&e.envelope.payload).unwrap())
        .collect()
}

#[test]
fn approve_path_resumes_and_completes() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("今天达到可对外发布", None, Default::default())
        .unwrap();

    // 默认验收标准与显式假设由系统补全（Thread Resolution 最小实现）
    assert!(!spec.done_when.is_empty());
    assert!(!spec.assumptions.is_empty());
    assert!(!spec.assumptions[0].confirmed);

    let projection = runtime.run(&spec.id).unwrap();
    assert_eq!(projection.state, Some(ThreadState::NeedsDecision));
    assert_eq!(projection.open_requests.len(), 1);
    assert!(workspace(&runtime, &spec).join("plan.md").exists());

    // approve → 一次性授权放行 shell → 自动续跑 → 按 DoneWhen 验收
    let request_id = projection.open_requests.keys().next().unwrap().clone();
    let after = runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    assert_eq!(after.state, Some(ThreadState::Completed));
    assert!(after.open_requests.is_empty());

    let events = payloads(&runtime, &spec.id);
    assert!(events.iter().any(|p| p.contains("outcome_verified")));
    assert!(events.iter().any(|p| p.contains("verified")));
}

#[test]
fn reject_path_replans_and_completes_without_asking_again() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("验证自动重规划", None, Default::default())
        .unwrap();
    let projection = runtime.run(&spec.id).unwrap();
    let request_id = projection.open_requests.keys().next().unwrap().clone();

    // reject → 自动重规划 → 只读替代路径校验 → 完成，不再打扰用户
    let after = runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("reject".into()),
        )
        .unwrap();
    assert_eq!(after.state, Some(ThreadState::Completed));
    assert!(after.open_requests.is_empty());

    let events = payloads(&runtime, &spec.id);
    assert!(events.iter().any(|p| p.contains("plan_replaced")));
    assert!(events.iter().any(|p| p.contains("outcome_verified")));
    // 重规划产生了新的计划产物
    assert_eq!(after.artifacts.len(), 2);
    // 只有一次本人决策请求，没有第二次打扰
    let attention_count = events
        .iter()
        .filter(|p| p.contains("attention_requested"))
        .count();
    assert_eq!(attention_count, 1);
}

#[test]
fn terminal_state_run_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("终态幂等", None, Default::default())
        .unwrap();
    let projection = runtime.run(&spec.id).unwrap();
    let request_id = projection.open_requests.keys().next().unwrap().clone();
    runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();

    let before = runtime.events(&spec.id).unwrap().len();
    let again = runtime.run(&spec.id).unwrap();
    assert_eq!(again.state, Some(ThreadState::Completed));
    assert_eq!(runtime.events(&spec.id).unwrap().len(), before);
}

#[test]
fn reopening_runtime_replays_same_state() {
    let dir = tempfile::tempdir().unwrap();
    let spec_id = {
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime
            .create_thread("验证恢复", None, Default::default())
            .unwrap();
        runtime.run(&spec.id).unwrap();
        spec.id
    };
    // 模拟重启：重开后投影一致
    let runtime = Runtime::open(dir.path()).unwrap();
    let projection = runtime.projection(&spec_id).unwrap();
    assert_eq!(projection.state, Some(ThreadState::NeedsDecision));
    assert_eq!(projection.open_requests.len(), 1);
}

/// Thread 的执行位置：Scratch 模式下每个 Thread 一个目录。
fn workspace(runtime: &Runtime, spec: &else_protocol::thread::ThreadSpec) -> std::path::PathBuf {
    runtime.thread_workspace(spec).unwrap()
}
