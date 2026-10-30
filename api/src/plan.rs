//! 结构化 Plan、重规划版本与外部事实影响分析。

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::{IntentId, PlanId, PlanStepId, ScopeId};
use else_protocol::intent::IntentSpec;
use else_protocol::plan::{
    Plan, PlanEvidence, PlanEvidenceKind, PlanImpact, PlanStep, PlanStepStatus,
};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// Planner / Harness 提交结构化计划。调用方只能建议计划，不能借此改变
    /// Intent、Scope 生命周期或权限。
    pub fn propose_plan(&self, intent_id: &IntentId, plan: Plan) -> Result<Plan> {
        let projection = self.projection(intent_id)?;
        let spec = projection
            .spec
            .as_ref()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        if plan.intent_revision != spec.revision {
            return Err(ApiError::Conflict(format!(
                "plan revision {} does not match intent revision {}",
                plan.intent_revision, spec.revision
            )));
        }
        if let Some(current) = projection.current_plan() {
            if plan.revision <= current.revision {
                return Err(ApiError::Conflict(format!(
                    "plan revision {} must advance current revision {}",
                    plan.revision, current.revision
                )));
            }
        }
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        let payload = if projection.current_plan().is_some() {
            EventPayload::StructuredPlanReplaced {
                reason: format!("Planner 提交计划 revision {}", plan.revision),
                plan: plan.clone(),
            }
        } else {
            EventPayload::StructuredPlanProposed { plan: plan.clone() }
        };
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id,
            generation,
            revision,
            Actor::Agent,
            payload,
        ))?;
        Ok(plan)
    }

    pub fn plan_history(&self, intent_id: &IntentId) -> Result<Vec<Plan>> {
        let projection = self.projection(intent_id)?;
        Ok(projection
            .plan_order
            .iter()
            .filter_map(|id| projection.plans.get(id).cloned())
            .collect())
    }

    /// 分析外部事实对当前 Plan 的影响，并把结论作为事件持久化。
    /// 没有显式监听条件的步骤不会被猜测为受影响。
    pub fn analyze_plan_impact(
        &self,
        intent_id: &IntentId,
        source_event_id: &str,
        source: &str,
        summary: &str,
    ) -> Result<Option<PlanImpact>> {
        let projection = self.projection(intent_id)?;
        let Some(plan) = projection.current_plan().cloned() else {
            return Ok(None);
        };
        let affected_step_ids = plan.affected_by(source, summary);
        let affected: BTreeSet<_> = affected_step_ids.iter().cloned().collect();
        let unaffected_step_ids = plan
            .steps
            .iter()
            .filter(|step| !affected.contains(&step.id))
            .map(|step| step.id.clone())
            .collect();
        let invalidated_scope_ids: Vec<ScopeId> = plan
            .steps
            .iter()
            .filter(|step| affected.contains(&step.id))
            .filter_map(|step| step.scope_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let explanation = if affected_step_ids.is_empty() {
            "新事实未命中计划步骤声明的 source / term 依赖，未自动使任何 Scope 失效".into()
        } else {
            format!(
                "新事实命中 {} 个计划步骤，建议由 Orchestrator 评估 {} 个受影响 Scope",
                affected_step_ids.len(),
                invalidated_scope_ids.len()
            )
        };
        let impact = PlanImpact {
            plan_id: plan.id.clone(),
            source_event_id: source_event_id.into(),
            source: source.into(),
            summary: summary.into(),
            affected_step_ids,
            invalidated_scope_ids,
            unaffected_step_ids,
            explanation,
            created_at: Utc::now(),
        };
        let (scope_id, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            scope_id,
            generation,
            revision,
            Actor::System,
            EventPayload::PlanImpactAnalyzed {
                impact: impact.clone(),
            },
        ))?;
        Ok(Some(impact))
    }

    /// 真实 Harness 运行前保证当前 Plan 至少覆盖目标 Scope。该计划显式标记为
    /// 系统分解，不冒充模型推理；已有 Planner 计划会保留其步骤和证据。
    pub(crate) fn ensure_plan_for_scope(
        &self,
        intent_id: &IntentId,
        spec: &IntentSpec,
        scope_id: &ScopeId,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let current = projection.current_plan().cloned();
        if current.as_ref().is_some_and(|plan| {
            plan.steps
                .iter()
                .any(|step| step.scope_id == Some(scope_id.clone()))
        }) {
            return Ok(());
        }

        let mut steps = current
            .as_ref()
            .map(|plan| plan.steps.clone())
            .unwrap_or_default();
        let mut scope_to_step: BTreeMap<_, _> = steps
            .iter()
            .filter_map(|step| step.scope_id.clone().map(|scope| (scope, step.id.clone())))
            .collect();
        for (scope, info) in &projection.scopes {
            if steps
                .iter()
                .any(|step| step.scope_id.as_ref() == Some(scope))
            {
                continue;
            }
            let outcome = projection
                .scopes
                .get(scope)
                .and_then(|scope| scope.spec.as_ref())
                .map(|spec| spec.outcome.clone())
                .unwrap_or_else(|| spec.outcome.clone());
            let step_id = PlanStepId::new();
            let depends_on = info
                .depends_on
                .iter()
                .filter_map(|dependency| scope_to_step.get(dependency).cloned())
                .collect();
            steps.push(PlanStep {
                id: step_id.clone(),
                title: outcome,
                description: format!("在 Scope {scope} 中推进并提交独立验收证据"),
                scope_id: Some(scope.clone()),
                depends_on,
                status: PlanStepStatus::Planned,
                watch_sources: vec![],
                watch_terms: vec![],
            });
            scope_to_step.insert(scope.clone(), step_id);
        }
        if steps.is_empty() {
            return Ok(());
        }
        let revision = current.as_ref().map_or(1, |plan| plan.revision + 1);
        let plan = Plan {
            id: PlanId::new(),
            revision,
            supersedes: current.as_ref().map(|plan| plan.id.clone()),
            intent_revision: spec.revision,
            summary: current
                .as_ref()
                .map(|plan| plan.summary.clone())
                .unwrap_or_else(|| format!("系统分解：{}", spec.outcome)),
            steps,
            evidence: vec![PlanEvidence {
                kind: PlanEvidenceKind::SystemDecomposition,
                event_id: None,
                summary: "Orchestrator 为运行 Scope 补齐可检查计划覆盖".into(),
            }],
            created_at: Utc::now(),
        };
        let (root_scope, generation, intent_revision) = self.kernel_context(intent_id)?;
        let payload = if current.is_some() {
            EventPayload::StructuredPlanReplaced {
                plan,
                reason: format!("补齐 Scope {scope_id} 的计划覆盖"),
            }
        } else {
            EventPayload::StructuredPlanProposed { plan }
        };
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root_scope,
            generation,
            intent_revision,
            Actor::System,
            payload,
        ))?;
        Ok(())
    }
}
