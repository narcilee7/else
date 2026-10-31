//! 只读日历连接器：事实来源、有效期、去重与 Context Snapshot。

use chrono::{Duration, Utc};
use else_api::Runtime;

#[test]
fn calendar_facts_are_read_only_deduplicated_and_reach_context() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("准备明天的会议", None).unwrap();
    let start = Utc::now() + Duration::hours(2);
    let end = start + Duration::hours(1);
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:meeting-1\r\nSUMMARY:产品评审\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        start.format("%Y%m%dT%H%M%SZ"),
        end.format("%Y%m%dT%H%M%SZ")
    );
    let path = dir.path().join("calendar.ics");
    std::fs::write(&path, ics).unwrap();
    let connector = runtime
        .register_calendar_connector(&spec.id, "work", path.to_str().unwrap())
        .unwrap();
    let first = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(first.observed, 1);
    assert_eq!(first.inserted, 1);
    let second = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(second.inserted, 0);
    assert_eq!(second.skipped, 1);

    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.external_events.len(), 1);
    assert_eq!(
        projection.connectors[&connector.id].kind,
        else_protocol::connector::ConnectorKind::CalendarReadOnly
    );
    let snapshot = runtime
        .compile_snapshot(&spec.id, else_protocol::memory::SnapshotAudience::Planner)
        .unwrap();
    assert_eq!(snapshot.external_events.len(), 1);
    assert!(snapshot.external_events[0].summary.contains("产品评审"));

    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(
        reopened.projection(&spec.id).unwrap().external_events.len(),
        1
    );
}
