//! World：承诺、等待、行动与风险。

use else_protocol::ids::IntentId;
use else_protocol::world::{WorldItem, WorldItemResolutionKind};

use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

/// World 条目列表。
pub fn pane(items: &[WorldItem], intent_id: Option<&IntentId>) -> Pane {
    let mut rows = Vec::new();
    if items.is_empty() {
        rows.push(Row::muted("nothing open in this world"));
        return Pane {
            title: "World".into(),
            rows,
        };
    }
    for item in items {
        rows.push(item_row(intent_id, item));
    }
    Pane {
        title: "World".into(),
        rows,
    }
}

/// 一个 World 条目。
///
/// `intent_id` 缺省时取条目自己的归属 —— World 条目总是属于某个 Intent。
pub fn item_row(intent_id: Option<&IntentId>, item: &WorldItem) -> Row {
    let style = match item.state {
        else_protocol::world::WorldItemState::Overdue => theme::critical(),
        else_protocol::world::WorldItemState::AtRisk => theme::warning(),
        else_protocol::world::WorldItemState::WaitingExternal => theme::waiting(),
        else_protocol::world::WorldItemState::Completed
        | else_protocol::world::WorldItemState::Dismissed => theme::done(),
        _ => theme::text(),
    };
    let due = item
        .deadline
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "-".into());
    let text = format!(
        "  {:?}  {:?}  due={due}  {}",
        item.kind, item.state, item.title
    );
    let intent_id = intent_id
        .cloned()
        .unwrap_or_else(|| IntentId::from("unknown"));
    Row::action(
        text,
        style,
        RowAction::ResolveWorld {
            intent_id,
            item_id: item.id.clone(),
            kind: WorldItemResolutionKind::Completed,
        },
    )
}
