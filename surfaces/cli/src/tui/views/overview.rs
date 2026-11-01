//! Today：需要本人注意的事、活跃工作与最近结果。

use else_protocol::ids::IntentId;
use else_protocol::intent::IntentState;
use else_protocol::world::WorldItemRecord;

use super::super::data::OverviewData;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;
use super::{intent_state_label, intent_style};

pub fn attention_pane(data: &OverviewData) -> Pane {
    let mut rows = Vec::new();
    let requests = decision_requests(data);
    rows.push(Row::heading("Needs you"));
    if requests.is_empty() {
        rows.push(Row::muted("  No decisions waiting"));
    } else {
        for (intent_id, request) in requests {
            rows.push(Row::action(
                format!("  {}", request.summary),
                theme::critical(),
                RowAction::OpenDecision {
                    intent_id,
                    request_id: request.id.clone(),
                },
            ));
            rows.push(Row::muted(format!("      {}", request.blocked_reason)));
        }
    }

    push_section(&mut rows, "Overdue", &data.brief.overdue, true);
    push_section(
        &mut rows,
        "Waiting on others",
        &data.brief.waiting_external,
        false,
    );
    push_section(&mut rows, "Risks", &data.brief.risks, false);
    Pane {
        title: "Attention".into(),
        rows,
    }
}

pub fn world_pane(data: &OverviewData) -> Pane {
    let mut rows = Vec::new();
    rows.push(Row::heading("Active"));
    let active: Vec<_> = data
        .intents
        .iter()
        .filter(|intent| {
            !matches!(
                intent.state,
                IntentState::Completed | IntentState::Cancelled | IntentState::Failed
            )
        })
        .collect();
    if active.is_empty() {
        rows.push(Row::muted("  No active intents · press n to create one"));
    } else {
        for intent in active {
            rows.push(intent_row(intent));
        }
    }

    let finished: Vec<_> = data
        .intents
        .iter()
        .filter(|intent| intent.state == IntentState::Completed)
        .take(3)
        .collect();
    if !finished.is_empty() {
        rows.push(Row::heading("Recently completed"));
        for intent in finished {
            rows.push(Row::action(
                format!("  ✓ {}", intent.spec.outcome),
                theme::done(),
                RowAction::OpenIntent(intent.spec.id.clone()),
            ));
            rows.push(Row::muted(format!(
                "      {} artifact(s)",
                intent.artifacts.len()
            )));
        }
    }

    push_section(
        &mut rows,
        "Commitments",
        &data.brief.open_commitments,
        false,
    );
    push_section(
        &mut rows,
        "Due soon",
        &data.brief.action_items_due_soon,
        false,
    );
    push_section(&mut rows, "Upcoming", &data.brief.upcoming_calendar, false);

    if !data.projects.is_empty() {
        rows.push(Row::heading("Projects"));
        for project in &data.projects {
            rows.push(Row::styled(
                format!(
                    "  {} · {} open · {} overdue",
                    project.title, project.open_items, project.overdue
                ),
                theme::text(),
            ));
        }
    }
    if !data.identities.is_empty() {
        rows.push(Row::heading("Relationships"));
        for identity in &data.identities {
            rows.push(Row::styled(
                format!(
                    "  {} · {} open · {} overdue",
                    identity.title, identity.open_items, identity.overdue
                ),
                theme::text(),
            ));
        }
    }
    Pane {
        title: "Your work".into(),
        rows,
    }
}

fn intent_row(intent: &else_protocol::api::IntentSnapshot) -> Row {
    let project = intent
        .spec
        .project
        .as_deref()
        .map(|project| format!(" · {project}"))
        .unwrap_or_default();
    Row::action(
        format!(
            "  {}  {}{}",
            intent_state_label(intent.state),
            intent.spec.outcome,
            project
        ),
        intent_style(intent.state),
        RowAction::OpenIntent(intent.spec.id.clone()),
    )
}

fn decision_requests(
    data: &OverviewData,
) -> Vec<(IntentId, else_protocol::attention::AttentionRequest)> {
    let mut requests = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for window in &data.attention {
        for request in &window.requests {
            if seen.insert(request.id.clone()) {
                requests.push((window.intent_id.clone(), request.clone()));
            }
        }
    }
    for intent in &data.intents {
        for request in &intent.decisions {
            if seen.insert(request.id.clone()) {
                requests.push((intent.spec.id.clone(), request.clone()));
            }
        }
    }
    requests
}

fn push_section(rows: &mut Vec<Row>, title: &str, records: &[WorldItemRecord], overdue: bool) {
    if records.is_empty() {
        return;
    }
    rows.push(Row::heading(title));
    for record in records {
        rows.push(Row::action(
            format!("  {} · Enter to complete", record.item.title),
            if overdue {
                theme::critical()
            } else {
                theme::text()
            },
            RowAction::ResolveWorld {
                intent_id: record.intent_id.clone(),
                item_id: record.item.id.clone(),
                kind: else_protocol::world::WorldItemResolutionKind::Completed,
            },
        ));
        if let Some(deadline) = record.item.deadline {
            rows.push(Row::muted(format!(
                "      due {}",
                deadline.format("%Y-%m-%d %H:%M UTC")
            )));
        }
    }
}
