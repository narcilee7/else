//! 结构化 Plan：版本、证据、外部事实影响与重放一致性。

use chrono::Utc;
use else_api::Runtime;
use else_protocol::attention::DecisionChoice;
use else_protocol::effect::SideEffectKind;
use else_protocol::ids::{PlanId, PlanStepId};
use else_protocol::intent::Criterion;
use else_protocol::plan::{Plan, PlanEvidence, PlanEvidenceKind, PlanStep, PlanStepStatus};

fn step(title: &str, scope_id: else_protocol::ids::ScopeId) -> PlanStep {
    PlanStep {
        id: PlanStepId::new(),
        title: title.into(),
        description: format!("在 {scope_id} 中完成 {title}"),
        scope_id: Some(scope_id),
        depends_on: vec![],
        status: PlanStepStatus::Planned,
        watch_sources: vec!["mail".into()],
        watch_terms: vec!["合作方回复".into()],
    }
}

#[test]
fn external_fact_invalidates_only_declared_plan_steps_and_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_intent_with_done_when(
            "完成发布准备",
            None,
            vec![Criterion::file_exists("计划存在", "plan.md")],
        )
        .unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let affected_scope = runtime
        .spawn_subagent(
            &spec.id,
            &root,
            "等待合作方回复后更新发布说明",
            SideEffectKind::ReadOnly,
        )
        .unwrap();
    let unaffected_scope = runtime
        .spawn_subagent(
            &spec.id,
            &root,
            "整理本地测试结果",
            SideEffectKind::ReadOnly,
        )
        .unwrap();
    let affected_step = step("更新发布说明", affected_scope.clone());
    let mut unaffected_step = step("整理测试结果", unaffected_scope.clone());
    unaffected_step.watch_sources.clear();
    unaffected_step.watch_terms.clear();
    let affected_id = affected_step.id.clone();
    let unaffected_id = unaffected_step.id.clone();
    let plan = Plan {
        id: PlanId::new(),
        revision: 1,
        supersedes: None,
        intent_revision: spec.revision,
        summary: "两个独立分支，只有发布说明依赖合作方回复".into(),
        steps: vec![affected_step, unaffected_step],
        evidence: vec![PlanEvidence {
            kind: PlanEvidenceKind::UserIntent,
            event_id: None,
            summary: spec.outcome.clone(),
        }],
        created_at: Utc::now(),
    };
    runtime.propose_plan(&spec.id, plan).unwrap();

    runtime
        .record_external_event(&spec.id, "mail", "合作方回复：发布说明需更新")
        .unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    let plan = projection.current_plan().unwrap();
    assert_eq!(
        plan.steps
            .iter()
            .find(|step| step.id == affected_id)
            .unwrap()
            .status,
        PlanStepStatus::NeedsReplan
    );
    assert_eq!(
        plan.steps
            .iter()
            .find(|step| step.id == unaffected_id)
            .unwrap()
            .status,
        PlanStepStatus::Planned
    );
    let impact = projection.plan_impacts.last().unwrap();
    assert_eq!(impact.affected_step_ids, vec![affected_id.clone()]);
    assert_eq!(impact.invalidated_scope_ids, vec![affected_scope.clone()]);
    assert!(impact.unaffected_step_ids.contains(&unaffected_id));
    assert_eq!(
        projection.scopes[&unaffected_scope].state,
        else_protocol::event::ScopeState::Running,
        "Plan Impact 是 Orchestrator 的建议，不得自行取消无关 Scope"
    );

    drop(runtime);
    let runtime = Runtime::open(dir.path()).unwrap();
    let replayed = runtime.projection(&spec.id).unwrap();
    assert_eq!(replayed.current_plan(), projection.current_plan());
    assert_eq!(replayed.plan_impacts, projection.plan_impacts);
}

#[test]
fn reject_creates_a_replayable_plan_revision_and_keeps_original_done_when() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("验证自动重规划", None).unwrap();
    let projection = runtime.run(&spec.id).unwrap();
    let request = projection.open_requests.keys().next().unwrap().clone();
    runtime
        .decide(&spec.id, &request, DecisionChoice::Option("reject".into()))
        .unwrap();

    let history = runtime.plan_history(&spec.id).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].revision, 1);
    assert_eq!(history[1].revision, 2);
    assert_eq!(history[1].supersedes, Some(history[0].id.clone()));
    assert!(history[0]
        .steps
        .iter()
        .all(|step| step.status == PlanStepStatus::Superseded));

    let snapshot = runtime
        .compile_snapshot(&spec.id, else_protocol::memory::SnapshotAudience::Planner)
        .unwrap();
    assert_eq!(snapshot.done_when, spec.done_when);
    assert_eq!(snapshot.plan.unwrap().revision, 2);
    let verifier = runtime
        .compile_snapshot(&spec.id, else_protocol::memory::SnapshotAudience::Verifier)
        .unwrap();
    assert!(verifier.plan.is_none());
}

#[test]
fn external_fact_triggers_replan_before_the_next_execution_continues() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_intent_with_done_when(
            "根据合作方回复更新计划",
            None,
            vec![Criterion::file_exists("计划存在", "plan.md")],
        )
        .unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let mut watched = step("等待合作方回复", root);
    watched.watch_sources = vec!["mail".into()];
    watched.watch_terms = vec!["合作方".into()];
    let plan = Plan {
        id: PlanId::new(),
        revision: 1,
        supersedes: None,
        intent_revision: spec.revision,
        summary: "先等待合作方事实".into(),
        steps: vec![watched],
        evidence: vec![],
        created_at: Utc::now(),
    };
    runtime.propose_plan(&spec.id, plan).unwrap();
    std::fs::write(dir.path().join("workspace/plan.md"), "planned").unwrap();

    let waiting = runtime.run(&spec.id).unwrap();
    assert_eq!(
        waiting.state,
        Some(else_protocol::intent::IntentState::NeedsDecision)
    );
    let request = waiting.open_requests.keys().next().unwrap().clone();
    runtime
        .record_external_event(&spec.id, "mail", "合作方补充了新的验收要求")
        .unwrap();
    let completed = runtime
        .decide(&spec.id, &request, DecisionChoice::Option("approve".into()))
        .unwrap();
    assert_eq!(
        completed.state,
        Some(else_protocol::intent::IntentState::Completed)
    );
    let history = runtime.plan_history(&spec.id).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].revision, 2);
    assert!(history[1]
        .evidence
        .iter()
        .any(|evidence| evidence.kind == PlanEvidenceKind::ExternalFact));
}
