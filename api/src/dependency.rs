//! 依赖失败后的自动收敛：一次替换、下游重接线或显式 Superseded。

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use else_ledger::IntentProjection;
use else_protocol::event::{Actor, EventEnvelope, EventPayload, ScopeState};
use else_protocol::ids::{IntentId, PlanId, PlanStepId, ScopeId};
use else_protocol::plan::{PlanEvidence, PlanEvidenceKind, PlanStep, PlanStepStatus};

use crate::{ApiError, Result, Runtime};

impl Runtime {
    /// 处理一个因上游 Failed / Cancelled 而无法调度的 Scope。
    /// Failed 上游可自动替换一次；替换链耗尽或上游被用户取消时，只裁剪下游。
    pub(crate) fn converge_dependency_failure(
        &self,
        intent_id: &IntentId,
        scope_id: &ScopeId,
        dependencies: &[ScopeId],
    ) -> Result<IntentProjection> {
        let mut projection = self.projection(intent_id)?;
        let Some(dependent) = projection.scopes.get(scope_id).cloned() else {
            return Ok(projection);
        };
        if matches!(
            dependent.state,
            ScopeState::Completed
                | ScopeState::Failed
                | ScopeState::Superseded
                | ScopeState::Cancelled
        ) {
            return Ok(projection);
        }

        let mut replacements = BTreeMap::new();
        for dependency in dependencies {
            let Some(failed) = projection.scopes.get(dependency) else {
                continue;
            };
            if failed.state != ScopeState::Failed {
                continue;
            }
            if let Some(existing) = projection.scope_replacements.get(dependency) {
                replacements.insert(dependency.clone(), existing.clone());
                continue;
            }
            if projection
                .scope_replacements
                .values()
                .any(|replacement| replacement == dependency)
            {
                continue;
            }
            match self.spawn_dependency_replacement(intent_id, dependency, failed) {
                Ok(replacement) => {
                    replacements.insert(dependency.clone(), replacement);
                }
                Err(ApiError::Conflict(_)) | Err(ApiError::NotFound(_)) => continue,
                Err(error) => return Err(error),
            }
        }

        // 先把替换接到所有受影响下游，避免其他 Scope 继续等旧节点。
        let updated_projection = self.projection(intent_id)?;
        for (previous, replacement) in &replacements {
            for (dependent_id, scope) in &updated_projection.scopes {
                if scope.depends_on.contains(previous) {
                    self.record_dependency_replacement(
                        intent_id,
                        dependent_id,
                        previous,
                        replacement,
                    )?;
                }
            }
        }

        projection = self.projection(intent_id)?;
        let mut mapped = Vec::new();
        for dependency in dependencies {
            mapped.push(
                projection
                    .scope_replacements
                    .get(dependency)
                    .cloned()
                    .unwrap_or_else(|| dependency.clone()),
            );
        }
        let viable = mapped.iter().all(|dependency| {
            projection.scopes.get(dependency).is_some_and(|scope| {
                !matches!(
                    scope.state,
                    ScopeState::Failed | ScopeState::Superseded | ScopeState::Cancelled
                )
            })
        });
        if !viable {
            let current = projection
                .scopes
                .get(scope_id)
                .cloned()
                .unwrap_or(dependent.clone());
            let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
            self.ledger.append(&EventEnvelope::new(
                intent_id.clone(),
                scope_id.clone(),
                current.generation,
                revision,
                Actor::System,
                EventPayload::ScopeStateChanged {
                    state: ScopeState::Superseded,
                },
            ))?;
            if projection.root_scope_id.as_ref() == Some(scope_id) {
                self.ledger.append(&EventEnvelope::new(
                    intent_id.clone(),
                    scope_id.clone(),
                    current.generation,
                    revision,
                    Actor::System,
                    EventPayload::IntentStateChanged {
                        state: else_protocol::intent::IntentState::Failed,
                    },
                ))?;
            }
        }

        self.record_dependency_replan(intent_id, scope_id, dependencies, &replacements, !viable)?;
        self.projection(intent_id)
    }

    fn spawn_dependency_replacement(
        &self,
        intent_id: &IntentId,
        failed_scope_id: &ScopeId,
        failed: &else_ledger::ScopeProjection,
    ) -> Result<ScopeId> {
        let parent = failed.parent.clone().ok_or_else(|| {
            ApiError::NotFound(format!("parent of failed scope {failed_scope_id}"))
        })?;
        let projection = self.projection(intent_id)?;
        let parent_scope = projection
            .scopes
            .get(&parent)
            .ok_or_else(|| ApiError::NotFound(format!("parent scope {parent}")))?;
        if !matches!(parent_scope.state, ScopeState::Running | ScopeState::Paused) {
            return Err(ApiError::Conflict(format!(
                "parent scope {parent} is not active"
            )));
        }
        let spec = failed
            .spec
            .clone()
            .ok_or_else(|| ApiError::IntentNotFound(intent_id.clone()))?;
        let depends_on = failed
            .depends_on
            .iter()
            .filter(|dependency| *dependency != failed_scope_id)
            .cloned()
            .collect();
        let replacement = ScopeId::new();
        let generation = self
            .events(intent_id)?
            .iter()
            .map(|event| event.envelope.generation)
            .max()
            .unwrap_or(parent_scope.generation)
            .saturating_add(1);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            parent,
            generation,
            spec.revision,
            Actor::System,
            EventPayload::ScopeReplacementSpawned {
                child: replacement.clone(),
                replaces: failed_scope_id.clone(),
                spec,
                depends_on,
            },
        ))?;
        Ok(replacement)
    }

    fn record_dependency_replacement(
        &self,
        intent_id: &IntentId,
        dependent_id: &ScopeId,
        previous: &ScopeId,
        replacement: &ScopeId,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let dependent = projection
            .scopes
            .get(dependent_id)
            .ok_or_else(|| ApiError::NotFound(format!("scope {dependent_id}")))?;
        let revision = projection.spec.as_ref().map_or(1, |spec| spec.revision);
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            dependent_id.clone(),
            dependent.generation,
            revision,
            Actor::System,
            EventPayload::ScopeDependencyReplaced {
                previous: previous.clone(),
                replacement: replacement.clone(),
                reason: "自动替换终态失败的上游 Scope".into(),
            },
        ))?;
        Ok(())
    }

    fn record_dependency_replan(
        &self,
        intent_id: &IntentId,
        dependent_id: &ScopeId,
        dependencies: &[ScopeId],
        replacements: &BTreeMap<ScopeId, ScopeId>,
        superseded: bool,
    ) -> Result<()> {
        let projection = self.projection(intent_id)?;
        let Some(current) = projection.current_plan().cloned() else {
            return Ok(());
        };
        let affected_scopes: BTreeSet<_> = dependencies
            .iter()
            .cloned()
            .chain(std::iter::once(dependent_id.clone()))
            .collect();
        let mut plan = current.clone();
        plan.id = PlanId::new();
        plan.revision = current.revision.saturating_add(1);
        plan.supersedes = Some(current.id.clone());
        plan.created_at = Utc::now();
        plan.summary = format!("依赖失败自动重规划：{}", current.summary);
        plan.evidence.push(PlanEvidence {
            kind: PlanEvidenceKind::ExecutionObserved,
            event_id: None,
            summary: format!(
                "Scope {} 的上游依赖失败，自动{}",
                dependent_id,
                if superseded {
                    "裁剪失效下游"
                } else {
                    "替换失败节点"
                }
            ),
        });

        let mut old_to_new_step = BTreeMap::new();
        for step in &mut plan.steps {
            if step
                .scope_id
                .as_ref()
                .is_some_and(|scope| affected_scopes.contains(scope))
            {
                step.status = if superseded && step.scope_id.as_ref() == Some(dependent_id) {
                    PlanStepStatus::Superseded
                } else {
                    PlanStepStatus::NeedsReplan
                };
            }
        }
        for (previous, replacement) in replacements {
            if let Some(step) = plan
                .steps
                .iter_mut()
                .find(|step| step.scope_id.as_ref() == Some(previous))
            {
                step.status = PlanStepStatus::Superseded;
                let replacement_step = PlanStep {
                    id: PlanStepId::new(),
                    title: format!("替换失败步骤：{}", step.title),
                    description: format!("自动替换 Scope {}，保持原权限边界", previous),
                    scope_id: Some(replacement.clone()),
                    depends_on: step.depends_on.clone(),
                    status: PlanStepStatus::Planned,
                    watch_sources: step.watch_sources.clone(),
                    watch_terms: step.watch_terms.clone(),
                };
                old_to_new_step.insert(step.id.clone(), replacement_step.id.clone());
                plan.steps.push(replacement_step);
            }
        }
        for step in &mut plan.steps {
            for dependency in &mut step.depends_on {
                if let Some(replacement) = old_to_new_step.get(dependency) {
                    *dependency = replacement.clone();
                }
            }
        }
        let (root, generation, revision) = self.kernel_context(intent_id)?;
        self.ledger.append(&EventEnvelope::new(
            intent_id.clone(),
            root,
            generation,
            revision,
            Actor::System,
            EventPayload::StructuredPlanReplaced {
                reason: "依赖失败触发自动重规划".into(),
                plan,
            },
        ))?;
        Ok(())
    }
}
