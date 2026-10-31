//! 从可重放 Signal / Interpretation 聚合 Personal World 与每日 Brief。

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use else_ledger::IntentProjection;
use else_protocol::connector::{ObjectKind, SignalMeaning, SignalStatus};
use else_protocol::ids::{ConnectorId, IntentId, WorldItemId};
use else_protocol::world::{
    PersonalBrief, WorldItem, WorldItemKind, WorldItemRecord, WorldItemResolution,
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
        let mut records = Vec::new();
        for spec in self.list_intents()? {
            let projection = self.projection(&spec.id)?;
            records.extend(
                build_world_items(&projection, now, false)
                    .into_iter()
                    .map(|item| WorldItemRecord {
                        intent_id: spec.id.clone(),
                        item,
                    }),
            );
        }
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
