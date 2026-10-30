//! 基础投影：从事件重建 Intent 与 Scope 委托树。
//! 投影可丢弃、可重建；UI 与 Decision Queue 只读投影。

use std::collections::BTreeMap;

use else_protocol::artifact::{Artifact, Checkpoint};
use else_protocol::attention::{
    AttentionRequest, AttentionWindow, AttentionWindowKind, Decision, Ruling,
};
use else_protocol::event::{Actor, EventPayload, ScopeState};
use else_protocol::ids::{ArtifactId, AttentionId, AttentionWindowId, LeaseId, ScopeId, TimerId};
use else_protocol::intent::{IntentSpec, IntentState};
use else_protocol::plan::{Plan, PlanImpact, PlanStepStatus};
use else_protocol::resource::{ResourceBudget, ResourceUsage, SandboxUsage, SandboxUsageBasis};

use crate::{Projection, StoredEvent};

/// Scope 的生命周期与委托关系，全部来自事件。
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeProjection {
    pub parent: Option<ScopeId>,
    pub state: ScopeState,
    pub generation: u64,
    /// 最近一次 RunStarted 的事件水位，用于跨 Scope 公平调度。
    pub last_run_started_seq: Option<u64>,
    pub depends_on: Vec<ScopeId>,
    /// Subagent 的收窄权限契约；根 Scope 使用 IntentSpec。
    pub spec: Option<IntentSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerProjection {
    pub id: TimerId,
    pub fire_at: chrono::DateTime<chrono::Utc>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEventProjection {
    pub event_id: String,
    pub seq: u64,
    pub source: String,
    pub summary: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

/// 注意力请求的完整审计记录。Decision 会关闭请求，但记录仍用于指标与解释。
#[derive(Debug, Clone, PartialEq)]
pub struct AttentionRecordProjection {
    pub request: AttentionRequest,
    pub ruling: Option<Ruling>,
    pub decision: Option<Decision>,
}

/// 从事件投影计算的注意力成本指标。误打断率只统计用户明确 Dismiss 的
/// Interrupt 请求，不用拒绝选项猜测。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AttentionMetrics {
    pub requested: u64,
    pub resolved: u64,
    pub interventions: u64,
    pub batch_notifications: u64,
    pub defer_notifications: u64,
    pub interrupt_notifications: u64,
    pub batched_requests: u64,
    pub deferred_requests: u64,
    pub silent_requests: u64,
    pub open_critical: u64,
    pub missed_critical: u64,
    pub resolved_interrupts: u64,
    pub dismissed_interrupts: u64,
    pub batch_rate: Option<f64>,
    pub misinterrupt_rate: Option<f64>,
}

/// Intent 的基础投影：目标契约、状态、待本人决策、产物、检查点与 Scope 树。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IntentProjection {
    pub spec: Option<IntentSpec>,
    /// Intent 创建时确定的根 Scope。根身份不随 Run、重试或 Revision 改变。
    pub root_scope_id: Option<ScopeId>,
    pub state: Option<IntentState>,
    /// Decision Queue：已请求但未决策的注意力请求。
    pub open_requests: BTreeMap<AttentionId, AttentionRequest>,
    /// Attention Kernel 的持久裁决；没有裁决的请求不能被表面直接打断用户。
    pub attention_rulings: BTreeMap<AttentionId, Ruling>,
    /// 请求与裁决的完整历史，用于重放后的注意力指标。
    pub attention_records: BTreeMap<AttentionId, AttentionRecordProjection>,
    /// Batch / Defer 的持久调度窗口。
    pub attention_windows: BTreeMap<AttentionWindowId, AttentionWindow>,
    /// 结构化计划版本，按版本 ID 保存；平面顺序用于审计。
    pub plans: BTreeMap<else_protocol::ids::PlanId, Plan>,
    pub plan_order: Vec<else_protocol::ids::PlanId>,
    pub current_plan_id: Option<else_protocol::ids::PlanId>,
    pub plan_impacts: Vec<PlanImpact>,
    pub artifacts: Vec<Artifact>,
    pub artifact_scopes: BTreeMap<ArtifactId, ScopeId>,
    pub checkpoints: Vec<Checkpoint>,
    pub scopes: BTreeMap<ScopeId, ScopeProjection>,
    /// Scope 的创建顺序，用于打破首次调度时的 ID 随机性。
    pub scope_order: Vec<ScopeId>,
    pub timers: BTreeMap<TimerId, TimerProjection>,
    pub external_events: Vec<ExternalEventProjection>,
    pub external_wait_reason: Option<String>,
    pub resource_budget: ResourceBudget,
    pub resource_usage: ResourceUsage,
    /// Sandbox 实际生命周期；Key 为 Lease，便于 Compare 对账预留与实际。
    pub sandbox_usages: BTreeMap<LeaseId, SandboxUsage>,
    active_sandbox_leases: BTreeMap<LeaseId, u64>,
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
                        last_run_started_seq: None,
                        depends_on: vec![],
                        spec: None,
                    },
                );
                self.scope_order.push(event.envelope.scope_id.clone());
                self.spec = Some(spec.clone());
            }
            EventPayload::IntentRevised { spec, .. } => {
                self.spec = Some(spec.clone());
            }
            EventPayload::IntentStateChanged { state } => {
                self.state = Some(*state);
            }
            EventPayload::StructuredPlanProposed { plan } => {
                self.plans.insert(plan.id.clone(), plan.clone());
                self.plan_order.push(plan.id.clone());
                self.current_plan_id = Some(plan.id.clone());
            }
            EventPayload::StructuredPlanReplaced { plan, .. } => {
                if let Some(previous) = self.current_plan_id.as_ref() {
                    if let Some(previous) = self.plans.get_mut(previous) {
                        for step in &mut previous.steps {
                            if !matches!(step.status, PlanStepStatus::Completed) {
                                step.status = PlanStepStatus::Superseded;
                            }
                        }
                    }
                }
                self.plans.insert(plan.id.clone(), plan.clone());
                self.plan_order.push(plan.id.clone());
                self.current_plan_id = Some(plan.id.clone());
            }
            EventPayload::PlanImpactAnalyzed { impact } => {
                self.plan_impacts.push(impact.clone());
                if let Some(plan) = self.plans.get_mut(&impact.plan_id) {
                    for step in &mut plan.steps {
                        if impact.affected_step_ids.contains(&step.id)
                            && !matches!(
                                step.status,
                                PlanStepStatus::Completed
                                    | PlanStepStatus::Superseded
                                    | PlanStepStatus::Cancelled
                            )
                        {
                            step.status = PlanStepStatus::NeedsReplan;
                        }
                    }
                }
            }
            EventPayload::AttentionRequested { request } => {
                self.open_requests
                    .insert(request.id.clone(), request.clone());
                self.attention_records.insert(
                    request.id.clone(),
                    AttentionRecordProjection {
                        request: request.clone(),
                        ruling: None,
                        decision: None,
                    },
                );
            }
            EventPayload::AttentionRuled { request_id, ruling } => {
                self.attention_rulings
                    .insert(request_id.clone(), ruling.clone());
                if let Some(record) = self.attention_records.get_mut(request_id) {
                    record.ruling = Some(ruling.clone());
                }
            }
            EventPayload::AttentionWindowOpened { window } => {
                self.attention_windows
                    .insert(window.id.clone(), window.clone());
            }
            EventPayload::AttentionWindowMemberAdded {
                window_id,
                request_id,
            } => {
                if let Some(window) = self.attention_windows.get_mut(window_id) {
                    if !window.request_ids.contains(request_id) {
                        window.request_ids.push(request_id.clone());
                    }
                }
            }
            EventPayload::AttentionWindowClosed {
                window_id,
                closed_at,
                delivered_at,
            } => {
                if let Some(window) = self.attention_windows.get_mut(window_id) {
                    window.closed_at.get_or_insert(*closed_at);
                    if window.delivered_at.is_none() {
                        window.delivered_at = *delivered_at;
                    }
                }
            }
            EventPayload::DecisionResolved { decision } => {
                self.open_requests.remove(&decision.request_id);
                if let Some(record) = self.attention_records.get_mut(&decision.request_id) {
                    record.decision = Some(decision.clone());
                }
            }
            EventPayload::ExternalEventReceived { source, summary } => {
                self.external_events.push(ExternalEventProjection {
                    event_id: event.envelope.event_id.clone(),
                    seq: event.seq,
                    source: source.clone(),
                    summary: summary.clone(),
                    occurred_at: event.envelope.occurred_at,
                });
                self.external_wait_reason = None;
            }
            EventPayload::ExternalWaitStarted { reason } => {
                self.external_wait_reason = Some(reason.clone());
            }
            EventPayload::TimerScheduled {
                timer_id,
                fire_at,
                reason,
            } => {
                self.timers.insert(
                    timer_id.clone(),
                    TimerProjection {
                        id: timer_id.clone(),
                        fire_at: *fire_at,
                        reason: reason.clone(),
                    },
                );
            }
            EventPayload::TimerFired { timer_id } => {
                self.timers.remove(timer_id);
                self.external_wait_reason = None;
            }
            EventPayload::TimerCancelled { timer_id, .. } => {
                self.timers.remove(timer_id);
            }
            EventPayload::RunStarted { .. } => {
                let scope = self
                    .scopes
                    .entry(event.envelope.scope_id.clone())
                    .or_insert(ScopeProjection {
                        parent: None,
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        last_run_started_seq: None,
                        depends_on: vec![],
                        spec: None,
                    });
                scope.state = ScopeState::Running;
                scope.generation = event.envelope.generation;
                scope.last_run_started_seq = Some(event.seq);
                if let Some(plan) = self
                    .current_plan_id
                    .as_ref()
                    .and_then(|id| self.plans.get_mut(id))
                {
                    for step in &mut plan.steps {
                        if step.scope_id.as_ref() == Some(&event.envelope.scope_id)
                            && matches!(
                                step.status,
                                PlanStepStatus::Planned | PlanStepStatus::Ready
                            )
                        {
                            step.status = PlanStepStatus::Running;
                        }
                    }
                }
            }
            EventPayload::ScopeSpawned { child } => {
                self.scopes.insert(
                    child.clone(),
                    ScopeProjection {
                        parent: Some(event.envelope.scope_id.clone()),
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        last_run_started_seq: None,
                        depends_on: vec![],
                        spec: None,
                    },
                );
                self.scope_order.push(child.clone());
            }
            EventPayload::SubagentSpawned {
                child,
                spec,
                depends_on,
            } => {
                self.scopes.insert(
                    child.clone(),
                    ScopeProjection {
                        parent: Some(event.envelope.scope_id.clone()),
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        last_run_started_seq: None,
                        depends_on: depends_on.clone(),
                        spec: Some(spec.clone()),
                    },
                );
                self.scope_order.push(child.clone());
            }
            EventPayload::ScopeStateChanged { state } => {
                self.scopes
                    .entry(event.envelope.scope_id.clone())
                    .or_insert(ScopeProjection {
                        parent: None,
                        state: ScopeState::Running,
                        generation: event.envelope.generation,
                        last_run_started_seq: None,
                        depends_on: vec![],
                        spec: None,
                    })
                    .state = *state;
                if let Some(plan) = self
                    .current_plan_id
                    .as_ref()
                    .and_then(|id| self.plans.get_mut(id))
                {
                    let status = match state {
                        ScopeState::Completed => Some(PlanStepStatus::Completed),
                        ScopeState::Cancelled => Some(PlanStepStatus::Cancelled),
                        ScopeState::Failed => Some(PlanStepStatus::Blocked),
                        _ => None,
                    };
                    if let Some(status) = status {
                        for step in &mut plan.steps {
                            if step.scope_id.as_ref() == Some(&event.envelope.scope_id) {
                                step.status = status;
                            }
                        }
                    }
                }
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
                self.active_sandbox_leases
                    .insert(lease.id.clone(), lease.quantity);
                self.resource_usage.sandbox_reserved_seconds = self
                    .resource_usage
                    .sandbox_reserved_seconds
                    .saturating_add(lease.quantity);
            }
            EventPayload::LeaseRevoked { lease_id } => {
                if let Some(quantity) = self.active_sandbox_leases.remove(lease_id) {
                    self.resource_usage.sandbox_reserved_seconds = self
                        .resource_usage
                        .sandbox_reserved_seconds
                        .saturating_sub(quantity);
                }
            }
            EventPayload::SandboxFinished { usage } => {
                if self.sandbox_usages.contains_key(&usage.lease_id) {
                    return;
                }
                let seconds = usage
                    .elapsed_millis
                    .saturating_add(999)
                    .saturating_div(1000)
                    .max(1);
                self.resource_usage.sandbox_seconds =
                    self.resource_usage.sandbox_seconds.saturating_add(seconds);
                self.resource_usage.sandbox_actual_millis = self
                    .resource_usage
                    .sandbox_actual_millis
                    .saturating_add(usage.elapsed_millis);
                match usage.basis {
                    SandboxUsageBasis::Reported => {
                        self.resource_usage.sandbox_reported_seconds = self
                            .resource_usage
                            .sandbox_reported_seconds
                            .saturating_add(seconds);
                    }
                    SandboxUsageBasis::Estimated => {
                        self.resource_usage.sandbox_estimated_seconds = self
                            .resource_usage
                            .sandbox_estimated_seconds
                            .saturating_add(seconds);
                    }
                    SandboxUsageBasis::Unknown => {
                        self.resource_usage.sandbox_unknown_seconds = self
                            .resource_usage
                            .sandbox_unknown_seconds
                            .saturating_add(seconds);
                    }
                }
                self.sandbox_usages
                    .insert(usage.lease_id.clone(), usage.clone());
            }
            EventPayload::ArtifactProduced { artifact } => {
                self.artifacts.push(artifact.clone());
                self.artifact_scopes
                    .insert(artifact.id.clone(), event.envelope.scope_id.clone());
            }
            EventPayload::CheckpointCreated { checkpoint } => {
                self.checkpoints.push(checkpoint.clone());
            }
            _ => {}
        }
    }
}

impl IntentProjection {
    pub fn current_plan(&self) -> Option<&Plan> {
        self.current_plan_id
            .as_ref()
            .and_then(|id| self.plans.get(id))
    }

    /// 请求最近进入的 Attention Window。正常情况下一个请求只属于一个窗口。
    pub fn attention_window_for_request(
        &self,
        request_id: &AttentionId,
    ) -> Option<&AttentionWindow> {
        self.attention_windows
            .values()
            .filter(|window| window.contains(request_id))
            .max_by_key(|window| (window.opened_at, window.id.clone()))
    }

    /// 用户表面只展示已被 Kernel 裁决为可行动、且 Defer 已到期的请求。
    /// 没有裁决记录的请求失败关闭，不能绕过 Attention Kernel 直接打扰用户。
    pub fn actionable_requests(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Vec<&AttentionRequest> {
        self.open_requests
            .values()
            .filter(|request| match self.attention_rulings.get(&request.id) {
                Some(Ruling::Interrupt) => true,
                Some(Ruling::Batch | Ruling::Defer { .. }) => self
                    .attention_window_for_request(&request.id)
                    .is_some_and(|window| window.closed_at.is_some() && window.due_at <= now),
                Some(Ruling::Silent) | None => false,
            })
            .collect()
    }

    /// 已关闭且仍有未决请求的窗口，按截止时间排序；一个 Batch 只出现一次。
    pub fn actionable_attention_windows(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Vec<(&AttentionWindow, Vec<&AttentionRequest>)> {
        let mut windows = Vec::new();
        for window in self.attention_windows.values() {
            if window.closed_at.is_none() || window.due_at > now {
                continue;
            }
            let mut requests: Vec<_> = window
                .request_ids
                .iter()
                .filter_map(|request_id| self.open_requests.get(request_id))
                .filter(|request| {
                    matches!(
                        self.attention_rulings.get(&request.id),
                        Some(Ruling::Batch | Ruling::Defer { .. })
                    )
                })
                .collect();
            requests.sort_by_key(|request| request.id.clone());
            if !requests.is_empty() {
                windows.push((window, requests));
            }
        }
        windows.sort_by_key(|(window, _)| {
            (window.closed_at.unwrap_or(window.due_at), window.id.clone())
        });
        windows
    }

    /// 事件派生的注意力指标；不依赖进程内计数器，重启后保持一致。
    pub fn attention_metrics(&self, now: chrono::DateTime<chrono::Utc>) -> AttentionMetrics {
        let mut metrics = AttentionMetrics {
            requested: self.attention_records.len() as u64,
            ..AttentionMetrics::default()
        };
        for record in self.attention_records.values() {
            if record.decision.is_some() {
                metrics.resolved += 1;
            }
            match &record.ruling {
                Some(Ruling::Interrupt) => {
                    metrics.interrupt_notifications += 1;
                    if record.decision.is_some() {
                        metrics.resolved_interrupts += 1;
                    }
                    if matches!(
                        &record.decision,
                        Some(Decision {
                            choice: else_protocol::attention::DecisionChoice::Dismissed,
                            decided_by: Actor::User,
                            ..
                        })
                    ) {
                        metrics.dismissed_interrupts += 1;
                    }
                }
                Some(Ruling::Batch) => metrics.batched_requests += 1,
                Some(Ruling::Defer { .. }) => metrics.deferred_requests += 1,
                Some(Ruling::Silent) => metrics.silent_requests += 1,
                None => {}
            }
            if record.request.expected_loss == else_protocol::attention::ExpectedLoss::Critical
                && record.decision.is_none()
            {
                metrics.open_critical += 1;
                let actionable = self
                    .actionable_requests(now)
                    .iter()
                    .any(|request| request.id == record.request.id);
                if !actionable {
                    metrics.missed_critical += 1;
                }
            }
        }
        for window in self.attention_windows.values() {
            if window.delivered_at.is_none() {
                continue;
            }
            match window.kind {
                AttentionWindowKind::Batch => metrics.batch_notifications += 1,
                AttentionWindowKind::Defer => metrics.defer_notifications += 1,
            }
        }
        metrics.interventions = metrics.interrupt_notifications
            + metrics.batch_notifications
            + metrics.defer_notifications;
        if metrics.interventions > 0 {
            metrics.batch_rate =
                Some(metrics.batch_notifications as f64 / metrics.interventions as f64);
        }
        if metrics.resolved_interrupts > 0 {
            metrics.misinterrupt_rate =
                Some(metrics.dismissed_interrupts as f64 / metrics.resolved_interrupts as f64);
        }
        metrics
    }

    pub fn due_timers(&self, now: chrono::DateTime<chrono::Utc>) -> Vec<&TimerProjection> {
        let mut timers: Vec<_> = self
            .timers
            .values()
            .filter(|timer| timer.fire_at <= now)
            .collect();
        timers.sort_by_key(|timer| (timer.fire_at, timer.id.clone()));
        timers
    }

    pub fn next_timer_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.timers.values().map(|timer| timer.fire_at).min()
    }
}
