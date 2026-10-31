//! 列表行：屏幕渲染出来的一行，以及它能触发的动作。
//!
//! 每个屏幕把自己的内容摊平成一串行。`App` 只知道「第几行被选中了」，
//! 按下 Enter 时再看这一行带什么动作。屏幕因此不需要各自实现一套导航逻辑。

use else_protocol::attention::DecisionChoice;
use else_protocol::ids::{AttentionId, ConnectorId, IntentId, WorldItemId};
use else_protocol::world::WorldItemResolutionKind;
use ratatui::style::Style;

/// 一行内容。
#[derive(Debug, Clone)]
pub struct Row {
    pub text: String,
    pub style: Style,
    /// 这一行按下 Enter / 快捷键时会做什么。
    pub action: Option<RowAction>,
}

impl Row {
    /// 带样式的文本行。
    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            action: None,
        }
    }

    /// 带动作的行。
    pub fn action(text: impl Into<String>, style: Style, action: RowAction) -> Self {
        Self {
            text: text.into(),
            style,
            action: Some(action),
        }
    }

    /// 分组标题行。
    pub fn heading(text: impl Into<String>) -> Self {
        Self::styled(text, crate::tui::theme::section())
    }

    /// 次要说明行。
    pub fn muted(text: impl Into<String>) -> Self {
        Self::styled(text, crate::tui::theme::muted())
    }
}

/// 一个面板：一组带标题的行。
#[derive(Debug, Clone)]
pub struct Pane {
    pub title: String,
    pub rows: Vec<Row>,
}

/// 行能触发的动作。
#[derive(Debug, Clone)]
pub enum RowAction {
    /// 打开一个 Intent 的详情。
    OpenIntent(IntentId),
    /// 推进一个 Intent。
    AdvanceIntent(IntentId),
    /// 对一条注意力请求做出裁决。
    Decide {
        intent_id: IntentId,
        request_id: AttentionId,
        choice: DecisionChoice,
    },
    /// 解决一个 World 条目。
    ResolveWorld {
        intent_id: IntentId,
        item_id: WorldItemId,
        kind: WorldItemResolutionKind,
    },
    /// 立即同步一个 Connector。
    SyncConnector {
        intent_id: IntentId,
        connector_id: ConnectorId,
    },
}
