//! 工件与检查点：分支的不是代码，而是任务状态。
//!
//! 检查点 = 事件水位 + 工件快照 + 在途副作用结论，三者缺一不可。
//! Git 只是 Artifact 的一种后端，协议不绑定任何存储实现。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::effect::EffectConclusion;
use crate::ids::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// 文件树（代码、文档目录等），后端可为 CAS 或 Git。
    FileTree,
    /// 单个文档/二进制产物。
    Document,
    /// 结构化记录（表格、数据库行等）。
    StructuredRecord,
    /// 外部实体句柄（邮件草稿、日历事件等），比较靠证据而非 diff。
    ExternalHandle,
}

/// 工件及其血缘：由哪个 Run 产生。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    /// 内容寻址哈希、路径或外部句柄，由后端解释。
    pub locator: String,
    /// 可选的快照 manifest（文件清单等）。
    pub manifest: Option<String>,
    pub produced_by: RunId,
    pub note: String,
}

/// 结果证据：支撑某条 done_when 成立的工件或外部事实。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomeEvidence {
    pub criterion: String,
    pub artifact_id: Option<ArtifactId>,
    pub note: String,
}

/// 检查点时仍在途的副作用及其结论。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InFlightEffect {
    pub call_id: ToolCallId,
    pub conclusion: EffectConclusion,
    pub note: String,
}

/// 检查点：从它派生新 Scope（Fork）时，
/// 递增 Generation、工件 COW 隔离、按事件水位重建上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: CheckpointId,
    pub scope_id: ScopeId,
    pub generation: u64,
    pub thread_revision: u64,
    /// 事件水位：重放到此序号为止。
    pub event_cursor: u64,
    /// 工件快照根（文件树 manifest 或文档集合）。
    pub artifact_root: Option<ArtifactId>,
    /// 在途副作用结论。只有文件快照而没有它，"恢复"是假的。
    pub inflight: Vec<InFlightEffect>,
    pub created_at: DateTime<Utc>,
}
