//! 外部世界连接器：首批默认只读，事实必须带来源与有效期。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::ConnectorId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    CalendarReadOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorRegistration {
    pub id: ConnectorId,
    pub name: String,
    pub kind: ConnectorKind,
    /// 本地文件路径或 HTTPS URL。
    pub source: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalFact {
    pub connector_id: ConnectorId,
    pub fact_id: String,
    pub source: String,
    pub summary: String,
    pub occurred_at: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorSyncResult {
    pub observed: u64,
    pub inserted: u64,
    pub skipped: u64,
}
