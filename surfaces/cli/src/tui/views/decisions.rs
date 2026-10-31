//! Decisions：真正需要你裁决的事。
//!
//! Batch 与 Defer 窗口在这里被**摊平**成一条队列：窗口是调度机制，
//! 不是用户要理解的东西。用户看到的是「有几件事在等我」。

use else_protocol::attention::DecisionChoice;

use super::super::data::DecisionsData;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

/// 决策队列。
pub fn pane(data: &DecisionsData) -> Pane {
    let mut rows = Vec::new();
    let mut count = 0usize;
    for window in &data.windows {
        for request in &window.requests {
            count += 1;
            let label = match window.window.kind {
                else_protocol::attention::AttentionWindowKind::Batch => "batch",
                else_protocol::attention::AttentionWindowKind::Defer => "deferred",
            };
            rows.push(Row::action(
                format!(
                    "  [{}] {}  ({label} window {})",
                    request.id, request.summary, window.window.id
                ),
                theme::critical(),
                RowAction::Decide {
                    intent_id: window.intent_id.clone(),
                    request_id: request.id.clone(),
                    choice: DecisionChoice::Option("approve".into()),
                },
            ));
            rows.push(Row::muted(format!(
                "      why: {}   loss={:?}",
                request.blocked_reason, request.expected_loss
            )));
        }
    }
    for (intent_id, request) in &data.loose {
        count += 1;
        rows.push(Row::action(
            format!("  [{}] {}", request.id, request.summary),
            theme::critical(),
            RowAction::Decide {
                intent_id: intent_id.clone(),
                request_id: request.id.clone(),
                choice: DecisionChoice::Option("approve".into()),
            },
        ));
        rows.push(Row::muted(format!(
            "      why: {}   loss={:?}",
            request.blocked_reason, request.expected_loss
        )));
    }
    if count == 0 {
        rows.push(Row::muted("nothing needs you right now"));
    } else {
        rows.push(Row::muted(format!(
            "{count} request(s) — press a to approve, r to reject, s to defer"
        )));
    }
    Pane {
        title: format!("Decisions ({count})"),
        rows,
    }
}
