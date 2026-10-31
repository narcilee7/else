//! Memory 与 Context Snapshot：分层记忆与最小只读上下文。
//!
//! 核心边界：Memory 不是权限来源。记忆记录只能出现在快照的 `facts`
//! 里，永远不能改写 `authority`；Verifier 只按快照 revision 时点的
//! IntentSpec 验收，不看召回的记忆。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ArtifactId, IntentId, MemoryId, SnapshotId};
use crate::intent::{Assumption, Criterion};
use crate::plan::Plan;

/// 记忆分层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// 当前 Run 的临时工作状态，可丢弃；若持久化也只召回给同一 Intent。
    Working,
    /// 历史 Intent 的结论、产物引用与失败经验。
    Episodic,
    /// 有证据、有作用域、经晋升规则确认的稳定事实与偏好。
    Semantic,
}

/// 规则式召回：Global + 匹配的 Project / Intent，再按层级与有效期过滤。
///
/// 召回是纯函数：同一批记忆、同一组条件，任何 surface 都得到同一批结果。
/// 定义放在协议层是为了让 CLI、TUI 与 App 用同一套规则，不各自实现一遍。
///
/// 记忆只是上下文，不是权威：召回结果不会改写授权。
pub fn recall_memories<'a>(
    promoted: impl Iterator<Item = &'a MemoryRecord>,
    project: Option<&str>,
    intent_id: Option<&IntentId>,
    tiers: &[MemoryTier],
    now: DateTime<Utc>,
) -> Vec<MemoryRecord> {
    promoted
        .filter(|record| tiers.contains(&record.tier))
        .filter(|record| record.valid_until.is_none_or(|until| until > now))
        .filter(|record| match &record.scope {
            MemoryScope::Global => true,
            MemoryScope::Project(name) => Some(name.as_str()) == project,
            MemoryScope::Intent(id) => Some(id) == intent_id,
        })
        .cloned()
        .collect()
}

/// 记忆作用域。召回按作用域匹配：Global + 匹配的 Project。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MemoryScope {
    Global,
    Project(String),
    Intent(IntentId),
}

/// 记忆来源：决定晋升规则。Semantic 层永不由单次观察自动晋升。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// 用户明确陈述（含纠正）。作用域以用户指定的为准，不泛化到 Global。
    UserStated,
    ExternalObserved,
    ExecutionObserved,
    Inferred,
}

/// 一条记忆。候选（candidate）不是权威状态，晋升（promoted）后才进入投影。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: MemoryId,
    pub tier: MemoryTier,
    pub scope: MemoryScope,
    pub subject: String,
    pub predicate: String,
    pub value: String,
    pub source: MemorySource,
    /// 支撑证据：相关事件 id。
    pub evidence: Vec<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub supersedes: Option<MemoryId>,
    pub created_at: DateTime<Utc>,
}

/// Context Snapshot 的服务对象：各取所需的最小视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotAudience {
    Planner,
    Executor,
    Verifier,
}

/// 一次执行使用的一致上下文版本。可序列化（Scale-ready：
/// 未来可发送给远程 Worker），内容只读。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub id: SnapshotId,
    pub intent_id: IntentId,
    /// 快照基于的 Intent Revision；Verifier 永远按此 revision 的 spec 验收。
    pub intent_revision: u64,
    /// 快照基于的事件水位。
    pub event_cursor: u64,
    pub audience: SnapshotAudience,
    pub outcome: String,
    pub done_when: Vec<Criterion>,
    pub constraints: Vec<String>,
    pub assumptions: Vec<Assumption>,
    /// 被召回记忆的 ID（来源可审计）。
    pub memory_refs: Vec<MemoryId>,
    /// 被召回记忆的内容（只读副本，永不写回）。
    pub facts: Vec<MemoryRecord>,
    pub artifact_refs: Vec<ArtifactId>,
    /// 已物化到当前 Workspace 的工件相对路径，供 Harness 直接读取。
    #[serde(default)]
    pub artifact_paths: Vec<String>,
    /// 自本 Scope 上次 RunStarted 以来到达的外部事实。
    #[serde(default)]
    pub external_events: Vec<ExternalEventContext>,
    /// 当前结构化 Plan 的只读版本；Verifier 不接收 Plan。
    #[serde(default)]
    pub plan: Option<Plan>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalEventContext {
    pub event_id: String,
    pub seq: u64,
    pub source: String,
    pub summary: String,
    pub occurred_at: DateTime<Utc>,
}
