//! 结构化计划协议。
//!
//! Plan 是 Orchestrator 的可检查建议，不是 Thread，也不拥有执行权限。
//! 外部事实只能通过 Plan Impact 标记受影响步骤；是否取消 Scope 仍由
//! Orchestrator 根据生命周期语义决定。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{PlanId, PlanStepId, ScopeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Planned,
    Ready,
    Running,
    Completed,
    NeedsReplan,
    Blocked,
    Superseded,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEvidenceKind {
    #[serde(alias = "user_intent")]
    UserThread,
    ExternalFact,
    Memory,
    ExecutionObserved,
    PlannerInference,
    SystemDecomposition,
}

/// 计划判断的证据来源。事件 ID 只引用 Ledger 内事实，不复制权限。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanEvidence {
    pub kind: PlanEvidenceKind,
    pub event_id: Option<String>,
    pub summary: String,
}

/// 一个计划步骤。`watch_sources` / `watch_terms` 显式声明它依赖哪些外部事实；
/// Runtime 只据此做确定性影响分析，不猜测未声明依赖。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: PlanStepId,
    pub title: String,
    pub description: String,
    pub scope_id: Option<ScopeId>,
    pub depends_on: Vec<PlanStepId>,
    pub status: PlanStepStatus,
    #[serde(default)]
    pub watch_sources: Vec<String>,
    #[serde(default)]
    pub watch_terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub id: PlanId,
    pub revision: u64,
    pub supersedes: Option<PlanId>,
    #[serde(alias = "intent_revision")]
    pub thread_revision: u64,
    pub summary: String,
    pub steps: Vec<PlanStep>,
    pub evidence: Vec<PlanEvidence>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStepDraft {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub watch_sources: Vec<String>,
    #[serde(default)]
    pub watch_terms: Vec<String>,
}

/// Harness 的轻量 Plan 提案。持久 ID、Revision、Scope 与证据由 Runtime 补全，
/// 避免模型伪造生命周期标识。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanDraft {
    pub summary: String,
    pub steps: Vec<PlanStepDraft>,
}

/// 新事实对当前计划的影响。空 `affected_step_ids` 表示已分析但无需重规划。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanImpact {
    pub plan_id: PlanId,
    pub source_event_id: String,
    pub source: String,
    pub summary: String,
    pub affected_step_ids: Vec<PlanStepId>,
    pub invalidated_scope_ids: Vec<ScopeId>,
    pub unaffected_step_ids: Vec<PlanStepId>,
    pub explanation: String,
    pub created_at: DateTime<Utc>,
}

impl Plan {
    pub fn affected_by(&self, source: &str, summary: &str) -> Vec<PlanStepId> {
        let source = source.to_lowercase();
        let summary = summary.to_lowercase();
        self.steps
            .iter()
            .filter(|step| {
                !matches!(
                    step.status,
                    PlanStepStatus::Completed
                        | PlanStepStatus::Superseded
                        | PlanStepStatus::Cancelled
                )
            })
            .filter(|step| {
                step.watch_sources
                    .iter()
                    .any(|watch| watch.to_lowercase() == source)
                    || step.watch_terms.iter().any(|watch| {
                        let watch = watch.to_lowercase();
                        source.contains(&watch) || summary.contains(&watch)
                    })
            })
            .map(|step| step.id.clone())
            .collect()
    }
}
