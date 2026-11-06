//! W4：Checkpoint → Fork → Rerun → Compare。
//! 检查点三要素、分支工作区隔离、Generation 隔离、三维度比较。

use else_api::Runtime;
use else_protocol::attention::DecisionChoice;
use else_protocol::thread::ThreadState;

#[test]
fn checkpoint_fork_rerun_compare() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("把发布推进到可对外状态", None, Default::default())
        .unwrap();
    let projection = runtime.run(&spec.id).unwrap();

    // 检查点：shell 已调用未完成 → 在途副作用结论必须存在
    let checkpoint = runtime.checkpoint(&spec.id).unwrap();
    assert_eq!(checkpoint.inflight.len(), 1);
    assert!(checkpoint.artifact_root.is_some());
    assert!(checkpoint.event_cursor > 0);

    // 主线 approve 完成
    let request_id = projection.open_requests.keys().next().unwrap().clone();
    let main = runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    assert_eq!(main.state, Some(ThreadState::Completed));
    let main_plan = std::fs::read_to_string(workspace(&runtime, &spec).join("plan.md")).unwrap();

    // 回检查点，换替代路径重跑：Generation 递增、分支工作区隔离
    let (scope_id, generation) = runtime.rerun(&spec.id, &checkpoint.id, true).unwrap();
    assert_eq!(generation, 2);
    let branch_plan_path = dir
        .path()
        .join("branches")
        .join(scope_id.to_string())
        .join("plan.md");
    let branch_plan = std::fs::read_to_string(&branch_plan_path).unwrap();
    assert_ne!(main_plan, branch_plan, "替代路径应产生不同的计划产物");

    // 主线工作区未被分支修改
    assert_eq!(
        std::fs::read_to_string(workspace(&runtime, &spec).join("plan.md")).unwrap(),
        main_plan
    );

    // Compare：两个 Generation 都有验收证据，产物差异可见
    let report = runtime.compare(&spec.id).unwrap();
    assert!(report.contains("generation 1:"), "{report}");
    assert!(report.contains("generation 2:"), "{report}");
    assert_eq!(report.matches("outcome_verified: true").count(), 2);
    assert_eq!(report.matches("sandbox: reserved_seconds=").count(), 2);
    assert!(report.contains("actual_millis="), "{report}");
    assert!(report.contains("reported="), "{report}");
    assert!(report.contains("内容不同"), "{report}");

    // 重开后 Compare 可完整重建（恢复测试）
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(reopened.compare(&spec.id).unwrap(), report);
}

/// Thread 的执行位置：Scratch 模式下每个 Thread 一个目录。
fn workspace(runtime: &Runtime, spec: &else_protocol::thread::ThreadSpec) -> std::path::PathBuf {
    runtime.thread_workspace(spec).unwrap()
}
