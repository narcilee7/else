//! 只读连接器、World Signal、Resolver 与 Intent Candidate 的运行时实现。

use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Duration, Utc};
use else_protocol::capability::{
    CapabilityAdapter, CapabilityKind, CapabilityManifest, CapabilityRegistration,
    ExternalActionBinding,
};
use else_protocol::connector::{
    ConnectorAuthMode, ConnectorCredential, ConnectorCredentialMetadata, ConnectorHealthReport,
    ConnectorManifest, ConnectorObservation, ConnectorPolicy, ConnectorRegistration,
    ConnectorSyncResult, ConnectorSyncStatus, ExternalEntityRef, IntentAcceptedBy, IntentCandidate,
    IntentCandidateRecord, IntentCandidateState, IntentOrigin, IntentOriginKind, ObjectKind,
    ObservationChange, SignalInterpretation, SignalMeaning, SignalStatus, SyncCursor, WorldSignal,
    WorldSignalRecord,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{CapabilityId, ConnectorId, IntentId};
use else_protocol::intent::{AttentionPolicy, Authority, Criterion};
use serde_json::Value;

use crate::{ApiError, IntentDraft, Result, Runtime};

impl Runtime {
    pub fn register_calendar_connector(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
    ) -> Result<ConnectorRegistration> {
        self.register_connector_adapter_with_credential(
            intent_id,
            name,
            source,
            "calendar.ics",
            None,
            300,
        )
    }

    pub fn register_linear_connector(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
    ) -> Result<ConnectorRegistration> {
        self.register_connector_adapter_with_credential(
            intent_id,
            name,
            source,
            "linear.graphql",
            None,
            300,
        )
    }

    pub fn register_connector_adapter(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
        adapter_id: &str,
    ) -> Result<ConnectorRegistration> {
        self.register_connector_adapter_with_credential(
            intent_id, name, source, adapter_id, None, 300,
        )
    }

    pub fn register_connector_adapter_with_credential(
        &self,
        intent_id: &IntentId,
        name: &str,
        source: &str,
        adapter_id: &str,
        credential: Option<ConnectorCredential>,
        sync_interval_seconds: u64,
    ) -> Result<ConnectorRegistration> {
        validate_connector_source(source)?;
        if sync_interval_seconds < 15 {
            return Err(ApiError::Conflict(
                "connector sync_interval_seconds must be at least 15".into(),
            ));
        }
        let adapter = self
            .connector_adapters
            .get(adapter_id)
            .ok_or_else(|| ApiError::NotFound(format!("connector adapter {adapter_id}")))?;
        let manifest = adapter.manifest();
        let credential_metadata = if let Some(credential) = credential {
            if manifest.auth_mode == ConnectorAuthMode::None {
                return Err(ApiError::Conflict(format!(
                    "connector adapter {adapter_id} does not accept credentials"
                )));
            }
            credential
                .validate_scopes(&manifest.scopes)
                .map_err(ApiError::Conflict)?;
            let reference = uuid::Uuid::new_v4().to_string();
            self.credential_store.put(&reference, &credential)?;
            Some(ConnectorCredentialMetadata {
                reference,
                auth_mode: manifest.auth_mode,
                scopes: credential.scopes.clone(),
                expires_at: credential.expires_at,
                configured_at: Utc::now(),
                revoked_at: None,
            })
        } else {
            None
        };
        let webhook_session = adapter
            .webhook_stream_id()
            .map(|_| uuid::Uuid::new_v4().to_string());
        let connector = ConnectorRegistration {
            id: ConnectorId::new(),
            name: name.into(),
            adapter: adapter.id().into(),
            source: source.into(),
            enabled: true,
            manifest,
            webhook_session,
            sync_interval_seconds,
            registered_at: Some(Utc::now()),
            credential: credential_metadata,
        };
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        let appended = self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::User,
            EventPayload::ConnectorRegistered {
                connector: connector.clone(),
            },
        ));
        if appended.is_err() {
            if let Some(reference) = connector
                .credential
                .as_ref()
                .map(|credential| credential.reference.as_str())
            {
                let _ = self.credential_store.delete(reference);
            }
        }
        appended?;
        self.register_connector_write_capabilities(intent_id, &connector)?;
        Ok(connector)
    }

    pub fn connector_manifests(&self) -> Vec<ConnectorManifest> {
        self.connector_adapters.manifests()
    }

    pub(crate) fn register_connector_write_capabilities(
        &self,
        intent_id: &IntentId,
        connector: &ConnectorRegistration,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        for operation in &connector.manifest.write_capabilities {
            let id = CapabilityId::from(format!("connector:{}:{operation}", connector.id));
            if projection.capabilities.contains_key(&id) {
                continue;
            }
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                root.clone(),
                generation,
                revision,
                Actor::System,
                EventPayload::CapabilityRegistered {
                    capability: CapabilityRegistration {
                        manifest: CapabilityManifest {
                            id,
                            name: operation.clone(),
                            description: format!(
                                "{} write capability via Tool Gateway",
                                connector.manifest.provider
                            ),
                            kind: CapabilityKind::ExternalAction,
                            // Provider Adapter must return a more precise conclusion;
                            // authorization is conservative until it does.
                            effect: SideEffectKind::Unknown,
                            idempotency_required: true,
                            input_schema: serde_json::json!({"type":"object"}),
                        },
                        adapter: CapabilityAdapter::ExternalAction(ExternalActionBinding {
                            connector_id: connector.id.clone(),
                            operation: operation.clone(),
                        }),
                    },
                },
            ))?;
        }
        Ok(())
    }

    pub fn attach_connector_credential(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        credential: ConnectorCredential,
    ) -> Result<ConnectorCredentialMetadata> {
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        if connector.manifest.auth_mode == ConnectorAuthMode::None {
            return Err(ApiError::Conflict(format!(
                "connector {connector_id} does not accept credentials"
            )));
        }
        credential
            .validate_scopes(&connector.manifest.scopes)
            .map_err(ApiError::Conflict)?;
        let reference = uuid::Uuid::new_v4().to_string();
        self.credential_store.put(&reference, &credential)?;
        let metadata = ConnectorCredentialMetadata {
            reference: reference.clone(),
            auth_mode: connector.manifest.auth_mode,
            scopes: credential.scopes,
            expires_at: credential.expires_at,
            configured_at: Utc::now(),
            revoked_at: None,
        };
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        let appended = self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::User,
            EventPayload::ConnectorCredentialUpdated {
                connector_id: connector_id.clone(),
                credential: metadata.clone(),
            },
        ));
        if let Err(error) = appended {
            let _ = self.credential_store.delete(&reference);
            return Err(error.into());
        }
        if let Some(previous) = connector.credential {
            let _ = self.credential_store.delete(&previous.reference);
        }
        Ok(metadata)
    }

    pub fn revoke_connector_credential(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorCredentialMetadata> {
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        let mut credential = connector
            .credential
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id} credential")))?;
        self.credential_store.delete(&credential.reference)?;
        credential.revoked_at = Some(Utc::now());
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::User,
            EventPayload::ConnectorCredentialUpdated {
                connector_id: connector_id.clone(),
                credential: credential.clone(),
            },
        ))?;
        Ok(credential)
    }

    pub fn connector_health(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorHealthReport> {
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        let fallback_next = connector
            .registered_at
            .unwrap_or_else(|| Utc::now() - Duration::seconds(1));
        let status = projection
            .connector_sync_status
            .get(connector_id)
            .cloned()
            .unwrap_or(ConnectorSyncStatus {
                connector_id: connector_id.clone(),
                last_attempt_at: None,
                last_success_at: None,
                consecutive_failures: 0,
                last_error: None,
                next_sync_at: fallback_next,
            });
        let credential = connector
            .credential
            .as_ref()
            .filter(|credential| credential.revoked_at.is_none());
        let credential_required = connector.manifest.auth_mode != ConnectorAuthMode::None
            && connector.source.starts_with("https://");
        let credential_configured = credential.is_some();
        let mut health = status.health();
        let mut last_error = status.last_error;
        if credential_required && !credential_configured {
            health = else_protocol::connector::ConnectorHealth::Error;
            last_error = Some("missing or revoked connector credential".into());
        }
        let sync_cursor = self
            .connector_adapters
            .get(&connector.adapter)
            .and_then(|adapter| {
                projection
                    .connector_cursors
                    .get(&(connector.id.clone(), adapter.pull_stream_id().into()))
            });
        Ok(ConnectorHealthReport {
            connector_id: connector.id,
            provider: connector.manifest.provider,
            health,
            credential_configured,
            credential_scopes: credential
                .map(|credential| credential.scopes.clone())
                .unwrap_or_default(),
            credential_expires_at: credential.and_then(|credential| credential.expires_at),
            last_attempt_at: status.last_attempt_at,
            last_success_at: status.last_success_at,
            consecutive_failures: status.consecutive_failures,
            last_error,
            next_sync_at: status.next_sync_at,
            backfill_in_progress: sync_cursor.is_some_and(|cursor| cursor.continuation.is_some()),
            backfill_started_at: sync_cursor.and_then(|cursor| cursor.backfill_started_at),
            backfill_completed_at: sync_cursor.and_then(|cursor| cursor.backfill_completed_at),
            pages_synced: sync_cursor.map(|cursor| cursor.pages_synced).unwrap_or(0),
            checked_at: Utc::now(),
        })
    }

    /// 常驻 Runtime 在每次 reconcile 前调用。返回下一次 Connector 调度时间，
    /// 由 supervisor 与 Attention Timer / MCP refresh 合并计算最早唤醒点。
    pub fn sync_due_connectors(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut due = Vec::new();
        if let Ok(intents) = self.list_intents() {
            for spec in intents {
                let Ok(projection) = self.projection(&spec.id) else {
                    continue;
                };
                for connector in projection.connectors.values() {
                    if !connector.enabled {
                        continue;
                    }
                    let next_sync_at = projection
                        .connector_sync_status
                        .get(&connector.id)
                        .map(|status| status.next_sync_at)
                        .or(connector.registered_at)
                        .unwrap_or(now);
                    if next_sync_at <= now {
                        due.push((spec.id.clone(), connector.id.clone()));
                    }
                }
            }
        }
        for (intent_id, connector_id) in due {
            let _ = self.sync_connector(&intent_id, &connector_id);
        }
        self.next_connector_sync_at()
    }

    pub fn next_connector_sync_at(&self) -> Option<DateTime<Utc>> {
        let mut next = None;
        for spec in self.list_intents().ok()? {
            let projection = self.projection(&spec.id).ok()?;
            for connector in projection
                .connectors
                .values()
                .filter(|connector| connector.enabled)
            {
                let due_at = projection
                    .connector_sync_status
                    .get(&connector.id)
                    .map(|status| status.next_sync_at)
                    .or(connector.registered_at);
                if let Some(due_at) = due_at {
                    next = Some(next.map_or(due_at, |current: DateTime<Utc>| current.min(due_at)));
                }
            }
        }
        next
    }

    pub fn sync_connector(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
    ) -> Result<ConnectorSyncResult> {
        let started_at = Utc::now();
        let result = self.sync_connector_inner(intent_id, connector_id, started_at);
        match &result {
            Ok(sync_result) => {
                let _ = self.record_connector_sync_success(
                    intent_id,
                    connector_id,
                    started_at,
                    sync_result.has_more,
                );
            }
            Err(error) => {
                let retry_after_seconds = match error {
                    ApiError::Connector(else_connector::ConnectorError::RateLimited {
                        retry_after_seconds,
                    }) => Some(*retry_after_seconds),
                    _ => None,
                };
                let _ = self.record_connector_sync_failure(
                    intent_id,
                    connector_id,
                    started_at,
                    &error.to_string(),
                    retry_after_seconds,
                );
            }
        }
        result
    }

    pub(crate) fn connector_credential(
        &self,
        connector: &ConnectorRegistration,
    ) -> Result<Option<ConnectorCredential>> {
        let Some(metadata) = &connector.credential else {
            if connector.manifest.auth_mode != ConnectorAuthMode::None
                && connector.source.starts_with("https://")
            {
                return Err(ApiError::Unauthorized(format!(
                    "connector {} requires {} credentials",
                    connector.id,
                    match connector.manifest.auth_mode {
                        ConnectorAuthMode::ApiToken => "API token",
                        ConnectorAuthMode::OAuth => "OAuth",
                        ConnectorAuthMode::None => "no",
                    }
                )));
            }
            return Ok(None);
        };
        if metadata.revoked_at.is_some() {
            return Err(ApiError::Unauthorized(format!(
                "connector {} credential is revoked",
                connector.id
            )));
        }
        self.credential_store
            .resolve(&metadata.reference, &connector.manifest.scopes)
            .map(Some)
            .map_err(ApiError::from)
    }

    fn record_connector_sync_success(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        started_at: DateTime<Utc>,
        has_more: bool,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        let now = Utc::now();
        let status = ConnectorSyncStatus {
            connector_id: connector_id.clone(),
            last_attempt_at: Some(started_at),
            last_success_at: Some(now),
            consecutive_failures: 0,
            last_error: None,
            next_sync_at: now
                + Duration::seconds(if has_more {
                    1
                } else {
                    connector.sync_interval_seconds as i64
                }),
        };
        self.append_connector_sync_status(intent_id, status)
    }

    fn record_connector_sync_failure(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        started_at: DateTime<Utc>,
        error: &str,
        retry_after_seconds: Option<u64>,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        if !projection.connectors.contains_key(connector_id) {
            return Err(ApiError::NotFound(format!("connector {connector_id}")));
        }
        let previous = projection.connector_sync_status.get(connector_id);
        let failures = previous
            .map(|status| status.consecutive_failures)
            .unwrap_or(0)
            .saturating_add(1);
        let backoff = retry_after_seconds.unwrap_or_else(|| {
            let exponent = failures.saturating_sub(1).min(7);
            30_u64.saturating_mul(1_u64 << exponent).min(3600)
        });
        let now = Utc::now();
        let status = ConnectorSyncStatus {
            connector_id: connector_id.clone(),
            last_attempt_at: Some(started_at),
            last_success_at: previous.and_then(|status| status.last_success_at),
            consecutive_failures: failures,
            last_error: Some(error.into()),
            next_sync_at: now + Duration::seconds(backoff as i64),
        };
        self.append_connector_sync_status(intent_id, status)
    }

    fn append_connector_sync_status(
        &self,
        intent_id: &IntentId,
        status: ConnectorSyncStatus,
    ) -> Result<()> {
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::System,
            EventPayload::ConnectorSyncStatusChanged { status },
        ))?;
        Ok(())
    }

    fn sync_connector_inner(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        started_at: DateTime<Utc>,
    ) -> Result<ConnectorSyncResult> {
        let initial_projection = self.projection(intent_id)?;
        let connector = initial_projection
            .connectors
            .get(connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        if !connector.enabled {
            return Err(ApiError::Conflict(format!(
                "connector {connector_id} is disabled"
            )));
        }
        let adapter = self
            .connector_adapters
            .get(&connector.adapter)
            .ok_or_else(|| {
                ApiError::NotFound(format!("connector adapter {}", connector.adapter))
            })?;
        let pull_stream_id = adapter.pull_stream_id();
        let mut cursor = initial_projection
            .connector_cursors
            .get(&(connector.id.clone(), pull_stream_id.into()))
            .cloned();
        let credential = self.connector_credential(&connector)?;
        let mut result = ConnectorSyncResult::default();
        const MAX_PAGES_PER_SYNC: usize = 100;
        let mut has_more = false;
        let mut seen_continuations = HashSet::new();
        for _ in 0..MAX_PAGES_PER_SYNC {
            let page = adapter
                .pull(
                    connector.id.clone(),
                    &connector.source,
                    cursor.as_ref(),
                    started_at,
                    credential.as_ref(),
                )
                .map_err(ApiError::from)?;
            if page.stream_id != pull_stream_id {
                return Err(ApiError::Conflict(format!(
                    "adapter {} returned stream {} instead of {pull_stream_id}",
                    connector.adapter, page.stream_id
                )));
            }
            if let Some(continuation) = &page.continuation {
                if !seen_continuations.insert(continuation.clone()) {
                    return Err(ApiError::Conflict(format!(
                        "connector {} returned a repeated continuation {continuation}",
                        connector.adapter
                    )));
                }
            }
            let projection = self.projection(intent_id)?;
            result.observed = result
                .observed
                .saturating_add(page.observations.len() as u64);
            let unchanged = cursor.as_ref().is_some_and(|cursor| {
                cursor.opaque_cursor == page.opaque_cursor
                    && cursor.continuation == page.continuation
            });
            if unchanged {
                result.skipped = result
                    .skipped
                    .saturating_add(page.observations.len() as u64);
                if let Some(cursor) = &cursor {
                    result.cursor = Some(cursor.opaque_cursor.clone());
                    result.pages_synced = cursor.pages_synced;
                    result.backfill_complete = cursor.backfill_completed_at.is_some();
                }
                has_more = page.continuation.is_some();
                break;
            }
            let page_now = Utc::now();
            let next_cursor = SyncCursor {
                connector_id: connector.id.clone(),
                stream_id: page.stream_id.clone(),
                opaque_cursor: page.opaque_cursor.clone(),
                overlap_until: page.overlap_until,
                checkpointed_at: page_now,
                continuation: page.continuation.clone(),
                pages_synced: cursor
                    .as_ref()
                    .map(|cursor| cursor.pages_synced)
                    .unwrap_or(0)
                    .saturating_add(1),
                backfill_started_at: cursor
                    .as_ref()
                    .and_then(|cursor| cursor.backfill_started_at)
                    .or(Some(page_now)),
                backfill_completed_at: if page.continuation.is_some() {
                    None
                } else {
                    cursor
                        .as_ref()
                        .and_then(|cursor| cursor.backfill_completed_at)
                        .or(Some(page_now))
                },
            };
            let (inserted, skipped) =
                self.ingest_observations(intent_id, &projection, &page.observations)?;
            result.inserted = result.inserted.saturating_add(inserted);
            result.skipped = result.skipped.saturating_add(skipped);
            result.cursor = Some(page.opaque_cursor);
            result.pages_synced = next_cursor.pages_synced;
            result.backfill_complete = next_cursor.backfill_completed_at.is_some();
            self.record_connector_cursor(intent_id, next_cursor.clone())?;
            has_more = next_cursor.continuation.is_some();
            cursor = Some(next_cursor);
            if !has_more {
                break;
            }
        }
        result.has_more = has_more;
        let (candidates, auto_accepted) = self.resolve_signals_internal(intent_id)?;
        result.candidates_proposed = candidates.len() as u64;
        result.auto_accepted = auto_accepted;
        Ok(result)
    }

    pub fn world_signals(
        &self,
        cursor: u64,
        provider: Option<&str>,
        entity: Option<&str>,
    ) -> Result<Vec<WorldSignalRecord>> {
        let mut records = Vec::new();
        for spec in self.list_intents()? {
            let projection = self.projection(&spec.id)?;
            for signal in projection.world_signals.into_values() {
                let Some(seq) = projection.signal_seqs.get(&signal.id).copied() else {
                    continue;
                };
                if seq <= cursor
                    || provider.is_some_and(|provider| signal.entity_ref.provider != provider)
                    || entity.is_some_and(|entity| signal.entity_ref.external_id != entity)
                {
                    continue;
                }
                records.push(WorldSignalRecord {
                    seq,
                    intent_id: spec.id.clone(),
                    signal,
                });
            }
        }
        records.sort_by_key(|record| (record.seq, record.intent_id.clone()));
        Ok(records)
    }

    pub fn intent_candidates(&self, cursor: u64) -> Result<Vec<IntentCandidateRecord>> {
        let mut records = Vec::new();
        for spec in self.list_intents()? {
            let projection = self.projection(&spec.id)?;
            for candidate in projection.intent_candidates.into_values() {
                let Some(seq) = projection.candidate_seqs.get(&candidate.id).copied() else {
                    continue;
                };
                if seq <= cursor {
                    continue;
                }
                records.push(IntentCandidateRecord {
                    seq,
                    intent_id: spec.id.clone(),
                    candidate,
                });
            }
        }
        records.sort_by_key(|record| (record.seq, record.intent_id.clone()));
        Ok(records)
    }

    /// Webhook 复用同一 Normalize、去重、Candidate 与 Cursor 路径。
    pub fn ingest_connector_webhook(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        session_token: &str,
        payload: Value,
    ) -> Result<ConnectorSyncResult> {
        let started_at = Utc::now();
        let result =
            self.ingest_connector_webhook_inner(intent_id, connector_id, session_token, payload);
        match &result {
            Ok(_) => {
                let _ =
                    self.record_connector_sync_success(intent_id, connector_id, started_at, false);
            }
            Err(error) => {
                let _ = self.record_connector_sync_failure(
                    intent_id,
                    connector_id,
                    started_at,
                    &error.to_string(),
                    None,
                );
            }
        }
        result
    }

    fn ingest_connector_webhook_inner(
        &self,
        intent_id: &IntentId,
        connector_id: &ConnectorId,
        session_token: &str,
        payload: Value,
    ) -> Result<ConnectorSyncResult> {
        let projection = self.projection(intent_id)?;
        let connector = projection
            .connectors
            .get(connector_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("connector {connector_id}")))?;
        if connector.webhook_session.as_deref() != Some(session_token) {
            return Err(ApiError::Unauthorized(
                "invalid connector webhook session".into(),
            ));
        }
        let adapter = self
            .connector_adapters
            .get(&connector.adapter)
            .ok_or_else(|| {
                ApiError::NotFound(format!("connector adapter {}", connector.adapter))
            })?;
        let stream_id = adapter.webhook_stream_id().ok_or_else(|| {
            ApiError::Conflict(format!("connector {connector_id} does not accept webhooks"))
        })?;
        let credential = self.connector_credential(&connector)?;
        let hydrated_payload = adapter
            .hydrate_webhook(
                &connector.id,
                &connector.source,
                &payload,
                credential.as_ref(),
            )
            .map_err(ApiError::from)?;
        let hydrated = hydrated_payload != payload;
        let observations = adapter
            .normalize_webhook(&connector.id, &hydrated_payload)
            .map_err(ApiError::from)?;
        let mut result = ConnectorSyncResult {
            observed: observations.len() as u64,
            hydrated,
            ..ConnectorSyncResult::default()
        };
        let (inserted, skipped) =
            self.ingest_observations(intent_id, &projection, &observations)?;
        result.inserted = inserted;
        result.skipped = skipped;
        let cursor_value = format!("{stream_id}:{:016x}", stable_hash(&payload.to_string()));
        result.cursor = Some(cursor_value.clone());
        let unchanged = projection
            .connector_cursors
            .get(&(connector.id.clone(), stream_id.into()))
            .is_some_and(|cursor| cursor.opaque_cursor == cursor_value);
        if !unchanged {
            let previous = projection
                .connector_cursors
                .get(&(connector.id.clone(), stream_id.into()));
            let now = Utc::now();
            self.record_connector_cursor(
                intent_id,
                SyncCursor {
                    connector_id: connector.id.clone(),
                    stream_id: stream_id.into(),
                    opaque_cursor: cursor_value,
                    overlap_until: None,
                    checkpointed_at: now,
                    continuation: None,
                    pages_synced: previous
                        .map(|cursor| cursor.pages_synced)
                        .unwrap_or(0)
                        .saturating_add(1),
                    backfill_started_at: previous
                        .and_then(|cursor| cursor.backfill_started_at)
                        .or(Some(now)),
                    backfill_completed_at: previous
                        .and_then(|cursor| cursor.backfill_completed_at)
                        .or(Some(now)),
                },
            )?;
        }
        let (candidates, auto_accepted) = self.resolve_signals_internal(intent_id)?;
        result.candidates_proposed = candidates.len() as u64;
        result.auto_accepted = auto_accepted;
        Ok(result)
    }

    fn ingest_observations(
        &self,
        intent_id: &IntentId,
        projection: &else_ledger::IntentProjection,
        observations: &[ConnectorObservation],
    ) -> Result<(u64, u64)> {
        let mut seen = projection
            .signal_dedupe
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let mut inserted = 0;
        let mut skipped = 0;
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        for observation in observations {
            if !seen.insert(observation.dedupe_key.clone()) {
                skipped += 1;
                continue;
            }
            let signal = world_signal_from_observation(observation);
            let stored = self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                root.clone(),
                generation,
                revision,
                Actor::External,
                EventPayload::WorldSignalIngested {
                    signal: signal.clone(),
                },
            ))?;
            for interpretation in self.signal_interpreter.interpret(&signal, Utc::now()) {
                self.ledger.append(&EventEnvelope::new(
                    intent_id.clone(),
                    root.clone(),
                    generation,
                    revision,
                    Actor::System,
                    EventPayload::SignalInterpreted { interpretation },
                ))?;
            }
            self.analyze_plan_impact(
                intent_id,
                &stored.envelope.event_id,
                &observation.source,
                &observation.summary,
            )?;
            inserted += 1;
        }
        if inserted > 0 {
            let projection = self.projection(intent_id)?;
            if matches!(
                projection.state,
                Some(else_protocol::intent::IntentState::WaitingExternal)
                    | Some(else_protocol::intent::IntentState::Paused)
            ) {
                self.ledger.append(&EventEnvelope::new(
                    intent_id.clone(),
                    root,
                    generation,
                    revision,
                    Actor::System,
                    EventPayload::IntentStateChanged {
                        state: else_protocol::intent::IntentState::Active,
                    },
                ))?;
            }
        }
        Ok((inserted, skipped))
    }

    fn record_connector_cursor(
        &self,
        intent_id: &IntentId,
        cursor: SyncCursor,
    ) -> Result<SyncCursor> {
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::System,
            EventPayload::ConnectorSyncCheckpointed {
                cursor: cursor.clone(),
            },
        ))?;
        Ok(cursor)
    }

    pub fn configure_connector_policy(
        &self,
        intent_id: &IntentId,
        policy: ConnectorPolicy,
    ) -> Result<else_protocol::intent::IntentSpec> {
        if policy.min_confidence > 100 {
            return Err(ApiError::Conflict(
                "connector policy min_confidence must be between 0 and 100".into(),
            ));
        }
        if policy.max_autonomous_effect == SideEffectKind::Unknown {
            return Err(ApiError::Conflict(
                "connector policy cannot preauthorize Unknown effects".into(),
            ));
        }
        let projection = self.projection(intent_id)?;
        let mut spec = projection
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        if policy.max_autonomous_effect > spec.authority.max_autonomous_effect {
            return Err(ApiError::PermissionEscalation(
                "connector policy exceeds Intent Authority".into(),
            ));
        }
        spec.revision = spec.revision.saturating_add(1);
        spec.connector_policy = policy;
        let root = projection
            .root_scope_id
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let generation = projection
            .scopes
            .get(&root)
            .map(|scope| scope.generation)
            .unwrap_or(1);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            spec.revision,
            Actor::User,
            EventPayload::IntentRevised {
                spec: spec.clone(),
                reason: "用户配置 Connector 低风险自动接受策略".into(),
            },
        ))?;
        Ok(spec)
    }

    pub fn propose_intent_candidate(
        &self,
        intent_id: &IntentId,
        mut candidate: IntentCandidate,
    ) -> Result<IntentCandidate> {
        let projection = self.projection(intent_id)?;
        if let Some(existing_id) = projection.candidate_dedupe.get(&candidate.dedupe_key) {
            return projection
                .intent_candidates
                .get(existing_id)
                .cloned()
                .ok_or_else(|| {
                    ApiError::Conflict("candidate dedupe index is inconsistent".into())
                });
        }
        if projection.intent_candidates.contains_key(&candidate.id) {
            return Err(ApiError::Conflict("candidate already exists".into()));
        }
        let source_spec = projection
            .spec
            .as_ref()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        if candidate.proposed_outcome.trim().is_empty() {
            return Err(ApiError::Conflict(
                "candidate proposed_outcome cannot be empty".into(),
            ));
        }
        if candidate.done_when.is_empty() {
            return Err(ApiError::Conflict(
                "candidate done_when cannot be empty".into(),
            ));
        }
        if candidate.dedupe_key.trim().is_empty() {
            return Err(ApiError::Conflict(
                "candidate dedupe_key cannot be empty".into(),
            ));
        }
        if candidate.confidence > 100 {
            return Err(ApiError::Conflict(
                "candidate confidence must be between 0 and 100".into(),
            ));
        }
        if !source_spec.authority.permits_child(&candidate.authority) {
            return Err(ApiError::PermissionEscalation(format!(
                "candidate {}",
                candidate.id
            )));
        }
        if candidate
            .related_intent_id
            .as_ref()
            .is_some_and(|related| related != intent_id)
        {
            return Err(ApiError::Conflict(
                "candidate related_intent_id does not match source intent".into(),
            ));
        }
        if candidate
            .related_signal_ids
            .iter()
            .any(|id| !projection.world_signals.contains_key(id))
        {
            return Err(ApiError::NotFound(
                "candidate references an unknown world signal".into(),
            ));
        }
        candidate.state = IntentCandidateState::Proposed;
        candidate.accepted_intent_id = None;
        candidate.accepted_by = None;
        candidate.related_intent_id = Some(intent_id.clone());
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::System,
            EventPayload::IntentCandidateProposed {
                candidate: candidate.clone(),
            },
        ))?;
        Ok(candidate)
    }

    pub fn resolve_intent_candidate(
        &self,
        source_intent_id: &IntentId,
        candidate_id: &else_protocol::ids::IntentCandidateId,
        accept: bool,
    ) -> Result<Option<else_protocol::intent::IntentSpec>> {
        self.resolve_intent_candidate_as(
            source_intent_id,
            candidate_id,
            accept,
            if accept {
                Some(IntentAcceptedBy::User)
            } else {
                None
            },
        )
    }

    fn resolve_intent_candidate_as(
        &self,
        source_intent_id: &IntentId,
        candidate_id: &else_protocol::ids::IntentCandidateId,
        accept: bool,
        accepted_by: Option<IntentAcceptedBy>,
    ) -> Result<Option<else_protocol::intent::IntentSpec>> {
        let projection = self.projection(source_intent_id)?;
        let source_spec = projection
            .spec
            .as_ref()
            .ok_or_else(|| ApiError::IntentNotFound(source_intent_id.clone()))?;
        let candidate = projection
            .intent_candidates
            .get(candidate_id)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("candidate {candidate_id}")))?;
        if candidate.state != IntentCandidateState::Proposed {
            return Err(ApiError::Conflict(format!(
                "candidate {candidate_id} is {:?}",
                candidate.state
            )));
        }
        if !accept {
            candidate_state_event(
                self,
                source_intent_id,
                candidate_id,
                IntentCandidateState::Rejected,
                None,
                None,
            )?;
            return Ok(None);
        }
        let accepted_by = accepted_by.ok_or_else(|| {
            ApiError::Conflict("candidate acceptance requires an accepted_by actor".into())
        })?;
        if !source_spec.authority.permits_child(&candidate.authority) {
            return Err(ApiError::PermissionEscalation(format!(
                "candidate {candidate_id}"
            )));
        }

        // Candidate 接受后使用确定性 Intent ID；事件边界崩溃后重试不会重复建 Intent。
        let accepted_intent_id = IntentId::from(format!("candidate-{}", candidate.id));
        let existing = self.projection(&accepted_intent_id)?;
        let spec = if let Some(spec) = existing.spec {
            let belongs_to_candidate = spec
                .origin
                .as_ref()
                .and_then(|origin| origin.candidate_id.as_ref())
                == Some(&candidate.id);
            if !belongs_to_candidate {
                return Err(ApiError::Conflict(format!(
                    "deterministic intent id {accepted_intent_id} is already used"
                )));
            }
            spec
        } else {
            let connector_ids = candidate
                .related_signal_ids
                .iter()
                .filter_map(|id| projection.world_signals.get(id))
                .map(|signal| signal.connector_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            self.create_intent_from_parts(IntentDraft {
                outcome: candidate.proposed_outcome.clone(),
                project: source_spec.project.clone(),
                done_when: candidate.done_when.clone(),
                constraints: candidate.constraints.clone(),
                authority: candidate.authority.clone(),
                attention_policy: candidate.attention_policy.clone(),
                deadline: candidate.deadline,
                origin: Some(IntentOrigin {
                    kind: candidate.origin,
                    connector_ids,
                    signal_ids: candidate.related_signal_ids.clone(),
                    candidate_id: Some(candidate.id.clone()),
                    accepted_by,
                    accepted_at: Utc::now(),
                }),
            })?
        };
        candidate_state_event(
            self,
            source_intent_id,
            candidate_id,
            IntentCandidateState::Accepted,
            Some(spec.id.clone()),
            Some(accepted_by),
        )?;
        Ok(Some(spec))
    }

    /// Deterministic Resolver：只提出 Candidate；是否自动接受由显式 Connector Policy 决定。
    pub fn resolve_signals(&self, intent_id: &IntentId) -> Result<Vec<IntentCandidate>> {
        Ok(self.resolve_signals_internal(intent_id)?.0)
    }

    fn resolve_signals_internal(
        &self,
        intent_id: &IntentId,
    ) -> Result<(Vec<IntentCandidate>, u64)> {
        let projection = self.projection(intent_id)?;
        let source_spec = projection
            .spec
            .as_ref()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let policy = source_spec.connector_policy.clone();
        let mut candidates = Vec::new();
        let mut auto_accepted = 0;
        for signal in projection.world_signals.values() {
            if signal.status != SignalStatus::Active {
                continue;
            }
            for interpretation in projection
                .signal_interpretations
                .get(&signal.id)
                .into_iter()
                .flatten()
            {
                let Some(mut candidate) =
                    resolver_candidate(intent_id, source_spec, signal, interpretation)
                else {
                    continue;
                };
                if projection
                    .candidate_dedupe
                    .contains_key(&candidate.dedupe_key)
                {
                    continue;
                }
                candidate = self.propose_intent_candidate(intent_id, candidate)?;
                if policy_accepts(&policy, &candidate) {
                    self.resolve_intent_candidate_as(
                        intent_id,
                        &candidate.id,
                        true,
                        Some(IntentAcceptedBy::AutonomousRule),
                    )?;
                    candidate.state = IntentCandidateState::Accepted;
                    candidate.accepted_by = Some(IntentAcceptedBy::AutonomousRule);
                    auto_accepted += 1;
                }
                candidates.push(candidate);
            }
        }
        Ok((candidates, auto_accepted))
    }

    /// 显式 Tombstone 或 Superseded 转换。失效 Signal 不再进入 Context Snapshot，
    /// 关联的 Proposed Candidate 在投影中也随之失效。
    pub fn set_world_signal_status(
        &self,
        intent_id: &IntentId,
        signal_id: &else_protocol::ids::WorldSignalId,
        status: SignalStatus,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let signal = projection
            .world_signals
            .get(signal_id)
            .ok_or_else(|| ApiError::NotFound(format!("world signal {signal_id}")))?;
        if signal.status == status {
            return Ok(());
        }
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::System,
            EventPayload::WorldSignalStatusChanged {
                signal_id: signal_id.clone(),
                status,
            },
        ))?;
        Ok(())
    }
}

fn world_signal_from_observation(observation: &ConnectorObservation) -> WorldSignal {
    let status = match observation.change {
        ObservationChange::Upsert => SignalStatus::Active,
        ObservationChange::Delete => SignalStatus::Deleted,
        ObservationChange::Supersede => SignalStatus::Superseded,
    };
    WorldSignal {
        id: else_protocol::ids::WorldSignalId::new(),
        connector_id: observation.connector_id.clone(),
        entity_ref: ExternalEntityRef {
            provider: observation.source.clone(),
            tenant: None,
            account: None,
            entity_kind: if observation.entity_kind.is_empty() {
                "external".into()
            } else {
                observation.entity_kind.clone()
            },
            external_id: observation.entity_id.clone(),
            canonical_url: observation.canonical_url.clone(),
            source_version: Some(observation.source_version.clone()),
        },
        object_kind: observation.object_kind,
        change: observation.change,
        legacy_signal_type: None,
        actor: observation.actor.clone(),
        occurred_at: observation.occurred_at,
        valid_from: Some(observation.occurred_at),
        valid_until: observation.valid_until,
        summary: observation.summary.clone(),
        structured_facts: observation.attributes.clone(),
        evidence_ref: observation.evidence_ref.clone(),
        dedupe_key: observation.dedupe_key.clone(),
        sensitivity: observation.sensitivity.clone(),
        source_trust: observation.source_trust.clone(),
        status,
        correlations: observation.correlations.clone(),
    }
}

fn resolver_candidate(
    intent_id: &IntentId,
    source_spec: &else_protocol::intent::IntentSpec,
    signal: &WorldSignal,
    interpretation: &SignalInterpretation,
) -> Option<IntentCandidate> {
    let (origin, outcome, constraint) = match interpretation.meaning {
        SignalMeaning::Commitment => (
            IntentOriginKind::Commitment,
            format!("履行承诺：{}", signal.summary),
            "只读解析外部承诺；修改外部状态必须经 Tool Gateway",
        ),
        SignalMeaning::WaitingExternal => (
            IntentOriginKind::InferredNeed,
            format!("跟进等待中的事项：{}", signal.summary),
            "只读解析外部依赖；不得自动催促或修改外部状态",
        ),
        SignalMeaning::ActionItem if signal.object_kind == ObjectKind::CalendarEvent => (
            IntentOriginKind::InferredNeed,
            format!("为日程「{}」准备必要材料", signal.summary),
            "只读解析日程；外部写入必须经 Tool Gateway",
        ),
        SignalMeaning::ActionItem => (
            IntentOriginKind::InferredNeed,
            format!("推进工作项：{}", signal.summary),
            "只读解析工作项；状态变更必须经 Tool Gateway",
        ),
        SignalMeaning::Risk => (
            IntentOriginKind::InferredNeed,
            format!("处理风险：{}", signal.summary),
            "只读分析风险；处置动作必须经 Tool Gateway",
        ),
        SignalMeaning::Context => return None,
    };
    let mut forbidden = source_spec.authority.forbidden.clone();
    if !forbidden.iter().any(|rule| rule == constraint) {
        forbidden.push(constraint.into());
    }
    Some(IntentCandidate {
        id: else_protocol::ids::IntentCandidateId::new(),
        origin,
        proposed_outcome: outcome,
        done_when: vec![Criterion::manual("确认外部工作项推进完成")],
        constraints: vec![constraint.into()],
        authority: Authority {
            max_autonomous_effect: SideEffectKind::ReadOnly,
            forbidden,
        },
        attention_policy: AttentionPolicy {
            allow_interrupt: false,
            batch_interval_minutes: 60,
        },
        deadline: interpretation.deadline,
        related_signal_ids: vec![signal.id.clone()],
        related_intent_id: Some(intent_id.clone()),
        confidence: interpretation.confidence,
        dedupe_key: format!(
            "resolver:{}:{}",
            meaning_key(interpretation.meaning),
            signal.dedupe_key
        ),
        state: IntentCandidateState::Proposed,
        accepted_intent_id: None,
        accepted_by: None,
    })
}

fn meaning_key(meaning: SignalMeaning) -> &'static str {
    match meaning {
        SignalMeaning::ActionItem => "action-item",
        SignalMeaning::Commitment => "commitment",
        SignalMeaning::WaitingExternal => "waiting-external",
        SignalMeaning::Risk => "risk",
        SignalMeaning::Context => "context",
    }
}

fn policy_accepts(policy: &ConnectorPolicy, candidate: &IntentCandidate) -> bool {
    policy.auto_accept_low_risk
        && policy.allowed_origins.contains(&candidate.origin)
        && candidate.confidence >= policy.min_confidence
        && candidate.authority.max_autonomous_effect <= policy.max_autonomous_effect
        && candidate.authority.max_autonomous_effect != SideEffectKind::Unknown
}

fn candidate_state_event(
    runtime: &Runtime,
    intent_id: &IntentId,
    candidate_id: &else_protocol::ids::IntentCandidateId,
    state: IntentCandidateState,
    accepted_intent_id: Option<IntentId>,
    accepted_by: Option<IntentAcceptedBy>,
) -> Result<()> {
    let (root, generation, revision) = runtime.kernel_context(intent_id)?;
    let actor = if accepted_by == Some(IntentAcceptedBy::User) {
        Actor::User
    } else {
        Actor::System
    };
    runtime.ledger.append(&EventEnvelope::new(
        intent_id.clone(),
        root,
        generation,
        revision,
        actor,
        EventPayload::IntentCandidateStateChanged {
            candidate_id: candidate_id.clone(),
            state,
            intent_id: accepted_intent_id,
            accepted_by,
        },
    ))?;
    Ok(())
}

fn validate_connector_source(source: &str) -> Result<()> {
    let loopback_http =
        source.starts_with("http://127.0.0.1:") || source.starts_with("http://localhost:");
    if !source.starts_with("https://") && !loopback_http && !std::path::Path::new(source).is_file()
    {
        return Err(ApiError::NotFound(format!(
            "connector source is not an existing file, HTTPS URL, or loopback HTTP test endpoint: {source}"
        )));
    }
    Ok(())
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
