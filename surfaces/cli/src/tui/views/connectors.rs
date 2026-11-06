//! Settings 中的 Connector 健康与同步。

use super::super::data::ConnectorRow;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;
use super::short_id;

pub fn pane(rows: &[ConnectorRow], thread_id: Option<&else_protocol::ids::ThreadId>) -> Pane {
    let mut lines = Vec::new();
    if rows.is_empty() {
        lines.push(Row::muted("No connectors registered for this thread."));
        lines.push(Row::muted(
            "Use the CLI connector commands to connect Calendar, Linear or Notion.",
        ));
        return Pane {
            title: "Connectors".into(),
            rows: lines,
        };
    }
    lines.push(Row::muted("Enter or s to sync the selected connector."));
    for row in rows {
        let health = &row.health;
        let style = match health.health {
            else_protocol::connector::ConnectorHealth::Healthy => theme::done(),
            else_protocol::connector::ConnectorHealth::Degraded => theme::warning(),
            else_protocol::connector::ConnectorHealth::Error => theme::critical(),
        };
        let health_label = match health.health {
            else_protocol::connector::ConnectorHealth::Healthy => "healthy",
            else_protocol::connector::ConnectorHealth::Degraded => "degraded",
            else_protocol::connector::ConnectorHealth::Error => "error",
        };
        lines.push(Row::action(
            format!(
                "  {} · {} · {health_label}",
                row.connector.name, row.connector.adapter
            ),
            style,
            RowAction::SyncConnector {
                thread_id: thread_id
                    .cloned()
                    .unwrap_or_else(|| else_protocol::ids::ThreadId::from("unknown")),
                connector_id: row.connector.id.clone(),
            },
        ));
        lines.push(Row::muted(format!(
            "      source={} · credential={}",
            row.connector.source,
            if health.credential_configured {
                "configured"
            } else {
                "missing"
            }
        )));
        lines.push(Row::muted(format!(
            "      last success={} · next sync={}",
            health
                .last_success_at
                .map(|value| value.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "never".into()),
            health.next_sync_at.format("%Y-%m-%d %H:%M UTC")
        )));
        if let Some(error) = &health.last_error {
            lines.push(Row::styled(format!("      {error}"), theme::critical()));
        }
    }
    Pane {
        title: thread_id
            .map(|id| format!("Connectors · {}", short_id(id)))
            .unwrap_or_else(|| "Connectors".into()),
        rows: lines,
    }
}
