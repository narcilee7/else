//! 连接器：外部来源的健康与同步。
//!
//! 一行一个来源，右边是"能不能用"。细节（凭据、上次成功、下次同步）放到
//! 选中时的 note 里 —— 平时不占地方，需要时就在页脚。

use else_protocol::connector::ConnectorHealth;

use super::super::data::ConnectorRow;
use super::super::row::{Row, RowAction};
use super::super::theme;

pub fn rows(rows: &[ConnectorRow], thread_id: Option<&else_protocol::ids::ThreadId>) -> Vec<Row> {
    if rows.is_empty() {
        return vec![
            Row::blank(),
            Row::muted(" No connector is connected to this thread."),
            Row::muted(" Use `else-bot connector` to connect Calendar, Linear or Notion."),
        ];
    }

    let mut rows_out = vec![Row::heading(" SOURCES")];
    for row in rows {
        let (style, label) = match row.health.health {
            ConnectorHealth::Healthy => (theme::done(), "healthy"),
            ConnectorHealth::Degraded => (theme::warning(), "degraded"),
            ConnectorHealth::Error => (theme::critical(), "error"),
        };
        let note = format!(
            "source {} · credential {} · last success {} · next sync {}",
            row.connector.source,
            if row.health.credential_configured {
                "configured"
            } else {
                "missing"
            },
            row.health
                .last_success_at
                .map(|value| value.format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "never".into()),
            row.health.next_sync_at.format("%m-%d %H:%M")
        );
        let mut line = Row::action(
            format!(" {} · {}", row.connector.name, row.connector.adapter),
            style,
            RowAction::SyncConnector {
                thread_id: thread_id
                    .cloned()
                    .unwrap_or_else(|| else_protocol::ids::ThreadId::from("unknown")),
                connector_id: row.connector.id.clone(),
            },
        )
        .with_meta(label)
        .with_note(note);
        if let Some(error) = &row.health.last_error {
            line = line.with_note(error.to_string());
        }
        rows_out.push(line);
    }
    rows_out
}
