//! Cross-Provider Project Correlation：Linear / Notion / Calendar 聚合。

use chrono::{Duration, Utc};
use else_api::Runtime;
use else_protocol::connector::CorrelationKind;

fn calendar_ics() -> String {
    let start = Utc::now() + Duration::hours(4);
    let end = start + Duration::hours(1);
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:project-calendar\r\nSUMMARY:发布评审\r\nCATEGORIES:Launch\r\nATTENDEE:mailto:alice\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        start.format("%Y%m%dT%H%M%SZ"),
        end.format("%Y%m%dT%H%M%SZ")
    )
}

fn linear_json() -> serde_json::Value {
    serde_json::json!({
        "issues": [{
            "id": "project-linear",
            "identifier": "ENG-401",
            "title": "完成发布清单",
            "state": {"name": "In Progress", "type": "started"},
            "updatedAt": Utc::now().to_rfc3339(),
            "project": {"name": "Launch"},
            "milestone": {"name": "Launch GA"},
            "assignee": {"name": "Alice"},
            "labels": [],
            "blockedBy": []
        }]
    })
}

fn notion_json() -> serde_json::Value {
    serde_json::json!({
        "results": [{
            "object": "page",
            "id": "project-notion",
            "last_edited_time": Utc::now().to_rfc3339(),
            "archived": false,
            "url": "https://notion.so/project-notion",
            "parent": {"type": "database_id", "database_id": "project-db"},
            "properties": {
                "Name": {
                    "type": "title",
                    "title": [{"plain_text": "发布说明"}]
                },
                "Project": {
                    "type": "select",
                    "select": {"name": "Launch"}
                },
                "Status": {
                    "type": "status",
                    "status": {"name": "In Progress", "group": "in_progress"}
                },
                "Customer": {
                    "type": "select",
                    "select": {"name": "Acme"}
                },
                "Contact": {
                    "type": "select",
                    "select": {"name": "Alice"}
                },
                "Thread": {
                    "type": "rich_text",
                    "rich_text": [{"plain_text": "launch-review"}]
                },
                "Milestone": {
                    "type": "select",
                    "select": {"name": "Launch GA"}
                }
            }
        }]
    })
}

#[test]
fn project_world_correlates_connectors_and_intents_by_project_key() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let first = runtime.create_intent("推进 Launch", None).unwrap();
    let linear_path = dir.path().join("linear.json");
    let notion_path = dir.path().join("notion.json");
    let calendar_path = dir.path().join("calendar.ics");
    std::fs::write(&linear_path, linear_json().to_string()).unwrap();
    std::fs::write(&notion_path, notion_json().to_string()).unwrap();
    std::fs::write(&calendar_path, calendar_ics()).unwrap();

    for (adapter, source) in [
        ("linear", linear_path.to_str().unwrap()),
        ("notion", notion_path.to_str().unwrap()),
        ("calendar", calendar_path.to_str().unwrap()),
    ] {
        let connector = runtime
            .register_connector_adapter(&first.id, adapter, source, adapter)
            .unwrap();
        runtime.sync_connector(&first.id, &connector.id).unwrap();
    }

    let projects = runtime.project_worlds(Some(&first.id), false).unwrap();
    assert_eq!(projects.len(), 1);
    let launch = &projects[0];
    assert_eq!(launch.key, "project:launch");
    assert_eq!(launch.title, "Launch");
    assert_eq!(launch.items.len(), 3);
    assert_eq!(launch.connector_ids.len(), 3);
    assert_eq!(launch.intent_ids, vec![first.id.clone()]);

    let second = runtime.create_intent("准备 Launch 评审", None).unwrap();
    let calendar = runtime
        .register_connector_adapter(
            &second.id,
            "calendar",
            calendar_path.to_str().unwrap(),
            "calendar",
        )
        .unwrap();
    runtime.sync_connector(&second.id, &calendar.id).unwrap();

    let scoped = runtime.project_worlds(Some(&first.id), false).unwrap();
    assert_eq!(scoped[0].items.len(), 3);
    let global = runtime.project_worlds(None, false).unwrap();
    let launch = global
        .iter()
        .find(|project| project.key == "project:launch")
        .unwrap();
    assert_eq!(launch.items.len(), 4);
    assert_eq!(launch.intent_ids.len(), 2);
    assert!(launch.connector_ids.len() >= 4);

    let identities = runtime.identity_worlds(None, false).unwrap();
    let alice = identities
        .iter()
        .find(|identity| {
            identity.kind == CorrelationKind::Contact && identity.key == "contact:alice"
        })
        .unwrap();
    assert_eq!(alice.items.len(), 4);
    assert_eq!(alice.intent_ids.len(), 2);
    assert!(identities.iter().any(|identity| {
        identity.kind == CorrelationKind::Customer
            && identity.key == "customer:acme"
            && identity.items.len() == 1
    }));
    assert!(identities.iter().any(|identity| {
        identity.kind == CorrelationKind::Thread && identity.key == "thread:eng-401"
    }));
    assert!(identities.iter().any(|identity| {
        identity.kind == CorrelationKind::Milestone && identity.key == "milestone:launch-ga"
    }));
}
