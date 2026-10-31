//! 只读连接器注册与事实同步。

use chrono::{Duration, Utc};
use else_protocol::connector::{ConnectorKind, ConnectorRegistration, ConnectorSyncResult};
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{ConnectorId, IntentId};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    pub fn register_calendar_connector(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
    ) -> Result<ConnectorRegistration> {
        if !source.starts_with("https://") && !std::path::Path::new(source).is_file() {
            return Err(ApiError::NotFound(format!(
                "calendar source is not an existing file or HTTPS URL: {source}"
            )));
        }
        let connector = ConnectorRegistration {
            id: ConnectorId::new(),
            name: name.into(),
            kind: ConnectorKind::CalendarReadOnly,
            source: source.into(),
            enabled: true,
        };
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::User,
            EventPayload::ConnectorRegistered {
                connector: connector.clone(),
            },
        ))?;
        Ok(connector)
    }

    pub fn sync_connector(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorSyncResult> {
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        if !connector.enabled {
            return Err(ApiError::Conflict(format!(
                "connector {connector_id} is disabled"
            )));
        }
        let now = Utc::now();
        let facts = else_connector::read_calendar(
            connector.id.clone(),
            &connector.source,
            now - Duration::days(1),
            now + Duration::days(14),
        )
        .map_err(|error| ApiError::NotFound(error.to_string()))?;
        let mut result = ConnectorSyncResult {
            observed: facts.len() as u64,
            ..ConnectorSyncResult::default()
        };
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        for fact in facts {
            if projection.external_fact_keys.contains(&fact.fact_id) {
                result.skipped += 1;
                continue;
            }
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                root.clone(),
                generation,
                revision,
                Actor::External,
                EventPayload::ExternalFactIngested { fact },
            ))?;
            result.inserted += 1;
        }
        Ok(result)
    }
}
