use else_api::Runtime;

#[test]
fn data_export_contains_ledger_and_artifacts_without_private_state() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("导出测试", None, Default::default())
        .unwrap();
    runtime.drive(&spec.id, None, 16).unwrap();

    std::fs::create_dir_all(workspace(&runtime, &spec).join(".codex")).unwrap();
    std::fs::create_dir_all(dir.path().join("harness-private")).unwrap();
    std::fs::write(
        workspace(&runtime, &spec).join(".codex/provider-token"),
        "must-not-export",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("harness-private/session-secret"),
        "must-not-export-either",
    )
    .unwrap();

    let export = runtime.export_data().unwrap();
    assert_eq!(export.export_version, 1);
    assert_eq!(export.threads.len(), 1);
    assert_eq!(export.threads[0].spec.id, spec.id);
    assert!(!export.threads[0].events.is_empty());
    assert!(export.threads[0].artifacts.iter().any(|artifact| artifact
        .files
        .iter()
        .any(|file| file.path.ends_with("plan.md"))));

    let json = serde_json::to_string_pretty(&export).unwrap();
    assert!(json.contains("\"export_version\": 1"));
    assert!(json.contains("plan.md"));
    assert!(!json.contains("must-not-export"));
    assert!(!json.contains("must-not-export-either"));
}

/// Thread 的执行位置：Scratch 模式下每个 Thread 一个目录。
fn workspace(runtime: &Runtime, spec: &else_protocol::thread::ThreadSpec) -> std::path::PathBuf {
    runtime.thread_workspace(spec).unwrap()
}
