//! else Context Plane：Memory 分层投影与规则式召回。
//!
//! 权威状态在 Ledger；本投影随时可由事件重放重建。
//! 召回是规则式作用域匹配（Global + 匹配的 Project），不做向量检索。
//! Memory 不是权限来源：记忆只进入快照的 `facts`，永不改写授权。

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use else_ledger::{Projection, StoredEvent};
use else_protocol::api::MemoryView;
use else_protocol::event::EventPayload;
use else_protocol::ids::{MemoryId, SnapshotId, ThreadId};
use else_protocol::memory::{ContextSnapshot, MemoryRecord, MemoryTier};

/// Memory 投影：已晋升记忆、待确认候选与 Context Snapshot，全部来自事件。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MemoryProjection {
    /// 已晋升且未撤销的记忆。
    pub promoted: BTreeMap<MemoryId, MemoryRecord>,
    /// 候选：已提出但未经规则校验或用户确认，不是权威状态。
    pub candidates: BTreeMap<MemoryId, MemoryRecord>,
    /// 已撤销的记忆 ID（含被取代者）。
    pub revoked: BTreeSet<MemoryId>,
    /// 已编译的 Context Snapshot。
    pub snapshots: BTreeMap<SnapshotId, ContextSnapshot>,
    /// 已重放到的事件水位。
    pub cursor: u64,
}

impl MemoryProjection {
    /// 契约视图：跨进程传输用的记忆快照。
    ///
    /// 投影本身实现了 Ledger 的 `Projection` 契约，因此留在本 crate；
    /// 线上形状由 `else-protocol` 定义，这里只做一次受编译器检查的转换。
    pub fn view(&self) -> MemoryView {
        MemoryView {
            promoted: self.promoted.clone(),
            candidates: self.candidates.clone(),
            revoked: self.revoked.clone(),
            snapshots: self.snapshots.clone(),
            cursor: self.cursor,
        }
    }
}

impl Projection for MemoryProjection {
    fn apply(&mut self, event: &StoredEvent) {
        self.cursor = event.seq;
        match &event.envelope.payload {
            EventPayload::MemoryCandidateProposed { record } => {
                self.candidates.insert(record.id.clone(), record.clone());
            }
            EventPayload::MemoryPromoted { memory } => {
                self.candidates.remove(&memory.id);
                if let Some(old) = &memory.supersedes {
                    self.promoted.remove(old);
                    self.revoked.insert(old.clone());
                }
                self.promoted.insert(memory.id.clone(), memory.clone());
            }
            EventPayload::MemoryRevoked { memory_id, .. } => {
                self.promoted.remove(memory_id);
                self.candidates.remove(memory_id);
                self.revoked.insert(memory_id.clone());
            }
            EventPayload::ContextCompiled { snapshot } => {
                self.snapshots.insert(snapshot.id.clone(), snapshot.clone());
            }
            _ => {}
        }
    }
}

impl MemoryProjection {
    /// 规则式召回：Global + 匹配的 Project(+ 当前 Thread) 作用域内、
    /// 指定分层、有效期内的已晋升记忆。确定性排序（按 ID）。
    pub fn recall(
        &self,
        project: Option<&str>,
        thread_id: Option<&ThreadId>,
        tiers: &[MemoryTier],
        now: DateTime<Utc>,
    ) -> Vec<MemoryRecord> {
        else_protocol::memory::recall_memories(
            self.promoted.values(),
            project,
            thread_id,
            tiers,
            now,
        )
    }
}

/// 晋升规则：Semantic 层永不由单次观察自动晋升——需要显式用户确认，
/// 或 ≥2 条独立支撑证据。Episodic/Working 由系统按既定时机晋升。
pub fn promotion_allowed(record: &MemoryRecord, user_confirmed: bool) -> bool {
    if record.tier != MemoryTier::Semantic {
        return true;
    }
    if user_confirmed {
        return true;
    }
    record.evidence.len() >= 2
}
