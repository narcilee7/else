use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use else_connector::AdapterRegistry;
use else_protocol::connector::{ConnectorAction, ConnectorActionStatus, ConnectorCredential};
use else_protocol::effect::{EffectConclusion, ToolStatus};
use else_protocol::ids::{ConnectorActionId, ConnectorId};
use serde_json::{json, Value};

fn credential(scope: &str) -> ConnectorCredential {
    ConnectorCredential {
        access_token: "provider-test-token".into(),
        refresh_token: None,
        token_type: "bearer".into(),
        scopes: vec![scope.into()],
        expires_at: None,
        refresh: None,
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> (String, String, Value) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(header_end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if buffer.len() >= header_end + 4 + length {
            let method = headers.split_whitespace().next().unwrap().to_string();
            let path = headers.split_whitespace().nth(1).unwrap().to_string();
            let body = serde_json::from_slice(&buffer[header_end + 4..header_end + 4 + length])
                .unwrap_or(Value::Null);
            return (method, path, body);
        }
    }
    panic!("incomplete request")
}

fn respond(stream: &mut std::net::TcpStream, body: &Value) {
    let body = body.to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}

#[test]
fn linear_update_verify_and_compensate_cross_http_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(json!({
        "id": "issue-1",
        "title": "Old title",
        "description": "Old body",
        "dueDate": "2026-11-02",
        "priority": 2,
        "assigneeId": "user-1",
        "projectId": "project-1",
        "stateId": "state-1",
        "milestoneId": null
    })));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().unwrap();
            let (method, _path, body) = read_request(&mut stream);
            assert_eq!(method, "POST");
            let query = body["query"].as_str().unwrap();
            let response = if query.contains("mutation UpdateIssue") {
                let input = body.pointer("/variables/input").cloned().unwrap();
                let mut state = server_state.lock().unwrap();
                let object = state.as_object_mut().unwrap();
                for (key, value) in input.as_object().unwrap() {
                    object.insert(key.clone(), value.clone());
                }
                json!({"data":{"issueUpdate":{"success":true,"issue":state.clone()}}})
            } else {
                json!({"data":{"issue":server_state.lock().unwrap().clone()}})
            };
            respond(&mut stream, &response);
        }
    });

    let registry = AdapterRegistry::builtin();
    let adapter = registry.get("linear.graphql").unwrap();
    let update_schema = adapter.action_schema("linear.issue.update").unwrap();
    assert_eq!(update_schema["target_label"], "Issue ID");
    assert!(update_schema["fields"]
        .as_array()
        .is_some_and(|fields| fields.len() >= 8));
    let connector_id = ConnectorId::from("linear-connector");
    let cred = credential("issues:read");
    let source = format!("http://{address}/graphql");
    let preview = adapter
        .preview_action(
            &source,
            "linear.issue.update",
            "issue-1",
            Some(&cred),
            &json!({"changes":{"title":"New title"}}),
        )
        .unwrap();
    assert_eq!(preview["title"], "Old title");
    assert_eq!(preview["priority"], 2);
    let outcome = adapter
        .execute_action(
            &connector_id,
            &source,
            "linear.issue.update",
            Some(&cred),
            &json!({"target":"issue-1","changes":{"title":"New title"}}),
        )
        .unwrap();
    assert_eq!(outcome.status, ToolStatus::Succeeded);
    assert_eq!(outcome.output["after"]["title"], "New title");

    let action = ConnectorAction {
        id: ConnectorActionId::from("linear-action"),
        connector_id,
        capability: "linear.issue.update".into(),
        target: "issue-1".into(),
        input: json!({"changes":{"title":"New title"}}),
        effect: else_protocol::effect::SideEffectKind::Unknown,
        idempotency_key: "linear-action".into(),
        status: ConnectorActionStatus::Applied,
        proposed_at: chrono::Utc::now(),
        attention_id: None,
        tool_call_id: None,
        outcome: Some(outcome),
        last_error: None,
    };
    let verified = adapter
        .verify_action(&source, &action, Some(&cred))
        .unwrap();
    assert_eq!(verified.effect, EffectConclusion::Applied);
    let compensated = adapter
        .compensate_action(&source, &action, Some(&cred))
        .unwrap();
    assert_eq!(compensated.effect, EffectConclusion::Compensated);
    assert_eq!(state.lock().unwrap()["title"], "Old title");
    server.join().unwrap();
}

#[test]
fn notion_page_update_verify_and_compensate_cross_http_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(json!({
        "id": "page-1",
        "properties": {
            "Name": {"type":"title","title":[{"plain_text":"Old title"}]}
        },
        "archived": false,
        "icon": null,
        "cover": null
    })));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().unwrap();
            let (method, path, body) = read_request(&mut stream);
            assert_eq!(path, "/v1/pages/page-1");
            if method == "PATCH" {
                let mut state = server_state.lock().unwrap();
                let object = state.as_object_mut().unwrap();
                for (key, value) in body.as_object().unwrap() {
                    object.insert(key.clone(), value.clone());
                }
            }
            let response = server_state.lock().unwrap().clone();
            respond(&mut stream, &response);
        }
    });

    let registry = AdapterRegistry::builtin();
    let adapter = registry.get("notion.api").unwrap();
    let update_schema = adapter.action_schema("notion.page.update").unwrap();
    assert_eq!(update_schema["target_label"], "Page ID");
    assert!(update_schema["preview_supported"]
        .as_bool()
        .unwrap_or(false));
    let connector_id = ConnectorId::from("notion-connector");
    let cred = credential("pages:read");
    let source = format!("http://{address}/v1/search");
    let changes = json!({
        "properties": {
            "Name": {"type":"title","title":[{"plain_text":"New title"}]}
        }
    });
    let preview = adapter
        .preview_action(
            &source,
            "notion.page.update",
            "page-1",
            Some(&cred),
            &json!({"changes":changes}),
        )
        .unwrap();
    assert_eq!(preview["archived"], false);
    assert_eq!(
        preview["properties"]["Name"]["title"][0]["plain_text"],
        "Old title"
    );
    let outcome = adapter
        .execute_action(
            &connector_id,
            &source,
            "notion.page.update",
            Some(&cred),
            &json!({"target":"page-1","changes":changes}),
        )
        .unwrap();
    assert_eq!(outcome.output["after"]["properties"], changes["properties"]);

    let action = ConnectorAction {
        id: ConnectorActionId::from("notion-action"),
        connector_id,
        capability: "notion.page.update".into(),
        target: "page-1".into(),
        input: json!({"changes":changes}),
        effect: else_protocol::effect::SideEffectKind::Unknown,
        idempotency_key: "notion-action".into(),
        status: ConnectorActionStatus::Applied,
        proposed_at: chrono::Utc::now(),
        attention_id: None,
        tool_call_id: None,
        outcome: Some(outcome),
        last_error: None,
    };
    let verified = adapter
        .verify_action(&source, &action, Some(&cred))
        .unwrap();
    assert_eq!(verified.effect, EffectConclusion::Applied);
    let compensated = adapter
        .compensate_action(&source, &action, Some(&cred))
        .unwrap();
    assert_eq!(compensated.effect, EffectConclusion::Compensated);
    assert_eq!(
        state.lock().unwrap()["properties"]["Name"]["title"][0]["plain_text"],
        "Old title"
    );
    server.join().unwrap();
}

#[test]
fn linear_comment_create_verify_and_delete_cross_http_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let comment = json!({
        "id": "comment-1",
        "body": "Reviewed and approved",
        "issue": {"id": "issue-1"}
    });
    let stored = Arc::new(Mutex::new(Some(comment.clone())));
    let server_state = stored.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let (_, _, body) = read_request(&mut stream);
            let query = body["query"].as_str().unwrap();
            let response = if query.contains("mutation CreateComment") {
                *server_state.lock().unwrap() = Some(comment.clone());
                json!({"data":{"commentCreate":{"success":true,"comment":comment}}})
            } else if query.contains("mutation DeleteComment") {
                *server_state.lock().unwrap() = None;
                json!({"data":{"commentDelete":{"success":true}}})
            } else {
                json!({"data":{"comment":server_state.lock().unwrap().clone()}})
            };
            respond(&mut stream, &response);
        }
    });

    let registry = AdapterRegistry::builtin();
    let adapter = registry.get("linear.graphql").unwrap();
    let connector_id = ConnectorId::from("linear-comment-connector");
    let cred = credential("issues:read");
    let source = format!("http://{address}/graphql");
    let outcome = adapter
        .execute_action(
            &connector_id,
            &source,
            "linear.issue.comment",
            Some(&cred),
            &json!({"target":"issue-1","body":"Reviewed and approved"}),
        )
        .unwrap();
    let action = ConnectorAction {
        id: ConnectorActionId::from("linear-comment-action"),
        connector_id,
        capability: "linear.issue.comment".into(),
        target: "issue-1".into(),
        input: json!({"body":"Reviewed and approved"}),
        effect: else_protocol::effect::SideEffectKind::Unknown,
        idempotency_key: "linear-comment-action".into(),
        status: ConnectorActionStatus::Applied,
        proposed_at: chrono::Utc::now(),
        attention_id: None,
        tool_call_id: None,
        outcome: Some(outcome),
        last_error: None,
    };
    let verified = adapter
        .verify_action(&source, &action, Some(&cred))
        .unwrap();
    assert_eq!(verified.effect, EffectConclusion::Applied);
    let compensated = adapter
        .compensate_action(&source, &action, Some(&cred))
        .unwrap();
    assert_eq!(compensated.effect, EffectConclusion::Compensated);
    assert!(stored.lock().unwrap().is_none());
    server.join().unwrap();
}
