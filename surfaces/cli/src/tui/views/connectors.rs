//! Connectors：外部来源的健康状况。

use super::super::data::ConnectorRow;
use super::super::row::{Pane, Row, RowAction};
use super::super::theme;

/// Connector 列表。
pub fn pane(rows: &[ConnectorRow], intent_id: Option<&else_protocol::ids::IntentId>) -> Pane {
    let mut lines = Vec::new();
    if rows.is_empty() {
        lines.push(Row::muted("no connectors registered"));
        return Pane {
            title: "Connectors".into(),
            rows: lines,
        };
    }
    for row in rows {
        let health = &row.health;
        let style = match health.health {
            else_protocol::connector::ConnectorHealth::Healthy => theme::done(),
            else_protocol::connector::ConnectorHealth::Degraded => theme::warning(),
            else_protocol::connector::ConnectorHealth::Error => theme::critical(),
        };
        lines.push(Row::action(
            format!(
                "  {}  {:?}  health={:?}  failures={}  credential={}",
                row.connector.id,
                row.connector.adapter,
                health.health,
                health.consecutive_failures,
                if health.credential_configured {
                    "set"
                } else {
                    "none"
                }
            ),
            style,
            RowAction::SyncConnector {
                intent_id: intent_id
                    .cloned()
                    .unwrap_or_else(|| else_protocol::ids::IntentId::from("unknown")),
                connector_id: row.connector.id.clone(),
            },
        ));
        lines.push(Row::muted(format!(
            "      source={}  last_success={}  next_sync={}",
            row.connector.source,
            health
                .last_success_at
                .map(|value| value.to_rfc3339())
                .unwrap_or_else(|| "never".into()),
            health.next_sync_at.to_rfc3339()
        )));
        if let Some(error) = &health.last_error {
            lines.push(Row::styled(
                format!("      error: {error}"),
                theme::critical(),
            ));
        }
    }
    lines.push(Row::muted("press s to sync the selected connector"));
    Pane {
        title: "Connectors".into(),
        rows: lines,
    }
}
