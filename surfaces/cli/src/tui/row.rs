//! 列表行及其可执行动作。

use else_protocol::attention::DecisionChoice;
use else_protocol::ids::{AttentionId, ConnectorId, IntentId, WorldItemId};
use else_protocol::world::WorldItemResolutionKind;
use ratatui::style::Style;

#[derive(Debug, Clone)]
pub struct Row {
    pub text: String,
    pub style: Style,
    pub action: Option<RowAction>,
}

impl Row {
    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            action: None,
        }
    }

    pub fn action(text: impl Into<String>, style: Style, action: RowAction) -> Self {
        Self {
            text: text.into(),
            style,
            action: Some(action),
        }
    }

    pub fn heading(text: impl Into<String>) -> Self {
        Self::styled(text, crate::tui::theme::section())
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self::styled(text, crate::tui::theme::muted())
    }
}

#[derive(Debug, Clone)]
pub struct Pane {
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone)]
pub enum RowAction {
    OpenIntent(IntentId),
    OpenDecision {
        intent_id: IntentId,
        request_id: AttentionId,
    },
    OpenConnectors(IntentId),
    AdvanceIntent(IntentId),
    Decide {
        intent_id: IntentId,
        request_id: AttentionId,
        choice: DecisionChoice,
    },
    ResolveWorld {
        intent_id: IntentId,
        item_id: WorldItemId,
        kind: WorldItemResolutionKind,
    },
    SyncConnector {
        intent_id: IntentId,
        connector_id: ConnectorId,
    },
}

impl RowAction {
    /// 刷新后按业务对象恢复选择，而不是按可能已变化的行号恢复。
    pub fn stable_key(&self) -> String {
        match self {
            Self::OpenIntent(id) => format!("intent:{id}"),
            Self::OpenDecision {
                intent_id,
                request_id,
            } => format!("decision:{intent_id}:{request_id}"),
            Self::OpenConnectors(id) => format!("connectors:{id}"),
            Self::AdvanceIntent(id) => format!("advance:{id}"),
            Self::Decide {
                intent_id,
                request_id,
                choice,
            } => format!("decide:{intent_id}:{request_id}:{choice:?}"),
            Self::ResolveWorld {
                intent_id,
                item_id,
                kind,
            } => format!("world:{intent_id}:{item_id}:{kind:?}"),
            Self::SyncConnector {
                intent_id,
                connector_id,
            } => format!("sync:{intent_id}:{connector_id}"),
        }
    }
}
