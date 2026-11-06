//! World：承诺、等待、行动与风险。

use else_protocol::ids::ThreadId;
use else_protocol::world::{WorldItem, WorldItemResolutionKind};

use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

/// World 条目列表。
pub fn pane(items: &[WorldItem], thread_id: Option<&ThreadId>) -> Pane {
    let mut rows = Vec::new();
    if items.is_empty() {
        rows.push(Row::muted("nothing open in this world"));
        return Pane {
            title: "World".into(),
            rows,
        };
    }
    for item in items {
        rows.push(item_row(thread_id, item));
    }
    Pane {
        title: "World".into(),
        rows,
    }
}

/// 一个 World 条目。
///
/// `thread_id` 缺省时取条目自己的归属 —— World 条目总是属于某个 Thread。
pub fn item_row(thread_id: Option<&ThreadId>, item: &WorldItem) -> Row {
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
    let thread_id = thread_id
        .cloned()
        .unwrap_or_else(|| ThreadId::from("unknown"));
    Row::action(
        text,
        style,
        RowAction::ResolveWorld {
            thread_id,
            item_id: item.id.clone(),
            kind: WorldItemResolutionKind::Completed,
        },
    )
}
