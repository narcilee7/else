//! 列表行及其可执行动作。
//!
//! 界面上只有一种东西：一列从上往下读的行。每行最多带三样附加信息 ——
//! 右对齐的 `meta`（让同类数字竖着可比）、选中时才显示的 `note`
//! （补全句子，列表因此能保持一行）、以及一个可执行动作。

use else_protocol::attention::DecisionChoice;
use else_protocol::ids::{AttentionId, ConnectorId, ThreadId, WorldItemId};
use else_protocol::world::WorldItemResolutionKind;
use ratatui::style::Style;

#[derive(Debug, Clone)]
pub struct Row {
    pub text: String,
    /// 右对齐的一列：紧急度、耗时、花费、产物数。
    pub meta: Option<String>,
    /// 选中时显示在页脚的补充句。列表不为了塞下它而变成两行。
    pub note: Option<String>,
    pub style: Style,
    pub action: Option<RowAction>,
}

impl Row {
    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            meta: None,
            note: None,
            style,
            action: None,
        }
    }

    pub fn action(text: impl Into<String>, style: Style, action: RowAction) -> Self {
        Self {
            action: Some(action),
            ..Self::styled(text, style)
        }
    }

    pub fn heading(text: impl Into<String>) -> Self {
        Self::styled(text, crate::tui::theme::section())
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self::styled(text, crate::tui::theme::muted())
    }

    /// 空一行：分组之间唯一的分隔手段，不画边。
    pub fn blank() -> Self {
        Self::styled("", crate::tui::theme::text())
    }

    pub fn with_meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

#[derive(Debug, Clone)]
pub enum RowAction {
    OpenThread(ThreadId),
    /// 跳到完整列表：首屏每组只显示三行，多出来的走这里。
    OpenResults,
    OpenDecision {
        thread_id: ThreadId,
        request_id: AttentionId,
    },
    OpenConnectors(ThreadId),
    Decide {
        thread_id: ThreadId,
        request_id: AttentionId,
        choice: DecisionChoice,
    },
    ResolveWorld {
        thread_id: ThreadId,
        item_id: WorldItemId,
        kind: WorldItemResolutionKind,
    },
    SyncConnector {
        thread_id: ThreadId,
        connector_id: ConnectorId,
    },
}

impl RowAction {
    /// 刷新后按业务对象恢复选择，而不是按可能已变化的行号恢复。
    pub fn stable_key(&self) -> String {
        match self {
            Self::OpenThread(id) => format!("thread:{id}"),
            Self::OpenResults => "results".to_string(),
            Self::OpenDecision {
                thread_id,
                request_id,
            } => format!("decision:{thread_id}:{request_id}"),
            Self::OpenConnectors(id) => format!("connectors:{id}"),
            Self::Decide {
                thread_id,
                request_id,
                choice,
            } => format!("decide:{thread_id}:{request_id}:{choice:?}"),
            Self::ResolveWorld {
                thread_id,
                item_id,
                kind,
            } => format!("world:{thread_id}:{item_id}:{kind:?}"),
            Self::SyncConnector {
                thread_id,
                connector_id,
            } => format!("sync:{thread_id}:{connector_id}"),
        }
    }
}
