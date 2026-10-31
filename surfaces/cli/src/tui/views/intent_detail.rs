//! Intent 详情：目标、委托树、计划、资源账与事件流。

use super::super::data::IntentDetailData;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

/// Intent 详情。
pub fn pane(detail: &IntentDetailData) -> Pane {
    let mut rows = Vec::new();
    let intent = &detail.intent;

    rows.push(Row::heading("Outcome"));
    rows.push(Row::styled(
        format!("  {}", intent.spec.outcome),
        theme::text(),
    ));
    if let Some(project) = intent.spec.project.as_deref() {
        rows.push(Row::muted(format!("  project: {project}")));
    }
    rows.push(Row::muted(format!(
        "  state={:?} revision={} cursor={}",
        intent.state, intent.spec.revision, intent.cursor
    )));
    if !intent.spec.constraints.is_empty() {
        rows.push(Row::muted(format!(
            "  constraints: {}",
            intent.spec.constraints.join(" / ")
        )));
    }

    rows.push(Row::heading("Scopes"));
    if detail.scopes.is_empty() {
        rows.push(Row::muted("  no scopes"));
    }
    for scope in &detail.scopes {
        let style = match scope.state {
            else_protocol::event::ScopeState::Completed => theme::done(),
            else_protocol::event::ScopeState::Failed => theme::critical(),
            else_protocol::event::ScopeState::Fenced
            | else_protocol::event::ScopeState::Superseded => theme::muted(),
            _ => theme::text(),
        };
        rows.push(Row::styled(
            format!(
                "  {}  {:?}  generation={} failures={}",
                scope.scope_id, scope.state, scope.generation, scope.failure_count
            ),
            style,
        ));
        rows.push(Row::muted(format!("      {}", scope.outcome)));
    }

    rows.push(Row::heading("Plan"));
    match detail.plan.current.as_ref() {
        Some(plan) => {
            rows.push(Row::styled(
                format!("  {}  {}", plan.id, plan.summary),
                theme::text(),
            ));
            for step in &plan.steps {
                rows.push(Row::muted(format!(
                    "    {}  {:?}  {}",
                    step.id, step.status, step.title
                )));
            }
        }
        None => rows.push(Row::muted("  no plan yet")),
    }

    rows.push(Row::heading("Resources"));
    rows.push(Row::muted(format!(
        "  tokens {}/{}   tool calls {}/{}   sandbox {}s/{:.0}s",
        detail.resources.usage.model_tokens,
        detail.resources.budget.max_model_tokens,
        detail.resources.usage.tool_calls,
        detail.resources.budget.max_tool_calls,
        detail.resources.usage.sandbox_seconds,
        detail.resources.budget.max_sandbox_seconds as f64
    )));

    rows.push(Row::heading("Events"));
    let start = detail.events.len().saturating_sub(12);
    for event in &detail.events[start..] {
        rows.push(Row::muted(format!(
            "  #{} {} {}",
            event.seq, event.scope_id, event.event_id
        )));
    }

    rows.push(Row::action(
        "  press a to advance this intent",
        theme::hint(),
        RowAction::AdvanceIntent(intent.spec.id.clone()),
    ));

    Pane {
        title: format!("Intent {}", intent.spec.id),
        rows,
    }
}
