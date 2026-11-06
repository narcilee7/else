//! Runtime HTTP 契约：所有 surface 共用的请求与响应形状。
//!
//! 这里只定义**跨进程传输**的形状，不含领域语义。领域语义在
//! [`crate::thread`] / [`crate::attention`] / [`crate::world`] 等模块。
//!
//! 设计约束：
//!
//! - 同一形状只在这里定义一次。daemon 序列化它，CLI / TUI / Apple 客户端反序列化它。
//! - JSON 形状是**兼容性承诺**：只加字段，不改字段名，不改字段语义。
//!   破坏性变更必须升级 [`CONTRACT_VERSION`]。
//! - 契约版本与事件协议版本相互独立：契约描述 surface 之间的传输形状，
//!   [`crate::PROTOCOL_VERSION`] 描述写入 Ledger 的事件语义。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::artifact::Artifact;
use crate::attention::{AttentionMetrics, AttentionRequest, AttentionWindow};
use crate::event::ScopeState;
use crate::failure::ScopeFailure;
use crate::harness::{HarnessCapabilities, HarnessNetworkAccess};
use crate::ids::{MemoryId, ScopeId, SnapshotId, ThreadId};
use crate::memory::{ContextSnapshot, MemoryRecord};
use crate::plan::{Plan, PlanImpact};
use crate::resource::{ResourceBudget, ResourceUsage, SandboxUsage};
use crate::thread::{ThreadSpec, ThreadState};

/// Runtime HTTP 契约版本。surface 之间据此判断是否可以对话。
pub const CONTRACT_VERSION: &str = "1";

/// 一个 Thread 的完整可读快照。
///
/// surface 用它渲染列表与详情：状态、待决策请求、产物、资源消耗与事件水位。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub spec: ThreadSpec,
    pub state: ThreadState,
    /// 当前需要用户裁决的注意力请求；不含被 Batch / Defer 窗口折叠的部分。
    pub decisions: Vec<AttentionRequest>,
    pub artifacts: Vec<Artifact>,
    pub resource_budget: ResourceBudget,
    pub resource_usage: ResourceUsage,
    /// 已应用到的事件水位。客户端用它续读事件。
    pub cursor: u64,
    /// 异步操作的句柄；同步完成时缺省。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

/// 一个注意力窗口及其覆盖的请求。
///
/// 窗口是 Batch / Defer 的持久调度单位，surface 用它把可延后的请求分组呈现，
/// 而不是逐条打扰用户。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionWindowSnapshot {
    pub thread_id: ThreadId,
    pub window: AttentionWindow,
    pub requests: Vec<AttentionRequest>,
}

/// 一个 Thread 的注意力视图：打断成本、全部窗口与待决请求。
///
/// 指标是**成本账**：用户花掉的注意力是产品要优化的唯一稀缺资源，
/// 所以每个 surface 都用同一份数字解释「这个 Thread 打扰了我几次」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionView {
    pub metrics: AttentionMetrics,
    /// 全部窗口，含已关闭的。
    pub windows: Vec<AttentionWindow>,
    /// 当前可裁决的请求（含窗口内与窗口外）。
    pub requests: Vec<AttentionRequest>,
}

/// 一条已落账事件的传输形状。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventResponse {
    pub seq: u64,
    pub event_id: String,
    pub thread_id: String,
    pub scope_id: String,
    pub run_id: Option<String>,
    pub generation: u64,
    pub thread_revision: u64,
    pub occurred_at: String,
    pub payload: Value,
}

/// 一个 Scope 的可读快照。
///
/// Scope 是执行与权限的最小单位。surface 用它渲染委托树：
/// 父指针给出层级，`depends_on` 给出显式依赖，`generation` 给出围栏版本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopeView {
    pub scope_id: ScopeId,
    pub parent: Option<ScopeId>,
    pub state: ScopeState,
    /// 执行代次；旧 Generation 的写入与外部动作会被 Gateway 拒绝。
    pub generation: u64,
    pub depends_on: Vec<ScopeId>,
    /// 自动替换来源；替换链被限制为一层。
    pub replaces: Option<ScopeId>,
    pub failure_count: u32,
    pub last_failure: Option<ScopeFailure>,
    /// Scope 的目标；根 Scope 与 Thread 同义。
    pub outcome: String,
}

/// 计划视图：当前计划、版本历史与外部事实带来的影响分析。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanView {
    pub current: Option<Plan>,
    pub history: Vec<Plan>,
    pub impacts: Vec<PlanImpact>,
}

/// 记忆视图：已晋升记忆、待确认候选与已编译快照。
///
/// 记忆不是权威状态：它只影响 Context Snapshot 的 `facts`，永不改写授权。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryView {
    pub promoted: BTreeMap<MemoryId, MemoryRecord>,
    pub candidates: BTreeMap<MemoryId, MemoryRecord>,
    pub revoked: BTreeSet<MemoryId>,
    pub snapshots: BTreeMap<SnapshotId, ContextSnapshot>,
    /// 已重放到的事件水位。
    pub cursor: u64,
}

/// 资源账视图：硬预算、累计用量与逐个 Sandbox 的证据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceView {
    pub budget: ResourceBudget,
    pub usage: ResourceUsage,
    pub sandboxes: Vec<SandboxUsage>,
}

/// 一个 Harness 适配器的清单。
///
/// surface 用它解释「这个执行器能做什么、能不能联网、副作用可见性如何」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessView {
    pub name: String,
    pub capabilities: HarnessCapabilities,
    pub network_access: HarnessNetworkAccess,
}

/// 一次连续推进的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveView {
    pub snapshot: ThreadSnapshot,
    /// 推进停止的原因；见各 surface 的呈现约定。
    pub stopped: String,
    /// 实际执行的 Kernel 步数。
    pub steps: u64,
}
