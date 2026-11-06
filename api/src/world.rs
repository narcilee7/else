//! 从可重放 Signal / Interpretation 聚合 Personal World 与每日 Brief。

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use else_ledger::ThreadProjection;
use else_protocol::connector::{CorrelationKind, ObjectKind, SignalMeaning, SignalStatus};
use else_protocol::ids::{ConnectorId, ThreadId, WorldItemId};
use else_protocol::world::{
    IdentityWorld, PersonalBrief, ProjectWorld, WorldCorrelationCorrection,
    WorldCorrelationCorrectionKind, WorldItem, WorldItemKind, WorldItemRecord, WorldItemResolution,
    WorldItemResolutionKind, WorldItemState,
};

use crate::{ApiError, Result, Runtime};

type WorldKey = (ConnectorId, String, &'static str);

struct MeaningSummary {
    meaning: SignalMeaning,
    confidence: u8,
    deadline: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct CorrelationCorrections {
    merges: BTreeMap<(CorrelationKind, String), (String, Option<String>)>,
    renames: BTreeMap<(CorrelationKind, String), String>,
    splits: BTreeSet<(WorldItemId, CorrelationKind, String)>,
}

impl Runtime {
    pub fn world_items(
        &self,
        thread_id: &ThreadId,
        include_resolved: bool,
    ) -> Result<Vec<WorldItem>> {
        let projection = self.projection(thread_id)?;
        if projection.spec.is_none() {
            return Err(ApiError::ThreadNotFound(thread_id.clone()));
        }
        let corrections = self.world_corrections()?;
        Ok(build_world_items(
            &projection,
            Utc::now(),
            include_resolved,
            &corrections,
        ))
    }

    pub fn personal_brief(&self) -> Result<PersonalBrief> {
        let now = Utc::now();
        let records = self.world_records(None, false, now)?;
        let due_soon = now + Duration::hours(72);
        let calendar_soon = now + Duration::hours(24);
        let mut brief = PersonalBrief {
            generated_at: now,
            open_commitments: vec![],
            waiting_external: vec![],
            overdue: vec![],
            action_items_due_soon: vec![],
            risks: vec![],
            upcoming_calendar: vec![],
        };
        for record in records {
            if record.item.state == WorldItemState::Overdue {
                brief.overdue.push(record.clone());
            }
            if record.item.object_kind == ObjectKind::CalendarEvent
                && record
                    .item
                    .deadline
                    .is_some_and(|deadline| deadline >= now && deadline <= calendar_soon)
            {
                brief.upcoming_calendar.push(record.clone());
            }
            match record.item.kind {
                WorldItemKind::Commitment => brief.open_commitments.push(record),
                WorldItemKind::WaitingExternal => brief.waiting_external.push(record),
                WorldItemKind::ActionItem
                    if record.item.state == WorldItemState::Open
                        && record
                            .item
                            .deadline
                            .is_some_and(|deadline| deadline >= now && deadline <= due_soon) =>
                {
                    brief.action_items_due_soon.push(record);
                }
                WorldItemKind::Risk => brief.risks.push(record),
                _ => {}
            }
        }
        sort_records(&mut brief.open_commitments);
        sort_records(&mut brief.waiting_external);
        sort_records(&mut brief.overdue);
        sort_records(&mut brief.action_items_due_soon);
        sort_records(&mut brief.risks);
        sort_records(&mut brief.upcoming_calendar);
        Ok(brief)
    }

    pub fn project_worlds(
        &self,
        thread_id: Option<&ThreadId>,
        include_resolved: bool,
    ) -> Result<Vec<ProjectWorld>> {
        let records = self.world_records(thread_id, include_resolved, Utc::now())?;
        let mut projects = BTreeMap::<String, ProjectWorld>::new();
        for record in records {
            for project in record
                .item
                .projects
                .iter()
                .filter(|project| project.kind == CorrelationKind::Project)
            {
                let entry = projects
                    .entry(project.key.clone())
                    .or_insert_with(|| ProjectWorld {
                        key: project.key.clone(),
                        title: project.label.clone(),
                        items: vec![],
                        thread_ids: vec![],
                        connector_ids: vec![],
                        open_items: 0,
                        waiting_external: 0,
                        overdue: 0,
                        next_deadline: None,
                    });
                entry.items.push(record.clone());
                entry.thread_ids.push(record.thread_id.clone());
                entry
                    .connector_ids
                    .extend(record.item.connector_ids.iter().cloned());
                if !record.item.state.is_resolved() {
                    entry.open_items += 1;
                }
                if record.item.state == WorldItemState::WaitingExternal {
                    entry.waiting_external += 1;
                }
                if record.item.state == WorldItemState::Overdue {
                    entry.overdue += 1;
                }
                if !record.item.state.is_resolved()
                    && record.item.deadline.is_some_and(|deadline| {
                        entry.next_deadline.is_none_or(|current| deadline < current)
                    })
                {
                    entry.next_deadline = record.item.deadline;
                }
            }
        }
        let mut projects = projects.into_values().collect::<Vec<_>>();
        for project in &mut projects {
            project.thread_ids.sort();
            project.thread_ids.dedup();
            project.connector_ids.sort();
            project.connector_ids.dedup();
            sort_records(&mut project.items);
        }
        projects.sort_by(|left, right| {
            right
                .overdue
                .cmp(&left.overdue)
                .then_with(|| right.waiting_external.cmp(&left.waiting_external))
                .then_with(|| left.next_deadline.cmp(&right.next_deadline))
                .then_with(|| left.title.cmp(&right.title))
        });
        Ok(projects)
    }

    pub fn identity_worlds(
        &self,
        thread_id: Option<&ThreadId>,
        include_resolved: bool,
    ) -> Result<Vec<IdentityWorld>> {
        let records = self.world_records(thread_id, include_resolved, Utc::now())?;
        let mut identities = BTreeMap::<(CorrelationKind, String), IdentityWorld>::new();
        for record in records {
            for identity in record
                .item
                .identities
                .iter()
                .filter(|identity| identity.kind != CorrelationKind::Project)
            {
                let key = (identity.kind, identity.key.clone());
                let entry = identities
                    .entry(key.clone())
                    .or_insert_with(|| IdentityWorld {
                        kind: identity.kind,
                        key: key.1,
                        title: identity.label.clone(),
                        items: vec![],
                        thread_ids: vec![],
                        connector_ids: vec![],
                        open_items: 0,
                        overdue: 0,
                        next_deadline: None,
                    });
                entry.items.push(record.clone());
                entry.thread_ids.push(record.thread_id.clone());
                entry
                    .connector_ids
                    .extend(record.item.connector_ids.iter().cloned());
                if !record.item.state.is_resolved() {
                    entry.open_items += 1;
                }
                if record.item.state == WorldItemState::Overdue {
                    entry.overdue += 1;
                }
                if !record.item.state.is_resolved()
                    && record.item.deadline.is_some_and(|deadline| {
                        entry.next_deadline.is_none_or(|current| deadline < current)
                    })
                {
                    entry.next_deadline = record.item.deadline;
                }
            }
        }
        let mut identities = identities.into_values().collect::<Vec<_>>();
        for identity in &mut identities {
            identity.thread_ids.sort();
            identity.thread_ids.dedup();
            identity.connector_ids.sort();
            identity.connector_ids.dedup();
            sort_records(&mut identity.items);
        }
        identities.sort_by(|left, right| {
            right
                .overdue
                .cmp(&left.overdue)
                .then_with(|| right.open_items.cmp(&left.open_items))
                .then_with(|| left.next_deadline.cmp(&right.next_deadline))
                .then_with(|| left.title.cmp(&right.title))
        });
        Ok(identities)
    }

    pub fn resolve_world_item(
        &self,
        thread_id: &ThreadId,
        item_id: &WorldItemId,
        kind: WorldItemResolutionKind,
        note: Option<String>,
    ) -> Result<WorldItem> {
        let projection = self.projection(thread_id)?;
        if projection.spec.is_none() {
            return Err(ApiError::ThreadNotFound(thread_id.clone()));
        }
        let corrections = self.world_corrections()?;
        let items = build_world_items(&projection, Utc::now(), true, &corrections);
        let mut item = items
            .into_iter()
            .find(|item| &item.id == item_id)
            .ok_or_else(|| ApiError::NotFound(format!("world item {item_id}")))?;
        if projection
            .world_item_resolutions
            .get(item_id)
            .is_some_and(|resolution| resolution.kind == kind)
        {
            return Ok(item);
        }
        let (root, generation, revision) = self.kernel_context(thread_id)?;
        let resolution = WorldItemResolution {
            item_id: item_id.clone(),
            kind,
            note,
            resolved_at: Utc::now(),
        };
        self.ledger
            .append(&else_protocol::event::EventEnvelope::new(
                thread_id.clone(),
                root,
                generation,
                revision,
                else_protocol::event::Actor::User,
                else_protocol::event::EventPayload::WorldItemResolved {
                    resolution: resolution.clone(),
                },
            ))?;
        item.state = match resolution.kind {
            WorldItemResolutionKind::Completed => WorldItemState::Completed,
            WorldItemResolutionKind::Dismissed => WorldItemState::Dismissed,
        };
        Ok(item)
    }

    pub fn correct_world_correlation(
        &self,
        thread_id: &ThreadId,
        mut correction: WorldCorrelationCorrection,
    ) -> Result<WorldCorrelationCorrection> {
        match correction.kind {
            WorldCorrelationCorrectionKind::Merge => {
                if correction.source_keys.is_empty()
                    || correction
                        .target_key
                        .as_deref()
                        .is_none_or(|key| key.trim().is_empty())
                {
                    return Err(ApiError::Conflict(
                        "merge requires source_keys and target_key".into(),
                    ));
                }
            }
            WorldCorrelationCorrectionKind::Split => {
                if correction.item_id.is_none() || correction.source_keys.len() != 1 {
                    return Err(ApiError::Conflict(
                        "split requires one source key and item_id".into(),
                    ));
                }
            }
            WorldCorrelationCorrectionKind::Rename => {
                if correction
                    .target_key
                    .as_deref()
                    .is_none_or(|key| key.trim().is_empty())
                    || correction
                        .label
                        .as_deref()
                        .is_none_or(|label| label.trim().is_empty())
                {
                    return Err(ApiError::Conflict(
                        "rename requires target_key and label".into(),
                    ));
                }
            }
        }
        correction.corrected_at = Utc::now();
        let (root, generation, revision) = self.kernel_context(thread_id)?;
        self.ledger
            .append(&else_protocol::event::EventEnvelope::new(
                thread_id.clone(),
                root,
                generation,
                revision,
                else_protocol::event::Actor::User,
                else_protocol::event::EventPayload::WorldCorrelationCorrected {
                    correction: correction.clone(),
                },
            ))?;
        Ok(correction)
    }

    fn world_records(
        &self,
        thread_id: Option<&ThreadId>,
        include_resolved: bool,
        now: DateTime<Utc>,
    ) -> Result<Vec<WorldItemRecord>> {
        let specs = match thread_id {
            Some(thread_id) => vec![self
                .projection(thread_id)?
                .spec
                .ok_or_else(|| ApiError::ThreadNotFound(thread_id.clone()))?],
            None => self.list_threads()?,
        };
        let corrections = self.world_corrections()?;
        let mut records = Vec::new();
        for spec in specs {
            let projection = self.projection(&spec.id)?;
            records.extend(
                build_world_items(&projection, now, include_resolved, &corrections)
                    .into_iter()
                    .map(|item| WorldItemRecord {
                        thread_id: spec.id.clone(),
                        item,
                    }),
            );
        }
        Ok(records)
    }

    fn world_corrections(&self) -> Result<CorrelationCorrections> {
        let mut corrections = self
            .list_threads()?
            .into_iter()
            .flat_map(|spec| {
                self.projection(&spec.id)
                    .map(|projection| {
                        projection
                            .world_correlation_corrections
                            .into_values()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        corrections.sort_by_key(|correction| (correction.corrected_at, correction.id.clone()));
        let mut applied = CorrelationCorrections::default();
        for correction in corrections {
            match correction.kind {
                WorldCorrelationCorrectionKind::Merge => {
                    if let Some(target) = correction.target_key {
                        for source in correction.source_keys {
                            applied.merges.insert(
                                (correction.correlation_kind, source),
                                (target.clone(), correction.label.clone()),
                            );
                        }
                        if let Some(label) = correction.label {
                            applied
                                .renames
                                .insert((correction.correlation_kind, target), label);
                        }
                    }
                }
                WorldCorrelationCorrectionKind::Rename => {
                    if let (Some(key), Some(label)) = (correction.target_key, correction.label) {
                        applied
                            .renames
                            .insert((correction.correlation_kind, key), label);
                    }
                }
                WorldCorrelationCorrectionKind::Split => {
                    if let (Some(item_id), Some(key)) = (
                        correction.item_id,
                        correction.source_keys.into_iter().next(),
                    ) {
                        applied
                            .splits
                            .insert((item_id, correction.correlation_kind, key));
                    }
                }
            }
        }
        Ok(applied)
    }
}

fn build_world_items(
    projection: &ThreadProjection,
    now: DateTime<Utc>,
    include_resolved: bool,
    corrections: &CorrelationCorrections,
) -> Vec<WorldItem> {
    let mut meanings = BTreeMap::<WorldKey, MeaningSummary>::new();
    for interpretations in projection.signal_interpretations.values() {
        for interpretation in interpretations {
            if interpretation.meaning == SignalMeaning::Context {
                continue;
            }
            let Some(signal) = projection.world_signals.get(&interpretation.signal_id) else {
                continue;
            };
            let key = (
                signal.connector_id.clone(),
                signal.entity_ref.external_id.clone(),
                meaning_key(interpretation.meaning),
            );
            meanings
                .entry(key)
                .and_modify(|current| {
                    if interpretation.confidence >= current.confidence {
                        current.meaning = interpretation.meaning;
                        current.confidence = interpretation.confidence;
                        current.deadline = interpretation.deadline;
                    }
                })
                .or_insert(MeaningSummary {
                    meaning: interpretation.meaning,
                    confidence: interpretation.confidence,
                    deadline: interpretation.deadline,
                });
        }
    }

    let mut items = Vec::new();
    for ((connector_id, external_id, meaning), summary) in meanings {
        let versions = projection
            .world_signals
            .values()
            .filter(|signal| {
                signal.connector_id == connector_id && signal.entity_ref.external_id == external_id
            })
            .collect::<Vec<_>>();
        let Some(latest) = versions.iter().max_by_key(|signal| {
            (
                signal.occurred_at,
                projection.signal_seqs.get(&signal.id).copied().unwrap_or(0),
            )
        }) else {
            continue;
        };
        let latest_meanings = projection
            .signal_interpretations
            .get(&latest.id)
            .into_iter()
            .flatten()
            .filter(|interpretation| interpretation.meaning != SignalMeaning::Context)
            .map(|interpretation| meaning_key(interpretation.meaning))
            .collect::<BTreeSet<_>>();
        if latest.status == SignalStatus::Active
            && !latest_meanings.is_empty()
            && !latest_meanings.contains(meaning)
        {
            continue;
        }
        let display = versions
            .iter()
            .filter(|signal| {
                projection
                    .signal_interpretations
                    .get(&signal.id)
                    .into_iter()
                    .flatten()
                    .any(|interpretation| meaning_key(interpretation.meaning) == meaning)
            })
            .max_by_key(|signal| {
                (
                    signal.occurred_at,
                    projection.signal_seqs.get(&signal.id).copied().unwrap_or(0),
                )
            })
            .copied()
            .unwrap_or(*latest);
        let item_id = WorldItemId::from(format!("world:{connector_id}:{external_id}:{meaning}"));
        let mut state = world_state(latest.status, summary.meaning, summary.deadline, now);
        if let Some(resolution) = projection.world_item_resolutions.get(&item_id) {
            state = match resolution.kind {
                WorldItemResolutionKind::Completed => WorldItemState::Completed,
                WorldItemResolutionKind::Dismissed => WorldItemState::Dismissed,
            };
        }
        if state.is_resolved() && !include_resolved {
            continue;
        }
        let first_observed_at = versions
            .iter()
            .map(|signal| signal.occurred_at)
            .min()
            .unwrap_or(display.occurred_at);
        let last_observed_at = versions
            .iter()
            .map(|signal| signal.occurred_at)
            .max()
            .unwrap_or(display.occurred_at);
        let signal_ids = versions.iter().map(|signal| signal.id.clone()).collect();
        let projects = versions
            .iter()
            .flat_map(|signal| signal.correlations.iter())
            .filter(|hint| hint.kind == else_protocol::connector::CorrelationKind::Project)
            .fold(BTreeMap::new(), |mut projects, hint| {
                projects.entry(hint.key.clone()).or_insert(hint.clone());
                projects
            })
            .into_values()
            .collect::<Vec<_>>();
        let projects = apply_correlation_corrections(&item_id, projects, corrections);
        let identities = versions
            .iter()
            .flat_map(|signal| signal.correlations.iter())
            .filter(|hint| hint.kind != CorrelationKind::Project)
            .fold(BTreeMap::new(), |mut identities, hint| {
                identities
                    .entry((hint.kind, hint.key.clone()))
                    .or_insert(hint.clone());
                identities
            })
            .into_values()
            .collect::<Vec<_>>();
        let identities = apply_correlation_corrections(&item_id, identities, corrections);
        let (priority, priority_reason) = world_priority(
            summary.meaning,
            state,
            summary.deadline,
            summary.confidence,
            now,
        );
        items.push(WorldItem {
            id: item_id,
            entity_ref: latest.entity_ref.clone(),
            object_kind: display.object_kind,
            kind: world_kind(summary.meaning),
            state,
            title: display
                .structured_facts
                .get("title")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| display.summary.clone()),
            summary: display.summary.clone(),
            deadline: summary.deadline,
            priority,
            priority_reason,
            signal_ids,
            connector_ids: vec![connector_id],
            projects,
            identities,
            first_observed_at,
            last_observed_at,
        });
    }
    items.sort_by(|left, right| {
        left.state
            .is_resolved()
            .cmp(&right.state.is_resolved())
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.deadline.cmp(&right.deadline))
            .then_with(|| left.id.cmp(&right.id))
    });
    items
}

fn apply_correlation_corrections(
    item_id: &WorldItemId,
    hints: Vec<else_protocol::connector::CorrelationHint>,
    corrections: &CorrelationCorrections,
) -> Vec<else_protocol::connector::CorrelationHint> {
    let mut corrected = Vec::new();
    let mut seen = BTreeSet::new();
    for mut hint in hints {
        let original_key = hint.key.clone();
        let canonical = resolve_correlation_key(corrections, hint.kind, &original_key);
        if corrections
            .splits
            .contains(&(item_id.clone(), hint.kind, original_key.clone()))
            || corrections
                .splits
                .contains(&(item_id.clone(), hint.kind, canonical.clone()))
        {
            continue;
        }
        hint.key = canonical.clone();
        if let Some(label) = corrections
            .renames
            .get(&(hint.kind, canonical.clone()))
            .or_else(|| corrections.renames.get(&(hint.kind, original_key)))
        {
            hint.label = label.clone();
        }
        if seen.insert((hint.kind, hint.key.clone())) {
            corrected.push(hint);
        }
    }
    corrected
}

fn resolve_correlation_key(
    corrections: &CorrelationCorrections,
    kind: CorrelationKind,
    key: &str,
) -> String {
    let mut canonical = key.to_string();
    for _ in 0..16 {
        let Some((target, _)) = corrections.merges.get(&(kind, canonical.clone())) else {
            break;
        };
        if target == &canonical {
            break;
        }
        canonical = target.clone();
    }
    canonical
}

fn world_state(
    signal_status: SignalStatus,
    meaning: SignalMeaning,
    deadline: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> WorldItemState {
    match signal_status {
        SignalStatus::Deleted => WorldItemState::Completed,
        SignalStatus::Superseded => WorldItemState::Superseded,
        SignalStatus::Active => {
            if deadline.is_some_and(|deadline| deadline < now) {
                WorldItemState::Overdue
            } else {
                match meaning {
                    SignalMeaning::WaitingExternal => WorldItemState::WaitingExternal,
                    SignalMeaning::Risk => WorldItemState::AtRisk,
                    _ => WorldItemState::Open,
                }
            }
        }
    }
}

fn world_kind(meaning: SignalMeaning) -> WorldItemKind {
    match meaning {
        SignalMeaning::Commitment => WorldItemKind::Commitment,
        SignalMeaning::WaitingExternal => WorldItemKind::WaitingExternal,
        SignalMeaning::ActionItem => WorldItemKind::ActionItem,
        SignalMeaning::Risk => WorldItemKind::Risk,
        SignalMeaning::Context => unreachable!("context interpretations are filtered"),
    }
}

fn world_priority(
    meaning: SignalMeaning,
    state: WorldItemState,
    deadline: Option<DateTime<Utc>>,
    confidence: u8,
    now: DateTime<Utc>,
) -> (u8, String) {
    let (mut priority, mut reason): (u8, String) = match state {
        WorldItemState::Overdue => (94, "已逾期，需要立即收敛".to_string()),
        WorldItemState::AtRisk => (86, "外部事实被标记为风险".to_string()),
        WorldItemState::WaitingExternal => (64, "事项被外部对象阻塞".to_string()),
        WorldItemState::Open => match meaning {
            SignalMeaning::Commitment => (76, "存在尚未完成的对外承诺".to_string()),
            SignalMeaning::ActionItem => (58, "存在待推进的行动项".to_string()),
            SignalMeaning::Risk => (86, "外部事实被标记为风险".to_string()),
            SignalMeaning::WaitingExternal => (64, "事项被外部对象阻塞".to_string()),
            SignalMeaning::Context => (20, "仅作为背景上下文".to_string()),
        },
        WorldItemState::Completed | WorldItemState::Dismissed | WorldItemState::Superseded => {
            (10, "事项已经收敛".to_string())
        }
    };
    if !state.is_resolved() {
        if let Some(deadline) = deadline {
            let remaining = deadline.signed_duration_since(now);
            if remaining <= Duration::hours(6) {
                priority = priority.saturating_add(14).min(99);
                reason.push_str("；6 小时内到期");
            } else if remaining <= Duration::hours(24) {
                priority = priority.saturating_add(9).min(99);
                reason.push_str("；24 小时内到期");
            } else if remaining <= Duration::hours(72) {
                priority = priority.saturating_add(5).min(99);
                reason.push_str("；72 小时内到期");
            }
        }
    }
    if confidence < 60 && !state.is_resolved() {
        priority = priority.saturating_sub(6);
        reason.push_str("；解释置信度偏低");
    }
    (priority, reason)
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

fn sort_records(records: &mut [WorldItemRecord]) {
    records.sort_by(|left, right| {
        right
            .item
            .priority
            .cmp(&left.item.priority)
            .then_with(|| left.item.deadline.cmp(&right.item.deadline))
            .then_with(|| left.item.id.cmp(&right.item.id))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
}
