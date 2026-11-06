use else_api::Runtime;
use else_connector::{AdapterRegistry, SignalInterpreter};
use else_protocol::connector::ConnectorCredential;
use else_secrets::{CredentialBackend, CredentialStore};

#[test]
fn explicit_thread_delete_removes_events_credentials_and_scope_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let credential_store =
        CredentialStore::with_backend(dir.path(), CredentialBackend::PrivateFile).unwrap();
    let runtime = Runtime::open_with_connector_runtime_and_store(
        dir.path(),
        AdapterRegistry::builtin(),
        SignalInterpreter::builtin(),
        credential_store,
    )
    .unwrap();
    let spec = runtime
        .create_thread("待删除 Thread", None, Default::default())
        .unwrap();
    let connector = runtime
        .register_connector_adapter_with_credential(
            &spec.id,
            "linear",
            "https://api.linear.app/graphql",
            "linear.graphql",
            Some(ConnectorCredential {
                access_token: "delete-test-token".into(),
                refresh_token: None,
                token_type: "bearer".into(),
                scopes: vec!["issues:read".into()],
                expires_at: None,
                refresh: None,
            }),
            300,
        )
        .unwrap();
    let credential_reference = connector.credential.unwrap().reference;
    let projection = runtime.projection(&spec.id).unwrap();
    let root_scope = projection.root_scope_id.unwrap();
    let scope_path = dir
        .path()
        .join("workspace/scopes")
        .join(spec.id.to_string())
        .join(root_scope.to_string());
    let branch_path = dir.path().join("branches").join(root_scope.to_string());
    std::fs::create_dir_all(&scope_path).unwrap();
    std::fs::create_dir_all(&branch_path).unwrap();
    std::fs::write(scope_path.join("artifact.txt"), "delete me").unwrap();

    let report = runtime.delete_thread(&spec.id).unwrap();
    assert!(report.deleted_events > 0);
    assert_eq!(report.removed_credentials, 1);
    assert!(report.removed_workspace_paths >= 2);
    assert!(report.cleanup_errors.is_empty());
    assert!(!scope_path.exists());
    assert!(!branch_path.exists());
    assert!(runtime.events(&spec.id).unwrap().is_empty());
    assert!(runtime.projection(&spec.id).unwrap().spec.is_none());
    assert!(runtime.list_threads().unwrap().is_empty());

    let reopened_store =
        CredentialStore::with_backend(dir.path(), CredentialBackend::PrivateFile).unwrap();
    assert!(reopened_store.get(&credential_reference).is_err());
}
