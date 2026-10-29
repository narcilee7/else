//! W2 端到端：提交 Intent → 执行（模型/工具过网关并落账）→
//! 越权步骤停在本人决策 → 用户决策后队列清空。

use else_api::Runtime;
use else_protocol::attention::DecisionChoice;
use else_protocol::intent::IntentState;

#[test]
fn intent_executes_and_waits_for_human_decision() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("今天达到可对外发布").unwrap();
    let projection = runtime.run(&spec.id).unwrap();

    // shell（Unknown）越权 → 停在本人决策
    assert_eq!(projection.state, Some(IntentState::NeedsDecision));
    assert_eq!(projection.open_requests.len(), 1);

    // 产物真实写入工作区
    assert_eq!(projection.artifacts.len(), 1);
    assert!(dir.path().join("workspace/plan.md").exists());

    // 模型调用与工具调用均落账
    let events = runtime.events(&spec.id).unwrap();
    let payloads: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(&e.envelope.payload).unwrap())
        .collect();
    assert!(payloads.iter().any(|p| p.contains("model_completed")));
    assert!(payloads.iter().any(|p| p.contains("tool_completed")));
    assert!(payloads.iter().any(|p| p.contains("attention_requested")));

    // 本人决策 → 队列清空、回到 Active
    let request_id = projection.open_requests.keys().next().unwrap().clone();
    runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    let after = runtime.projection(&spec.id).unwrap();
    assert!(after.open_requests.is_empty());
    assert_eq!(after.state, Some(IntentState::Active));
}

#[test]
fn reopening_runtime_replays_same_state() {
    let dir = tempfile::tempdir().unwrap();
    let spec_id = {
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = runtime.create_intent("验证恢复").unwrap();
        runtime.run(&spec.id).unwrap();
        spec.id
    };
    // 模拟重启：重开后投影一致
    let runtime = Runtime::open(dir.path()).unwrap();
    let projection = runtime.projection(&spec_id).unwrap();
    assert_eq!(projection.state, Some(IntentState::NeedsDecision));
    assert_eq!(projection.open_requests.len(), 1);
}
