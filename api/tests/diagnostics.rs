use else_api::{Runtime, RuntimeSettings};

#[test]
fn diagnostics_report_contains_health_and_storage_without_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("诊断测试", Some("else".into()), Default::default())
        .unwrap();
    runtime.drive(&spec.id, None, 16).unwrap();
    runtime
        .update_runtime_settings(RuntimeSettings {
            retention_days: Some(30),
        })
        .unwrap();

    let report = runtime.diagnostics().unwrap();
    assert!(report.healthy);
    assert_eq!(report.database_integrity, "ok");
    assert!(report.event_count > 0);
    assert_eq!(report.thread_count, 1);
    assert_eq!(report.thread_states.get("needs_decision"), Some(&1));
    assert!(!report.scopes.is_empty());
    assert!(report.scopes[0].generation >= 1);
    assert!(report.storage.database_bytes > 0);
    assert_eq!(report.retention_days, Some(30));
    assert!(!report.credential_backend.is_empty());

    let json = serde_json::to_string_pretty(&report).unwrap();
    assert!(!json.contains("access_token"));
    assert!(!json.contains("refresh_token"));
    assert!(!json.contains("daemon-token"));
}
