//! Overview：今天真正要你管的事。
//!
//! 左边是「需要你的注意力」，右边是「你的世界」。这个分法是有意的：
//! 左边的每一项都在消耗你的注意力，右边是你在照看的东西。

use else_protocol::attention::DecisionChoice;
use else_protocol::ids::IntentId;
use else_protocol::world::WorldItemRecord;

use super::super::data::OverviewData;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

/// 注意力面板：决策、逾期、等待、风险。
pub fn attention_pane(data: &OverviewData) -> Pane {
    let mut rows = Vec::new();

    let requests = decision_requests(data);
    if requests.is_empty() {
        rows.push(Row::muted("no decisions waiting"));
    } else {
        rows.push(Row::heading("Decisions"));
        for (intent_id, request) in requests {
            rows.push(Row::action(
                format!("  {}  {}", request.summary, request.id),
                theme::critical(),
                RowAction::Decide {
                    intent_id,
                    request_id: request.id.clone(),
                    choice: DecisionChoice::Option("approve".into()),
                },
            ));
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

/// 世界面板：承诺、临近的行动、日程、进行中的 Intent 与项目。
pub fn world_pane(data: &OverviewData) -> Pane {
    let mut rows = Vec::new();

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

    rows.push(Row::heading("Intents"));
    if data.intents.is_empty() {
        rows.push(Row::muted("  none yet — press n to start one"));
    } else {
        for intent in &data.intents {
            let style = match intent.state {
                else_protocol::intent::IntentState::Completed
                | else_protocol::intent::IntentState::Cancelled => theme::done(),
                else_protocol::intent::IntentState::NeedsDecision => theme::critical(),
                else_protocol::intent::IntentState::Failed => theme::critical(),
                _ => theme::text(),
            };
            rows.push(Row::action(
                format!("  {:?}  {}", intent.state, intent.spec.outcome),
                style,
                RowAction::OpenIntent(intent.spec.id.clone()),
            ));
        }
    }

    if !data.projects.is_empty() {
        rows.push(Row::heading("Projects"));
        for project in &data.projects {
            rows.push(Row::styled(
                format!(
                    "  {}  open={} overdue={} next={}",
                    project.title,
                    project.open_items,
                    project.overdue,
                    project
                        .next_deadline
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "none".into())
                ),
                theme::text(),
            ));
        }
    }

    Pane {
        title: "Your world".into(),
        rows,
    }
}

/// 收集当前所有待裁决的请求。
fn decision_requests(
    data: &OverviewData,
) -> Vec<(IntentId, else_protocol::attention::AttentionRequest)> {
    let mut requests = Vec::new();
    for window in &data.attention {
        for request in &window.requests {
            requests.push((window.intent_id.clone(), request.clone()));
        }
    }
    // 没进窗口的请求也要出现在这里 —— 否则用户以为「没有要决定的事」。
    let grouped: Vec<_> = requests
        .iter()
        .map(|(_, request)| request.id.clone())
        .collect();
    for intent in &data.intents {
        for request in &intent.decisions {
            if !grouped.contains(&request.id) {
                requests.push((intent.spec.id.clone(), request.clone()));
            }
        }
    }
    requests
}

/// 追加一个 World 分组。
fn push_section(rows: &mut Vec<Row>, title: &str, records: &[WorldItemRecord], overdue: bool) {
    if records.is_empty() {
        return;
    }
    rows.push(Row::heading(title));
    for record in records {
        let style = if overdue {
            theme::critical()
        } else {
            theme::text()
        };
        rows.push(Row::action(
            format!("  {}  {}", record.item.title, record.item.id),
            style,
            RowAction::ResolveWorld {
                intent_id: record.intent_id.clone(),
                item_id: record.item.id.clone(),
                kind: else_protocol::world::WorldItemResolutionKind::Completed,
            },
        ));
    }
}
