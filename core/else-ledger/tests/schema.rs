//! 破坏性变更的边界：Thread 改名不迁移旧库，但必须在打开时
//! 明确报错，而不是等到第一次写入才报 `no such column`。

use else_ledger::{Ledger, LedgerError};
use rusqlite::Connection;

const PRE_THREAD_SCHEMA: &str = "
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
";

#[test]
fn pre_thread_schema_is_rejected_with_an_explicit_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("else.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(PRE_THREAD_SCHEMA).unwrap();
    }

    match Ledger::open(&path) {
        Err(LedgerError::LegacySchema) => {}
        Err(other) => panic!("expected LegacySchema, got {other}"),
        Ok(_) => panic!("expected LegacySchema, got a working ledger"),
    }
}

#[test]
fn current_schema_opens_normally() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("else.db")).unwrap();
    assert!(ledger.read_all().unwrap().is_empty());
}
