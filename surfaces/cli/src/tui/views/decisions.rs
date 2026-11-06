//! 决策队列与单条复核。
//!
//! 队列是一张"现在还挂着几件事"的清单；真正的选择在复核里发生，而那一步
//! 只有一屏：为什么需要你、你的选项、键。

use else_protocol::attention::{AttentionRequest, DecisionChoice, ExpectedLoss};
use else_protocol::ids::{AttentionId, ThreadId};

use super::super::app::DecisionFocus;
use super::super::data::{Data, DecisionsData};
use super::super::row::{Row, RowAction};
use super::{loss_label, loss_style, short_id};

pub fn all_requests(data: &DecisionsData) -> Vec<(ThreadId, AttentionRequest)> {
    let mut requests = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for window in &data.windows {
        for request in &window.requests {
            if seen.insert(request.id.clone()) {
                requests.push((window.thread_id.clone(), request.clone()));
            }
        }
    }
    for (thread_id, request) in &data.loose {
        if seen.insert(request.id.clone()) {
            requests.push((thread_id.clone(), request.clone()));
        }
    }
    requests
}

/// 找一个待决请求。决策既可能从队列点开，也可能在 Thread 时间线上直接选，
/// 所以两种数据都要能回答"这一条的预期损失有多大" —— 高风险批准要靠它
/// 触发二次确认。
pub fn find_request<'a>(data: &'a Data, request_id: &AttentionId) -> Option<&'a AttentionRequest> {
    match data {
        Data::Decisions(data) => data
            .windows
            .iter()
            .flat_map(|window| window.requests.iter())
            .chain(data.loose.iter().map(|(_, request)| request))
            .find(|request| &request.id == request_id),
        Data::ThreadDetail(detail) => detail
            .thread
            .decisions
            .iter()
            .find(|request| &request.id == request_id),
        _ => None,
    }
}

pub fn rows(data: &DecisionsData) -> Vec<Row> {
    let requests = all_requests(data);
    if requests.is_empty() {
        return vec![
            Row::blank(),
            Row::muted(" Nothing is waiting for your judgment."),
            Row::muted(" New requests will show up here without interrupting you."),
        ];
    }
    let mut rows = vec![Row::heading(" WAITING ON YOU")];
    for (thread_id, request) in requests {
        rows.push(
            Row::action(
                format!(" {}", request.summary),
                loss_style(request.expected_loss),
                RowAction::OpenDecision {
                    thread_id: thread_id.clone(),
                    request_id: request.id.clone(),
                },
            )
            .with_meta(format!(
                "{} · {}",
                loss_label(request.expected_loss),
                short_id(&thread_id)
            ))
            .with_note(request.blocked_reason.clone()),
        );
    }
    rows
}

pub fn detail_rows(data: &DecisionsData, focus: Option<&DecisionFocus>) -> Vec<Row> {
    let Some(focus) = focus else {
        return vec![Row::muted(" Pick a request from the queue first.")];
    };
    let Some((_, request)) = all_requests(data).into_iter().find(|(thread_id, request)| {
        thread_id == &focus.thread_id && request.id == focus.request_id
    }) else {
        return vec![
            Row::blank(),
            Row::muted(" This request is no longer waiting for a decision."),
            Row::muted(" Esc goes back to the queue."),
        ];
    };

    let mut rows = vec![
        Row::styled(
            format!(" {}", request.summary),
            loss_style(request.expected_loss),
        ),
        Row::blank(),
        Row::muted(format!(" {}", request.blocked_reason)),
        Row::muted(format!(
            " {} impact · {} · {}",
            loss_label(request.expected_loss),
            if request.blocking {
                "blocks the thread"
            } else {
                "does not block the thread"
            },
            if request.reversible {
                "waiting is reversible"
            } else {
                "waiting is not reversible"
            }
        )),
    ];
    if let Some(deadline) = request.decide_by {
        rows.push(Row::muted(format!(
            " decide by {}",
            deadline.format("%m-%d %H:%M")
        )));
    }
    rows.push(Row::blank());

    rows.push(Row::heading(" CHOOSE ONE"));
    if request.options.is_empty() {
        rows.push(Row::muted(" no predefined option · e writes one"));
    }
    for option in &request.options {
        let recommended =
            option.recommended || request.default_option.as_deref() == Some(option.id.as_str());
        rows.push(Row::action(
            format!(
                " ▸ {}{}",
                option.label,
                if recommended { " (recommended)" } else { "" }
            ),
            if recommended {
                super::super::theme::focused()
            } else {
                super::super::theme::text()
            },
            RowAction::Decide {
                thread_id: focus.thread_id.clone(),
                request_id: request.id.clone(),
                choice: DecisionChoice::Option(option.id.clone()),
            },
        ));
    }
    if request.expected_loss >= ExpectedLoss::High {
        rows.push(Row::blank());
        rows.push(Row::styled(
            " High impact: approving asks you to confirm once more.",
            super::super::theme::warning(),
        ));
    }
    rows
}
