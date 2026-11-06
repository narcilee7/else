//! Personal World：Signal Interpretation 聚合、Overdue 与跨 Thread Brief。

use chrono::{Duration, Utc};
use else_api::Runtime;
use else_protocol::world::{WorldItemKind, WorldItemResolutionKind, WorldItemState};

fn linear_work_payload() -> serde_json::Value {
    let now = Utc::now();
    serde_json::json!({
        "issues": [
            {
                "id": "world-commitment",
                "identifier": "ENG-301",
                "title": "交付客户方案",
                "state": {"name": "In Progress", "type": "started"},
                "updatedAt": now.to_rfc3339(),
                "dueDate": (now + Duration::days(2)).date_naive().format("%Y-%m-%d").to_string(),
                "labels": ["commitment"],
                "blockedBy": []
            },
            {
                "id": "world-waiting",
                "identifier": "ENG-302",
                "title": "等待法务反馈",
                "state": {"name": "Blocked", "type": "started"},
                "updatedAt": now.to_rfc3339(),
                "labels": ["waiting_external"],
                "blockedBy": ["legal"]
            },
            {
                "id": "world-overdue",
                "identifier": "ENG-303",
                "title": "补交发布材料",
                "state": {"name": "Todo", "type": "unstarted"},
                "updatedAt": now.to_rfc3339(),
                "dueDate": (now - Duration::days(1)).date_naive().format("%Y-%m-%d").to_string(),
                "labels": [],
                "blockedBy": []
            },
            {
                "id": "world-risk",
                "identifier": "ENG-304",
                "title": "发布窗口存在风险",
                "state": {"name": "Todo", "type": "unstarted"},
                "updatedAt": now.to_rfc3339(),
                "labels": ["at_risk"],
                "blockedBy": []
            }
        ]
    })
}

#[test]
fn personal_world_aggregates_commitments_waiting_overdue_and_brief() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_thread("推进发布承诺", None).unwrap();
    let source = dir.path().join("linear.json");
    std::fs::write(&source, linear_work_payload().to_string()).unwrap();
    let connector = runtime
        .register_connector_adapter(&spec.id, "engineering", source.to_str().unwrap(), "linear")
        .unwrap();
    runtime.sync_connector(&spec.id, &connector.id).unwrap();

    let items = runtime.world_items(&spec.id, false).unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(
        items
            .iter()
            .filter(|item| item.kind == WorldItemKind::Commitment)
            .count(),
        1
    );
    assert_eq!(
        items
            .iter()
            .filter(|item| item.state == WorldItemState::WaitingExternal)
            .count(),
        1
    );
    assert_eq!(
        items
            .iter()
            .filter(|item| item.state == WorldItemState::Overdue)
            .count(),
        1
    );
    assert_eq!(
        items
            .iter()
            .filter(|item| item.kind == WorldItemKind::Risk)
            .count(),
        1
    );
    let overdue = items
        .iter()
        .find(|item| item.state == WorldItemState::Overdue)
        .unwrap();
    assert!(overdue.priority >= 94);
    assert!(overdue.priority_reason.contains("已逾期"));
    let waiting = items
        .iter()
        .find(|item| item.kind == WorldItemKind::WaitingExternal)
        .unwrap();
    assert!(waiting.priority_reason.contains("外部对象阻塞"));
    assert!(items
        .windows(2)
        .all(|pair| pair[0].priority >= pair[1].priority));

    let brief = runtime.personal_brief().unwrap();
    assert_eq!(brief.open_commitments.len(), 1);
    assert_eq!(brief.waiting_external.len(), 1);
    assert_eq!(brief.overdue.len(), 1);
    assert_eq!(brief.risks.len(), 1);
    assert!(brief.action_items_due_soon.is_empty());

    let commitment_id = items
        .iter()
        .find(|item| item.kind == WorldItemKind::Commitment)
        .unwrap()
        .id
        .clone();
    let resolved_commitment = runtime
        .resolve_world_item(
            &spec.id,
            &commitment_id,
            WorldItemResolutionKind::Completed,
            Some("已向客户交付".into()),
        )
        .unwrap();
    assert_eq!(resolved_commitment.state, WorldItemState::Completed);
    assert_eq!(runtime.world_items(&spec.id, false).unwrap().len(), 3);
    assert!(runtime
        .personal_brief()
        .unwrap()
        .open_commitments
        .is_empty());

    let completed = serde_json::json!({
        "id": "world-waiting",
        "identifier": "ENG-302",
        "title": "等待法务反馈",
        "state": {"name": "Done", "type": "completed"},
        "updatedAt": (Utc::now() + Duration::seconds(1)).to_rfc3339(),
        "labels": ["waiting_external"],
        "blockedBy": ["legal"]
    });
    runtime
        .ingest_connector_webhook(
            &spec.id,
            &connector.id,
            connector.webhook_session.as_deref().unwrap(),
            serde_json::json!({
                "action": "update",
                "type": "Issue",
                "data": completed
            }),
        )
        .unwrap();
    assert_eq!(runtime.world_items(&spec.id, false).unwrap().len(), 2);
    let resolved = runtime.world_items(&spec.id, true).unwrap();
    let completed = resolved
        .iter()
        .find(|item| item.kind == WorldItemKind::WaitingExternal)
        .unwrap();
    assert_eq!(completed.state, WorldItemState::Completed);
    assert!(completed.signal_ids.len() >= 2);
    assert_eq!(
        resolved
            .iter()
            .find(|item| item.id == commitment_id)
            .unwrap()
            .state,
        WorldItemState::Completed
    );
    assert!(runtime
        .personal_brief()
        .unwrap()
        .waiting_external
        .is_empty());
}
