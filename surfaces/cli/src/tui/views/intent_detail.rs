//! Intent 详情：默认展示目标、下一步、计划与证据；调试信息显式展开。

use else_protocol::plan::PlanStepStatus;

use super::super::data::IntentDetailData;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;
use super::{intent_state_label, intent_style, results, short_id};

pub fn pane(detail: &IntentDetailData, debug: bool) -> Pane {
    let mut rows = Vec::new();
    let intent = &detail.intent;

    rows.push(Row::styled(
        format!("  {}", intent.spec.outcome),
        theme::heading(),
    ));
    rows.push(Row::styled(
        format!("  {}", intent_state_label(intent.state)),
        intent_style(intent.state),
    ));
    if let Some(project) = intent.spec.project.as_deref() {
        rows.push(Row::muted(format!("  Project · {project}")));
    }
    if !intent.spec.constraints.is_empty() {
        rows.push(Row::heading("Boundaries"));
        for constraint in &intent.spec.constraints {
            rows.push(Row::muted(format!("  • {constraint}")));
        }
    }

    if let Some(request) = intent.decisions.first() {
        rows.push(Row::heading("Next step"));
        rows.push(Row::action(
            format!("  Review · {}", request.summary),
            theme::critical(),
            RowAction::OpenDecision {
                intent_id: intent.spec.id.clone(),
                request_id: request.id.clone(),
            },
        ));
    } else if !matches!(
        intent.state,
        else_protocol::intent::IntentState::Completed
            | else_protocol::intent::IntentState::Cancelled
            | else_protocol::intent::IntentState::Failed
    ) {
        rows.push(Row::heading("Next step"));
        rows.push(Row::action(
            "  Advance this intent",
            theme::focused(),
            RowAction::AdvanceIntent(intent.spec.id.clone()),
        ));
    }

    rows.extend(results::acceptance_rows(intent));

    rows.push(Row::heading("Plan"));
    match detail.plan.current.as_ref() {
        Some(plan) => {
            rows.push(Row::styled(format!("  {}", plan.summary), theme::text()));
            for step in &plan.steps {
                rows.push(Row::muted(format!(
                    "  {}  {}",
                    step_status_label(step.status),
                    step.title
                )));
            }
        }
        None => rows.push(Row::muted("  No plan has been produced yet")),
    }

    if debug {
        rows.push(Row::heading("Debug · Scopes"));
        if detail.scopes.is_empty() {
            rows.push(Row::muted("  No scopes"));
        }
        for scope in &detail.scopes {
            rows.push(Row::muted(format!(
                "  {}  {:?}  generation={} failures={}",
                scope.scope_id, scope.state, scope.generation, scope.failure_count
            )));
            rows.push(Row::muted(format!("      {}", scope.outcome)));
        }
        rows.push(Row::heading("Debug · Resources"));
        rows.push(Row::muted(format!(
            "  tokens {}/{} · tool calls {}/{} · sandbox {}s/{}s",
            detail.resources.usage.model_tokens,
            detail.resources.budget.max_model_tokens,
            detail.resources.usage.tool_calls,
            detail.resources.budget.max_tool_calls,
            detail.resources.usage.sandbox_seconds,
            detail.resources.budget.max_sandbox_seconds
        )));
        rows.push(Row::heading("Debug · Events"));
        let start = detail.events.len().saturating_sub(12);
        if detail.events.is_empty() {
            rows.push(Row::muted("  No events"));
        }
        for event in &detail.events[start..] {
            rows.push(Row::muted(format!(
                "  #{} {} {}",
                event.seq, event.scope_id, event.event_id
            )));
        }
        rows.push(Row::muted(format!(
            "  intent={} revision={} cursor={}",
            short_id(&intent.spec.id),
            intent.spec.revision,
            intent.cursor
        )));
    } else {
        rows.push(Row::muted(
            "  d  show scopes, resources and event debug details",
        ));
    }

    Pane {
        title: format!("Intent · {}", short_id(&intent.spec.id)),
        rows,
    }
}

fn step_status_label(status: PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Planned => "○",
        PlanStepStatus::Ready => "→",
        PlanStepStatus::Running => "…",
        PlanStepStatus::Completed => "✓",
        PlanStepStatus::NeedsReplan => "!",
        PlanStepStatus::Blocked => "×",
        PlanStepStatus::Superseded => "—",
        PlanStepStatus::Cancelled => "—",
    }
}
