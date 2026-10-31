//! 从可重放 Signal / Interpretation 聚合 Personal World 与每日 Brief。

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use else_ledger::IntentProjection;
use else_protocol::connector::{CorrelationKind, ObjectKind, SignalMeaning, SignalStatus};
use else_protocol::ids::{ConnectorId, IntentId, WorldItemId};
use else_protocol::world::{
    PersonalBrief, ProjectWorld, WorldItem, WorldItemKind, WorldItemRecord, WorldItemResolution,
    WorldItemResolutionKind, WorldItemState,
};

use crate::{ApiError, Result, Runtime};

type WorldKey = (ConnectorId, String, &'static str);

struct MeaningSummary {
    meaning: SignalMeaning,
    confidence: u8,
    deadline: Option<DateTime<Utc>>,
}

impl Runtime {
    pub fn world_items(
        &self,
        intent_id: &IntentId,
        include_resolved: bool,
    ) -> Result<Vec<WorldItem>> {
        let projection = self.projection(intent_id)?;
        if projection.spec.is_none() {
            return Err(ApiError::IntentNotFound(intent_id.clone()));
        }
        Ok(build_world_items(&projection, Utc::now(), include_resolved))
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
        intent_id: Option<&IntentId>,
        include_resolved: bool,
    ) -> Result<Vec<ProjectWorld>> {
        let records = self.world_records(intent_id, include_resolved, Utc::now())?;
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
                        intent_ids: vec![],
                        connector_ids: vec![],
                        open_items: 0,
                        waiting_external: 0,
                        overdue: 0,
                        next_deadline: None,
                    });
                entry.items.push(record.clone());
                entry.intent_ids.push(record.intent_id.clone());
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
            project.intent_ids.sort();
            project.intent_ids.dedup();
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

    pub fn resolve_world_item(
        &self,
        intent_id: &IntentId,
        item_id: &WorldItemId,
        kind: WorldItemResolutionKind,
        note: Option<String>,
    ) -> Result<WorldItem> {
        let projection = self.projection(intent_id)?;
        if projection.spec.is_none() {
            return Err(ApiError::IntentNotFound(intent_id.clone()));
        }
        let items = build_world_items(&projection, Utc::now(), true);
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
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        let resolution = WorldItemResolution {
            item_id: item_id.clone(),
            kind,
            note,
            resolved_at: Utc::now(),
        };
        self.ledger
            .append(&else_protocol::event::EventEnvelope::new(
                intent_id.clone(),
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

    fn world_records(
        &self,
        intent_id: Option<&IntentId>,
        include_resolved: bool,
        now: DateTime<Utc>,
    ) -> Result<Vec<WorldItemRecord>> {
        let specs = match intent_id {
            Some(intent_id) => vec![self
                .projection(intent_id)?
                .spec
                .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?],
            None => self.list_intents()?,
        };
        let mut records = Vec::new();
        for spec in specs {
            let projection = self.projection(&spec.id)?;
            records.extend(
                build_world_items(&projection, now, include_resolved)
                    .into_iter()
                    .map(|item| WorldItemRecord {
                        intent_id: spec.id.clone(),
                        item,
                    }),
            );
        }
        Ok(records)
    }
}

fn build_world_items(
    projection: &IntentProjection,
    now: DateTime<Utc>,
    include_resolved: bool,
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
            .collect();
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
            signal_ids,
            connector_ids: vec![connector_id],
            projects,
            first_observed_at,
            last_observed_at,
        });
    }
    items.sort_by(|left, right| {
        left.state
            .is_resolved()
            .cmp(&right.state.is_resolved())
            .then_with(|| left.deadline.cmp(&right.deadline))
            .then_with(|| left.id.cmp(&right.id))
    });
    items
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
        left.item
            .deadline
            .cmp(&right.item.deadline)
            .then_with(|| left.item.id.cmp(&right.item.id))
            .then_with(|| left.intent_id.cmp(&right.intent_id))
    });
}
