use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use else_api::{ApiError, Runtime};
use else_connector::{AdapterRegistry, SignalInterpreter};
use else_protocol::attention::DecisionChoice;
use else_protocol::connector::{ConnectorActionStatus, ConnectorCredential, ConnectorManifest};
use else_secrets::{CredentialBackend, CredentialStore};
use serde_json::{json, Value};

fn credential() -> ConnectorCredential {
    ConnectorCredential {
        access_token: "google-access-token".into(),
        refresh_token: None,
        token_type: "bearer".into(),
        scopes: vec!["calendar.events".into()],
        expires_at: None,
        refresh: None,
    }
}

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

#[test]
fn google_calendar_update_verify_and_compensate_cross_http_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let original = json!({
        "id": "evt-1",
        "summary": "Old title",
        "location": "Room A",
        "status": "confirmed",
        "start": {"dateTime": "2026-11-01T09:00:00Z"},
        "end": {"dateTime": "2026-11-01T10:00:00Z"},
        "updated": "2026-10-31T00:00:00Z"
    });
    let current = Arc::new(Mutex::new(original.clone()));
    let server_current = current.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 2048];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(header_end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header_end = header_end + 4;
                    let headers = String::from_utf8_lossy(&buffer[..header_end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("Content-Length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if buffer.len() >= header_end + length {
                        break;
                    }
                }
            }
            let header_end = buffer.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let headers = String::from_utf8_lossy(&buffer[..header_end]);
            assert!(
                headers
                    .lines()
                    .any(|line| line
                        .eq_ignore_ascii_case("Authorization: Bearer google-access-token"))
            );
            let method = headers.split_whitespace().next().unwrap();
            let body = &buffer[header_end..];
            let response = if method == "PATCH" {
                let patch: Value = serde_json::from_slice(body).unwrap();
                let mut current = server_current.lock().unwrap();
                let object = current.as_object_mut().unwrap();
                for (key, value) in patch.as_object().unwrap() {
                    object.insert(key.clone(), value.clone());
                }
                current.clone()
            } else {
                server_current.lock().unwrap().clone()
            };
            let body = response.to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let runtime = runtime_with_file_credentials(dir.path());
    let spec = runtime.create_thread("更新日历事件", None).unwrap();
    let source = format!("http://{address}/events");
    let connector = runtime
        .register_connector_adapter_with_credential(
            &spec.id,
            "calendar",
            &source,
            "calendar.google",
            Some(credential()),
            300,
        )
        .unwrap();
    let update_capability = runtime
        .capabilities(&spec.id)
        .unwrap()
        .into_iter()
        .find(|capability| {
            matches!(
                &capability.adapter,
                else_protocol::capability::CapabilityAdapter::ExternalAction(binding)
                    if binding.connector_id == connector.id
                        && binding.operation == "calendar.event.update"
            )
        })
        .unwrap();
    assert_eq!(
        update_capability.manifest.input_schema["target_label"],
        "Event ID"
    );
    assert!(update_capability.manifest.input_schema["fields"]
        .as_array()
        .is_some_and(|fields| !fields.is_empty()));
    let action = runtime
        .propose_connector_action(
            &spec.id,
            &connector.id,
            "calendar.event.update",
            "evt-1",
            json!({
                "changes": {
                    "summary": "New title",
                    "location": "Room B"
                }
            }),
            Some("calendar-event-evt-1-update".into()),
        )
        .unwrap();
    let preview = runtime
        .preview_connector_action(
            &spec.id,
            &connector.id,
            "calendar.event.update",
            "evt-1",
            &json!({"changes":{"summary":"New title","location":"Room B"}}),
        )
        .unwrap();
    assert_eq!(preview.current["summary"], "Old title");
    assert_eq!(preview.proposed["location"], "Room B");

    assert!(matches!(
        runtime.execute_connector_action(&spec.id, &action.id),
        Err(ApiError::Gateway(_))
    ));
    let pending = runtime
        .connector_actions(&spec.id)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.id == action.id)
        .unwrap();
    runtime
        .decide(
            &spec.id,
            pending.attention_id.as_ref().unwrap(),
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    let applied = runtime
        .execute_connector_action(&spec.id, &action.id)
        .unwrap();
    assert_eq!(applied.status, ConnectorActionStatus::Applied);
    assert_eq!(current.lock().unwrap()["summary"], "New title");

    let verified = runtime
        .verify_connector_action(&spec.id, &action.id)
        .unwrap();
    assert_eq!(verified.status, ConnectorActionStatus::Applied);
    assert!(matches!(
        runtime.compensate_connector_action(&spec.id, &action.id),
        Err(ApiError::Gateway(_))
    ));
    let pending = runtime
        .connector_actions(&spec.id)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.id == action.id)
        .unwrap();
    runtime
        .decide(
            &spec.id,
            pending.attention_id.as_ref().unwrap(),
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    let compensated = runtime
        .compensate_connector_action(&spec.id, &action.id)
        .unwrap();
    assert_eq!(
        compensated.status,
        ConnectorActionStatus::Compensated,
        "{compensated:?}"
    );
    assert_eq!(*current.lock().unwrap(), original);
    server.join().unwrap();
}

#[test]
fn google_calendar_manifest_declares_gateway_managed_update() {
    let manifest: ConnectorManifest = AdapterRegistry::builtin()
        .get("calendar.google")
        .unwrap()
        .manifest();
    assert!(manifest
        .write_capabilities
        .contains(&"calendar.event.update".into()));
}
