use chrono::{Duration, Utc};
use else_api::Runtime;

#[test]
fn connector_repair_resets_cursor_and_resyncs_through_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_thread("Connector repair", None).unwrap();
    let start = Utc::now() + Duration::hours(2);
    let end = start + Duration::hours(1);
    let source = dir.path().join("calendar.ics");
    std::fs::write(
        &source,
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:repair-event\r\nSUMMARY:Repair test\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            start.format("%Y%m%dT%H%M%SZ"),
            end.format("%Y%m%dT%H%M%SZ")
        ),
    )
    .unwrap();
    let connector = runtime
        .register_calendar_connector(&spec.id, "work", source.to_str().unwrap())
        .unwrap();
    runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert!(!runtime
        .projection(&spec.id)
        .unwrap()
        .connector_cursors
        .is_empty());

    let report = runtime.repair_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(report.connector_id, connector.id);
    assert!(report.reset_cursors >= 1);
    assert!(report.sync.is_some());
    assert!(report.error.is_none());
    assert!(runtime
        .projection(&spec.id)
        .unwrap()
        .connector_sync_status
        .get(&connector.id)
        .is_some_and(|status| status.consecutive_failures == 0));
}
