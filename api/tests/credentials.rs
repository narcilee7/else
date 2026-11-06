use chrono::{Duration, Utc};
use else_api::{ApiError, Runtime};
use else_connector::{AdapterRegistry, SignalInterpreter};
use else_protocol::connector::{ConnectorCredential, ConnectorHealth};
use else_secrets::{CredentialBackend, CredentialStore};

fn runtime_with_file_credentials(dir: &std::path::Path) -> Runtime {
    let store = CredentialStore::with_backend(dir, CredentialBackend::PrivateFile).unwrap();
    Runtime::open_with_connector_runtime_and_store(
        dir,
        AdapterRegistry::builtin(),
        SignalInterpreter::builtin(),
        store,
    )
    .unwrap()
}

fn notion_credential(secret: &str) -> ConnectorCredential {
    ConnectorCredential {
        access_token: secret.into(),
        refresh_token: None,
        token_type: "bearer".into(),
        scopes: vec!["pages:read".into(), "databases:read".into()],
        expires_at: None,
        refresh: None,
    }
}

#[test]
fn connector_credentials_never_enter_the_ledger_and_can_be_revoked() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = runtime_with_file_credentials(dir.path());
    let spec = runtime.create_thread("整理 Notion 项目资料", None).unwrap();
    let source = dir.path().join("notion.json");
    std::fs::write(&source, r#"{"results":[]}"#).unwrap();
    let secret = "notion-super-secret-token";
    let connector = runtime
        .register_connector_adapter_with_credential(
            &spec.id,
            "notion",
            source.to_str().unwrap(),
            "notion.api",
            Some(notion_credential(secret)),
            300,
        )
        .unwrap();
    assert!(connector.credential.is_some());
    runtime.sync_connector(&spec.id, &connector.id).unwrap();

    let events = runtime
        .events(&spec.id)
        .unwrap()
        .into_iter()
        .map(|record| serde_json::to_string(&record.envelope.payload).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!events.contains(secret));
    drop(runtime);
    let database = std::fs::read(dir.path().join("else.db")).unwrap();
    assert!(!database
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));

    let runtime = runtime_with_file_credentials(dir.path());
    let health = runtime.connector_health(&spec.id, &connector.id).unwrap();
    assert!(health.credential_configured);
    assert_eq!(
        health.credential_scopes,
        vec!["pages:read".to_string(), "databases:read".to_string()]
    );
    let revoked = runtime
        .revoke_connector_credential(&spec.id, &connector.id)
        .unwrap();
    assert!(revoked.revoked_at.is_some());
    assert!(matches!(
        runtime.sync_connector(&spec.id, &connector.id),
        Err(ApiError::Unauthorized(_))
    ));
}

#[test]
fn credential_scope_check_happens_before_storage_or_registration() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = runtime_with_file_credentials(dir.path());
    let spec = runtime.create_thread("检查凭据范围", None).unwrap();
    let source = dir.path().join("notion.json");
    std::fs::write(&source, r#"{"results":[]}"#).unwrap();
    let mut credential = notion_credential("scope-secret");
    credential.scopes = vec!["pages:read".into()];
    let result = runtime.register_connector_adapter_with_credential(
        &spec.id,
        "notion",
        source.to_str().unwrap(),
        "notion.api",
        Some(credential),
        300,
    );
    assert!(matches!(result, Err(ApiError::Conflict(_))));
    assert!(runtime.projection(&spec.id).unwrap().connectors.is_empty());
    let credential_dir = dir.path().join("credentials");
    assert_eq!(
        std::fs::read_dir(credential_dir).unwrap().count(),
        0,
        "failed scope validation must not persist the secret"
    );
}

#[test]
fn scheduler_syncs_due_connectors_and_degrades_after_repeated_failures() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = runtime_with_file_credentials(dir.path());
    let spec = runtime.create_thread("持续同步日历", None).unwrap();
    let source = dir.path().join("missing.ics");
    std::fs::write(&source, "BEGIN:VCALENDAR\nEND:VCALENDAR\n").unwrap();
    let connector = runtime
        .register_calendar_connector(&spec.id, "calendar", source.to_str().unwrap())
        .unwrap();
    std::fs::remove_file(&source).unwrap();

    let wake = runtime
        .sync_due_connectors(Utc::now() + Duration::seconds(1))
        .unwrap();
    assert!(wake > Utc::now());
    let health = runtime.connector_health(&spec.id, &connector.id).unwrap();
    assert_eq!(health.consecutive_failures, 1);
    assert_eq!(health.health, ConnectorHealth::Degraded);

    runtime.sync_connector(&spec.id, &connector.id).unwrap_err();
    runtime.sync_connector(&spec.id, &connector.id).unwrap_err();
    let health = runtime.connector_health(&spec.id, &connector.id).unwrap();
    assert_eq!(health.consecutive_failures, 3);
    assert_eq!(health.health, ConnectorHealth::Error);
}
