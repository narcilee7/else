//! 基础投影：从事件重建 Intent 与 Scope 委托树。
//! 投影可丢弃、可重建；UI 与 Decision Queue 只读投影。

use std::collections::BTreeMap;

use else_protocol::artifact::{Artifact, Checkpoint};
use else_protocol::attention::{AttentionRequest, Ruling};
use else_protocol::event::{EventPayload, ScopeState};
use else_protocol::ids::{AttentionId, ScopeId};
use else_protocol::intent::{IntentSpec, IntentState};
use else_protocol::resource::{ResourceBudget, ResourceUsage};

use crate::{Projection, StoredEvent};

/// Scope 的生命周期与委托关系，全部来自事件。
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeProjection {
    pub parent: Option<ScopeId>,
    pub state: ScopeState,
    pub generation: u64,
    /// Subagent 的收窄权限契约；根 Scope 使用 IntentSpec。
    pub spec: Option<IntentSpec>,
}

/// Intent 的基础投影：目标契约、状态、待本人决策、产物、检查点与 Scope 树。
#[derive(Debug, Default, PartialEq)]
pub struct IntentProjection {
    pub spec: Option<IntentSpec>,
    /// Intent 创建时确定的根 Scope。根身份不随 Run、重试或 Revision 改变。
    pub root_scope_id: Option<ScopeId>,
    pub state: Option<IntentState>,
    /// Decision Queue：已请求但未决策的注意力请求。
    pub open_requests: BTreeMap<AttentionId, AttentionRequest>,
    /// Attention Kernel 的持久裁决；没有裁决的请求不能被表面直接打断用户。
    pub attention_rulings: BTreeMap<AttentionId, Ruling>,
    pub artifacts: Vec<Artifact>,
    pub checkpoints: Vec<Checkpoint>,
    pub scopes: BTreeMap<ScopeId, ScopeProjection>,
    pub resource_budget: ResourceBudget,
    pub resource_usage: ResourceUsage,
    /// 已重放到的事件水位。
    pub cursor: u64,
}

impl Projection for IntentProjection {
    fn apply(&mut self, event: &StoredEvent) {
        self.cursor = event.seq;
        match &event.envelope.payload {
            EventPayload::IntentCreated { spec } => {
                self.root_scope_id = Some(event.envelope.scope_id.clone());
                self.scopes.insert(
                    event.envelope.scope_id.clone(),
                    ScopeProjection {
                        parent: None,
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        spec: None,
                    },
                );
                self.spec = Some(spec.clone());
            }
            EventPayload::IntentRevised { spec, .. } => {
                self.spec = Some(spec.clone());
            }
            EventPayload::IntentStateChanged { state } => {
                self.state = Some(*state);
            }
            EventPayload::AttentionRequested { request } => {
                self.open_requests
                    .insert(request.id.clone(), request.clone());
            }
            EventPayload::AttentionRuled { request_id, ruling } => {
                self.attention_rulings
                    .insert(request_id.clone(), ruling.clone());
            }
            EventPayload::DecisionResolved { decision } => {
                self.open_requests.remove(&decision.request_id);
            }
            EventPayload::RunStarted { .. } => {
                let scope = self
                    .scopes
                    .entry(event.envelope.scope_id.clone())
                    .or_insert(ScopeProjection {
                        parent: None,
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        spec: None,
                    });
                scope.state = ScopeState::Running;
                scope.generation = event.envelope.generation;
            }
            EventPayload::ScopeSpawned { child } => {
                self.scopes.insert(
                    child.clone(),
                    ScopeProjection {
                        parent: Some(event.envelope.scope_id.clone()),
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        spec: None,
                    },
                );
            }
            EventPayload::SubagentSpawned { child, spec } => {
                self.scopes.insert(
                    child.clone(),
                    ScopeProjection {
                        parent: Some(event.envelope.scope_id.clone()),
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        spec: Some(spec.clone()),
                    },
                );
            }
            EventPayload::ScopeStateChanged { state } => {
                self.scopes
                    .entry(event.envelope.scope_id.clone())
                    .or_insert(ScopeProjection {
                        parent: None,
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        spec: None,
                    })
                    .state = *state;
            }
            EventPayload::ResourceBudgetSet { budget } => {
                self.resource_budget = *budget;
            }
            EventPayload::ModelCompleted { usage, .. } => {
                self.resource_usage.model_tokens = self
                    .resource_usage
                    .model_tokens
                    .saturating_add(usage.input_tokens.saturating_add(usage.output_tokens));
                self.resource_usage.model_spend_usd_micros = self
                    .resource_usage
                    .model_spend_usd_micros
                    .saturating_add(usage.cost_usd_micros);
            }
            EventPayload::ToolCalled { .. } => {
                self.resource_usage.tool_calls = self.resource_usage.tool_calls.saturating_add(1);
            }
            EventPayload::LeaseGranted { lease }
                if lease.class == else_protocol::resource::ResourceClass::Sandbox =>
            {
                self.resource_usage.sandbox_seconds = self
                    .resource_usage
                    .sandbox_seconds
                    .saturating_add(lease.quantity);
            }
            EventPayload::ArtifactProduced { artifact } => {
                self.artifacts.push(artifact.clone());
            }
            EventPayload::CheckpointCreated { checkpoint } => {
                self.checkpoints.push(checkpoint.clone());
            }
            _ => {}
        }
    }
}

impl IntentProjection {
    /// 用户表面只展示已被 Kernel 裁决为可行动、且 Defer 已到期的请求。
    /// 没有裁决记录的请求失败关闭，不能绕过 Attention Kernel 直接打扰用户。
    pub fn actionable_requests(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Vec<&AttentionRequest> {
        self.open_requests
            .values()
            .filter(|request| match self.attention_rulings.get(&request.id) {
                Some(Ruling::Interrupt | Ruling::Batch) => true,
                Some(Ruling::Defer { until }) => *until <= now,
                Some(Ruling::Silent) | None => false,
            })
            .collect()
    }
}
