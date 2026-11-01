//! Decisions 队列与详情。列表只负责导航，真正的选择在详情中发生。

use else_protocol::attention::{AttentionRequest, DecisionChoice, ExpectedLoss};
use else_protocol::ids::{AttentionId, IntentId};

use super::super::app::DecisionFocus;
use super::super::data::{Data, DecisionsData};
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

pub fn all_requests(data: &DecisionsData) -> Vec<(IntentId, AttentionRequest)> {
    let mut requests = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for window in &data.windows {
        for request in &window.requests {
            if seen.insert(request.id.clone()) {
                requests.push((window.intent_id.clone(), request.clone()));
            }
        }
    }
    for (intent_id, request) in &data.loose {
        if seen.insert(request.id.clone()) {
            requests.push((intent_id.clone(), request.clone()));
        }
    }
    requests
}

pub fn find_request<'a>(data: &'a Data, request_id: &AttentionId) -> Option<&'a AttentionRequest> {
    let Data::Decisions(data) = data else {
        return None;
    };
    data.windows
        .iter()
        .flat_map(|window| window.requests.iter())
        .chain(data.loose.iter().map(|(_, request)| request))
        .find(|request| &request.id == request_id)
}

pub fn pane(data: &DecisionsData) -> Pane {
    let requests = all_requests(data);
    let mut rows = Vec::new();
    if requests.is_empty() {
        rows.push(Row::muted("Nothing needs your decision right now."));
        rows.push(Row::muted(
            "New requests will appear here without interrupting your work.",
        ));
        return Pane {
            title: "Decisions".into(),
            rows,
        };
    }
    rows.push(Row::muted(format!(
        "{} request(s) · Enter to review before choosing",
        requests.len()
    )));
    for (intent_id, request) in requests {
        rows.push(Row::action(
            format!("  {}", request.summary),
            loss_style(request.expected_loss),
            RowAction::OpenDecision {
                intent_id,
                request_id: request.id.clone(),
            },
        ));
        rows.push(Row::muted(format!(
            "      {} · {} impact",
            request.blocked_reason,
            loss_label(request.expected_loss)
        )));
    }
    Pane {
        title: "Needs your judgment".into(),
        rows,
    }
}

pub fn detail_pane(data: &DecisionsData, focus: Option<&DecisionFocus>) -> Pane {
    let mut rows = Vec::new();
    let Some(focus) = focus else {
        rows.push(Row::muted("Choose a decision from the queue first."));
        return Pane {
            title: "Decision".into(),
            rows,
        };
    };
    let Some((_, request)) = all_requests(data).into_iter().find(|(intent_id, request)| {
        intent_id == &focus.intent_id && request.id == focus.request_id
    }) else {
        rows.push(Row::muted(
            "This request is no longer waiting for a decision.",
        ));
        rows.push(Row::muted("Press Esc to return to the queue."));
        return Pane {
            title: "Decision resolved".into(),
            rows,
        };
    };

    rows.push(Row::heading("What needs you"));
    rows.push(Row::styled(format!("  {}", request.summary), theme::text()));
    rows.push(Row::heading("Why the runtime cannot decide"));
    rows.push(Row::muted(format!("  {}", request.blocked_reason)));
    rows.push(Row::muted(format!(
        "  {} impact · {} · {}",
        loss_label(request.expected_loss),
        if request.blocking {
            "blocks the intent"
        } else {
            "does not block the intent"
        },
        if request.reversible {
            "waiting is reversible"
        } else {
            "waiting is not reversible"
        }
    )));
    if let Some(deadline) = request.decide_by {
        rows.push(Row::muted(format!(
            "  Decide by {}",
            deadline.format("%Y-%m-%d %H:%M UTC")
        )));
    }

    rows.push(Row::heading("Your options"));
    if request.options.is_empty() {
        rows.push(Row::muted(
            "  No predefined options. Press e to write a response.",
        ));
    }
    for option in &request.options {
        let suffix = if option.recommended
            || request.default_option.as_deref() == Some(option.id.as_str())
        {
            "  · recommended"
        } else {
            ""
        };
        rows.push(Row::action(
            format!("  {}{suffix}", option.label),
            if option.recommended {
                theme::focused()
            } else {
                theme::text()
            },
            RowAction::Decide {
                intent_id: focus.intent_id.clone(),
                request_id: request.id.clone(),
                choice: DecisionChoice::Option(option.id.clone()),
            },
        ));
    }
    rows.push(Row::muted("  e  write a custom response"));
    rows.push(Row::muted("  x  dismiss this request"));
    if request.expected_loss >= ExpectedLoss::High {
        rows.push(Row::styled(
            "  Approval requires one more confirmation because the impact is high.",
            theme::warning(),
        ));
    }
    Pane {
        title: "Review before choosing".into(),
        rows,
    }
}

fn loss_label(loss: ExpectedLoss) -> &'static str {
    match loss {
        ExpectedLoss::Low => "low",
        ExpectedLoss::Medium => "medium",
        ExpectedLoss::High => "high",
        ExpectedLoss::Critical => "critical",
    }
}

fn loss_style(loss: ExpectedLoss) -> ratatui::style::Style {
    match loss {
        ExpectedLoss::Low => theme::text(),
        ExpectedLoss::Medium => theme::waiting(),
        ExpectedLoss::High | ExpectedLoss::Critical => theme::critical(),
    }
}
