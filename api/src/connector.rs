//! 只读连接器、World Signal 与 Intent Candidate 的运行时实现。

use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use else_protocol::connector::{
    ConnectorKind, ConnectorRegistration, ConnectorSyncResult, ExternalEntityRef, IntentAcceptedBy,
    IntentCandidate, IntentCandidateState, IntentOrigin, IntentOriginKind, SignalStatus,
    SignalType, WorldSignal,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{ConnectorId, IntentId};
use else_protocol::intent::{AttentionPolicy, Authority, Criterion};

use crate::{ApiError, IntentDraft, Result, Runtime};

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
        let mut seen = projection
            .signal_dedupe
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        for fact in facts {
            if !seen.insert(fact.fact_id.clone()) {
                result.skipped += 1;
                continue;
            }
            let signal = WorldSignal {
                id: else_protocol::ids::WorldSignalId::new(),
                connector_id: connector.id.clone(),
                entity_ref: ExternalEntityRef {
                    provider: "calendar".into(),
                    tenant: None,
                    account: None,
                    entity_kind: "calendar_event".into(),
                    external_id: fact.entity_id,
                    canonical_url: None,
                    source_version: Some(fact.source_version),
                },
                signal_type: SignalType::CalendarEvent,
                actor: None,
                occurred_at: fact.occurred_at,
                valid_from: Some(fact.occurred_at),
                valid_until: fact.valid_until,
                summary: fact.summary,
                structured_facts: serde_json::json!({}),
                evidence_ref: None,
                dedupe_key: fact.fact_id,
                sensitivity: "internal".into(),
                source_trust: "deterministic_read_only".into(),
                status: SignalStatus::Active,
            };
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                root.clone(),
                generation,
                revision,
                Actor::External,
                EventPayload::WorldSignalIngested { signal },
            ))?;
            result.inserted += 1;
        }
        result.candidates_proposed = self.resolve_signals(intent_id)?.len() as u64;
        Ok(result)
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
            )?;
            return Ok(None);
        }
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
                    accepted_by: IntentAcceptedBy::User,
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
        )?;
        Ok(Some(spec))
    }

    /// Deterministic Resolver：只提出 Candidate，绝不自动创建 Intent。
    /// 24 小时内的 Calendar 信号会生成一个只读准备类候选。
    pub fn resolve_signals(&self, intent_id: &IntentId) -> Result<Vec<IntentCandidate>> {
        let projection = self.projection(intent_id)?;
        let source_spec = projection
            .spec
            .as_ref()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let now = Utc::now();
        let horizon = now + Duration::hours(24);
        let mut candidates = Vec::new();
        for signal in projection.world_signals.values() {
            if signal.signal_type != SignalType::CalendarEvent
                || signal.status != SignalStatus::Active
                || signal.occurred_at < now
                || signal.occurred_at > horizon
                || signal
                    .valid_until
                    .is_some_and(|valid_until| valid_until < now)
            {
                continue;
            }
            let dedupe_key = format!("resolver:calendar-prep:{}", signal.dedupe_key);
            if projection.candidate_dedupe.contains_key(&dedupe_key) {
                continue;
            }
            let mut forbidden = source_spec.authority.forbidden.clone();
            let write_guard = "不得修改外部日历，除非经 Gateway 写入 Capability".to_string();
            if !forbidden.contains(&write_guard) {
                forbidden.push(write_guard);
            }
            let candidate = self.propose_intent_candidate(
                intent_id,
                IntentCandidate {
                    id: else_protocol::ids::IntentCandidateId::new(),
                    origin: IntentOriginKind::InferredNeed,
                    proposed_outcome: format!("为日程「{}」准备必要材料", signal.summary),
                    done_when: vec![Criterion::manual("确认日程准备完成")],
                    constraints: vec!["只读解析日历；外部写入必须经 Tool Gateway".into()],
                    authority: Authority {
                        max_autonomous_effect: SideEffectKind::ReadOnly,
                        forbidden,
                    },
                    attention_policy: AttentionPolicy {
                        allow_interrupt: false,
                        batch_interval_minutes: 60,
                    },
                    deadline: Some(signal.occurred_at),
                    related_signal_ids: vec![signal.id.clone()],
                    related_intent_id: Some(intent_id.clone()),
                    confidence: 65,
                    dedupe_key,
                    state: IntentCandidateState::Proposed,
                    accepted_intent_id: None,
                },
            )?;
            candidates.push(candidate);
        }
        Ok(candidates)
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

fn candidate_state_event(
    runtime: &Runtime,
    intent_id: &IntentId,
    candidate_id: &else_protocol::ids::IntentCandidateId,
    state: IntentCandidateState,
    accepted_intent_id: Option<IntentId>,
) -> Result<()> {
    let (root, generation, revision) = runtime.kernel_context(intent_id)?;
    runtime.ledger.append(&EventEnvelope::new(
        intent_id.clone(),
        root,
        generation,
        revision,
        Actor::User,
        EventPayload::IntentCandidateStateChanged {
            candidate_id: candidate_id.clone(),
            state,
            intent_id: accepted_intent_id,
        },
    ))?;
    Ok(())
}
