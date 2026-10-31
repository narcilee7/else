use else_api::Runtime;

#[test]
fn data_export_contains_ledger_and_artifacts_without_private_state() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("导出测试", None).unwrap();
    runtime.drive(&spec.id, None, 16).unwrap();

    std::fs::create_dir_all(dir.path().join("workspace/.codex")).unwrap();
    std::fs::create_dir_all(dir.path().join("harness-private")).unwrap();
    std::fs::write(
        dir.path().join("workspace/.codex/provider-token"),
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
    assert_eq!(export.intents.len(), 1);
    assert_eq!(export.intents[0].spec.id, spec.id);
    assert!(!export.intents[0].events.is_empty());
    assert!(export.intents[0].artifacts.iter().any(|artifact| artifact
        .files
        .iter()
        .any(|file| file.path.ends_with("plan.md"))));

    let json = serde_json::to_string_pretty(&export).unwrap();
    assert!(json.contains("\"export_version\": 1"));
    assert!(json.contains("plan.md"));
    assert!(!json.contains("must-not-export"));
    assert!(!json.contains("must-not-export-either"));
}
