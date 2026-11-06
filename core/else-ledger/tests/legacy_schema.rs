//! 0.1.x 的账本列名是 `intent_*`。改名不是重写历史：
//! 老库必须能原地打开、换列名、重放，事件内容一条不动。

use else_ledger::Ledger;
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{ScopeId, ThreadId};
use else_protocol::thread::{AttentionPolicy, Authority, ThreadSpec};
use rusqlite::{params, Connection};

/// 0.1.x 真实写过的 schema。
const LEGACY_SCHEMA: &str = "
CREATE TABLE events (
    seq              INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id         TEXT NOT NULL UNIQUE,
    protocol_version TEXT NOT NULL,
    intent_id        TEXT NOT NULL,
    scope_id         TEXT NOT NULL,
    generation       INTEGER NOT NULL,
    intent_revision  INTEGER NOT NULL,
    actor            TEXT NOT NULL,
    event_type       TEXT NOT NULL,
    occurred_at      TEXT NOT NULL,
    payload          TEXT NOT NULL
);
CREATE INDEX idx_events_intent ON events(intent_id, seq);
";

fn spec(thread_id: &ThreadId) -> ThreadSpec {
    ThreadSpec {
        id: thread_id.clone(),
        revision: 1,
        outcome: "验证老账本能读".into(),
        done_when: vec![],
        constraints: vec![],
        authority: Authority {
            max_autonomous_effect: SideEffectKind::ReadOnly,
            forbidden: vec![],
        },
        attention_policy: AttentionPolicy {
            allow_interrupt: true,
            batch_interval_minutes: 30,
        },
        opaque_harness_policy: else_protocol::thread::OpaqueHarnessPolicy::default(),
        deadline: None,
        assumptions: vec![],
        project: None,
        origin: None,
        connector_policy: Default::default(),
    }
}

/// 按 0.1.x 的写法序列化一条事件：`intent_id`、`intent_revision`、`intent_created`。
fn legacy_event_json(thread_id: &ThreadId, scope_id: &ScopeId) -> String {
    let mut value = serde_json::to_value(EventEnvelope::new(
        thread_id.clone(),
        scope_id.clone(),
        1,
        1,
        Actor::User,
        EventPayload::ThreadCreated {
            spec: spec(thread_id),
        },
    ))
    .unwrap();
    let object = value.as_object_mut().unwrap();
    let raw_thread_id = object.remove("thread_id").unwrap();
    object.insert("intent_id".into(), raw_thread_id);
    let raw_revision = object.remove("thread_revision").unwrap();
    object.insert("intent_revision".into(), raw_revision);
    object.get_mut("payload").unwrap()["type"] = "intent_created".into();
    value.to_string()
}

#[test]
fn legacy_ledger_migrates_columns_and_replays_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("else.db");
    let thread_id = ThreadId::new();
    let scope_id = ScopeId::new();
    let payload = legacy_event_json(&thread_id, &scope_id);

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(LEGACY_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO events\n             (event_id, protocol_version, intent_id, scope_id, generation,\n              intent_revision, actor, event_type, occurred_at, payload)\n             VALUES ('legacy-event', '0.1.0', ?1, ?2, 1, 1, 'user', 'intent_created',\n                     '2026-10-31T00:00:00+00:00', ?3)",
            params![thread_id.to_string(), scope_id.to_string(), payload],
        )
        .unwrap();
    }

    // 打开即迁移，读取即重放。
    let ledger = Ledger::open(&path).unwrap();
    let events = ledger.read_thread(&thread_id).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].envelope.scope_id, scope_id);
    assert_eq!(events[0].envelope.thread_revision, 1);
    assert!(matches!(
        events[0].envelope.payload,
        EventPayload::ThreadCreated { .. }
    ));

    // 再开一次：迁移是幂等的，旧列名不再出现。
    drop(ledger);
    let reopened = Ledger::open(&path).unwrap();
    assert_eq!(reopened.read_thread(&thread_id).unwrap().len(), 1);
    let columns: Vec<String> = {
        let conn = Connection::open(&path).unwrap();
        let mut statement = conn.prepare("PRAGMA table_info(events)").unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert!(columns.iter().any(|name| name == "thread_id"));
    assert!(!columns.iter().any(|name| name == "intent_id"));
}
