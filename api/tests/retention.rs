use chrono::{Duration, Utc};
use else_api::{Runtime, RuntimeSettings};

#[test]
fn runtime_settings_persist_and_default_retention_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    assert_eq!(
        runtime.runtime_settings().unwrap(),
        RuntimeSettings::default()
    );
    runtime
        .update_runtime_settings(RuntimeSettings {
            retention_days: Some(30),
        })
        .unwrap();
    drop(runtime);

    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(
        reopened.runtime_settings().unwrap().retention_days,
        Some(30)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(dir.path().join("runtime-settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(permissions, 0o600);
    }
}

#[test]
fn retention_deletes_only_terminal_threads_past_cutoff() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    runtime
        .update_runtime_settings(RuntimeSettings {
            retention_days: Some(7),
        })
        .unwrap();
    let terminal = runtime.create_thread("终态测试", None).unwrap();
    runtime.abort(&terminal.id, "retention test").unwrap();
    let running = runtime.create_thread("运行中测试", None).unwrap();

    let not_due = runtime.enforce_retention(Utc::now()).unwrap();
    assert!(not_due.is_empty());

    let due = runtime
        .enforce_retention(Utc::now() + Duration::days(8))
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].thread_id, terminal.id);
    assert!(runtime.projection(&terminal.id).unwrap().spec.is_none());
    assert_eq!(
        runtime.projection(&running.id).unwrap().spec.unwrap().id,
        running.id
    );
}
