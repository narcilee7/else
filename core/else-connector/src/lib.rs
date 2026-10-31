//! Connector Adapter Registry 与 Provider 无关的 Signal Interpreter。
//!
//! Adapter 只负责认证、拉取、校验与 Normalize，输出中性 Observation；
//! 业务含义由 Interpreter 独立产生，核心 Runtime 不感知 Provider 细节。

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use else_protocol::connector::{
    ConnectorAction, ConnectorAuthMode, ConnectorCredential, ConnectorHealth, ConnectorManifest,
    ConnectorObservation, ConnectorPullPage, ConnectorSyncMode, CorrelationHint, CorrelationKind,
    ObjectKind, ObservationChange, SignalInterpretation, SignalMeaning, SignalStatus, SignalType,
    SyncCursor, WorldSignal,
};
use else_protocol::effect::{EffectConclusion, ToolOutcome, ToolStatus};
use else_protocol::ids::ConnectorId;
use ical::IcalParser;
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("calendar: {0}")]
    Calendar(String),
    #[error("linear: {0}")]
    Linear(String),
    #[error("notion: {0}")]
    Notion(String),
    #[error("unknown connector adapter: {0}")]
    UnknownAdapter(String),
    #[error("connector adapter does not accept webhooks: {0}")]
    WebhookUnsupported(String),
    #[error("connector adapter does not implement write capability {capability}: {adapter}")]
    ActionUnsupported { adapter: String, capability: String },
    #[error("rate limited by provider; retry after {retry_after_seconds} seconds")]
    RateLimited { retry_after_seconds: u64 },
}

pub type Result<T> = std::result::Result<T, ConnectorError>;

pub trait ConnectorAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str];
    fn manifest(&self) -> ConnectorManifest;
    fn pull_stream_id(&self) -> &'static str;
    fn pull(
        &self,
        connector_id: ConnectorId,
        source: &str,
        cursor: Option<&SyncCursor>,
        now: DateTime<Utc>,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ConnectorPullPage>;
    fn webhook_stream_id(&self) -> Option<&'static str> {
        None
    }
    /// Webhook 只带实体 ID 时回源补齐。Adapter 必须保持幂等，失败时让
    /// Runtime 记录同步错误，而不是用不完整 payload 猜测对象内容。
    fn hydrate_webhook(
        &self,
        _connector_id: &ConnectorId,
        _source: &str,
        payload: &Value,
        _credential: Option<&ConnectorCredential>,
    ) -> Result<Value> {
        Ok(payload.clone())
    }
    fn normalize_webhook(
        &self,
        _connector_id: &ConnectorId,
        _payload: &Value,
    ) -> Result<Vec<ConnectorObservation>> {
        Err(ConnectorError::WebhookUnsupported(self.id().into()))
    }
    fn execute_action(
        &self,
        _connector_id: &ConnectorId,
        _source: &str,
        capability: &str,
        _credential: Option<&ConnectorCredential>,
        _input: &Value,
    ) -> Result<ToolOutcome> {
        Err(ConnectorError::ActionUnsupported {
            adapter: self.id().into(),
            capability: capability.into(),
        })
    }
    fn preview_action(
        &self,
        _source: &str,
        capability: &str,
        _target: &str,
        _credential: Option<&ConnectorCredential>,
        _input: &Value,
    ) -> Result<Value> {
        Err(ConnectorError::ActionUnsupported {
            adapter: self.id().into(),
            capability: format!("{capability}#preview"),
        })
    }
    fn action_schema(&self, _capability: &str) -> Option<Value> {
        None
    }
    fn verify_action(
        &self,
        _source: &str,
        action: &ConnectorAction,
        _credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        Err(ConnectorError::ActionUnsupported {
            adapter: self.id().into(),
            capability: format!("{}#verify", action.capability),
        })
    }
    fn compensate_action(
        &self,
        _source: &str,
        action: &ConnectorAction,
        _credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        Err(ConnectorError::ActionUnsupported {
            adapter: self.id().into(),
            capability: format!("{}#compensate", action.capability),
        })
    }
}

pub struct AdapterRegistry {
    adapters: Vec<Box<dyn ConnectorAdapter>>,
}

impl AdapterRegistry {
    pub fn new(adapters: Vec<Box<dyn ConnectorAdapter>>) -> Self {
        Self { adapters }
    }

    pub fn builtin() -> Self {
        Self::new(vec![
            Box::new(CalendarAdapter),
            Box::new(GoogleCalendarAdapter),
            Box::new(LinearAdapter),
            Box::new(NotionAdapter),
        ])
    }

    pub fn get(&self, id: &str) -> Option<&dyn ConnectorAdapter> {
        self.adapters.iter().find_map(|adapter| {
            (adapter.id() == id || adapter.aliases().contains(&id)).then_some(adapter.as_ref())
        })
    }

    pub fn manifests(&self) -> Vec<ConnectorManifest> {
        let mut manifests = self
            .adapters
            .iter()
            .map(|adapter| adapter.manifest())
            .collect::<Vec<_>>();
        manifests.sort_by(|left, right| left.adapter.cmp(&right.adapter));
        manifests
    }
}

pub struct CalendarAdapter;

impl ConnectorAdapter for CalendarAdapter {
    fn id(&self) -> &'static str {
        "calendar.ics"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["calendar", "calendar_read_only"]
    }

    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            adapter: self.id().into(),
            provider: "calendar".into(),
            auth_mode: ConnectorAuthMode::None,
            scopes: vec!["calendar.read".into()],
            read_capabilities: vec!["calendar.event.read".into()],
            write_capabilities: vec!["calendar.event.update".into()],
            sync_modes: vec![ConnectorSyncMode::Pull],
            data_sensitivity: "internal".into(),
            retention_policy: "metadata_only".into(),
            health: ConnectorHealth::Healthy,
        }
    }

    fn pull_stream_id(&self) -> &'static str {
        "calendar-events"
    }

    fn pull(
        &self,
        connector_id: ConnectorId,
        source: &str,
        _cursor: Option<&SyncCursor>,
        now: DateTime<Utc>,
        _credential: Option<&ConnectorCredential>,
    ) -> Result<ConnectorPullPage> {
        pull_calendar(
            connector_id,
            source,
            now - Duration::days(1),
            now + Duration::days(14),
        )
    }
}

pub struct GoogleCalendarAdapter;

impl ConnectorAdapter for GoogleCalendarAdapter {
    fn id(&self) -> &'static str {
        "calendar.google"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["google-calendar", "gcal"]
    }

    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            adapter: self.id().into(),
            provider: "calendar".into(),
            auth_mode: ConnectorAuthMode::OAuth,
            scopes: vec!["calendar.events".into()],
            read_capabilities: vec!["calendar.event.read".into()],
            write_capabilities: vec!["calendar.event.update".into()],
            sync_modes: vec![ConnectorSyncMode::Pull],
            data_sensitivity: "internal".into(),
            retention_policy: "metadata_and_summary".into(),
            health: ConnectorHealth::Healthy,
        }
    }

    fn pull_stream_id(&self) -> &'static str {
        "google-calendar-events"
    }

    fn pull(
        &self,
        connector_id: ConnectorId,
        source: &str,
        cursor: Option<&SyncCursor>,
        _now: DateTime<Utc>,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ConnectorPullPage> {
        pull_google_calendar(connector_id, source, cursor, credential)
    }

    fn execute_action(
        &self,
        connector_id: &ConnectorId,
        source: &str,
        capability: &str,
        credential: Option<&ConnectorCredential>,
        input: &Value,
    ) -> Result<ToolOutcome> {
        if capability != "calendar.event.update" {
            return Err(ConnectorError::ActionUnsupported {
                adapter: self.id().into(),
                capability: capability.into(),
            });
        }
        let target = input["target"]
            .as_str()
            .filter(|target| !target.is_empty())
            .ok_or_else(|| ConnectorError::Calendar("event target is required".into()))?;
        let changes = writable_google_event_fields(&input["changes"]);
        if changes.as_object().is_none_or(|changes| changes.is_empty()) {
            return Err(ConnectorError::Calendar(
                "calendar.action.update requires non-empty changes".into(),
            ));
        }
        let url = google_event_url(source, target)?;
        let before = google_calendar_json("GET", &url, credential, None)?;
        let after = google_calendar_json("PATCH", &url, credential, Some(changes))?;
        Ok(ToolOutcome {
            status: ToolStatus::Succeeded,
            output: json!({
                "connector_id": connector_id,
                "before": writable_google_event_fields(&before),
                "after": writable_google_event_fields(&after),
                "event_id": target,
            }),
            effect: EffectConclusion::Applied,
        })
    }

    fn preview_action(
        &self,
        source: &str,
        capability: &str,
        target: &str,
        credential: Option<&ConnectorCredential>,
        _input: &Value,
    ) -> Result<Value> {
        if capability != "calendar.event.update" {
            return Err(ConnectorError::ActionUnsupported {
                adapter: self.id().into(),
                capability: capability.into(),
            });
        }
        let url = google_event_url(source, target)?;
        let current = google_calendar_json("GET", &url, credential, None)?;
        Ok(writable_google_event_fields(&current))
    }

    fn action_schema(&self, capability: &str) -> Option<Value> {
        matches!(capability, "calendar.event.update")
            .then(|| connector_action_schema(capability))
            .flatten()
    }

    fn verify_action(
        &self,
        source: &str,
        action: &ConnectorAction,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        let url = google_event_url(source, &action.target)?;
        let current = google_calendar_json("GET", &url, credential, None)?;
        let expected = writable_google_event_fields(&action.input["changes"]);
        let matches = expected.as_object().is_none_or(|fields| {
            fields
                .iter()
                .all(|(key, value)| current.get(key) == Some(value))
        });
        Ok(ToolOutcome {
            status: if matches {
                ToolStatus::Succeeded
            } else {
                ToolStatus::Failed
            },
            output: json!({
                "event": writable_google_event_fields(&current),
                "matches_expected": matches,
            }),
            effect: if matches {
                EffectConclusion::Applied
            } else {
                EffectConclusion::NoEffect
            },
        })
    }

    fn compensate_action(
        &self,
        source: &str,
        action: &ConnectorAction,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        let before = action
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.output.get("before"))
            .map(writable_google_event_fields)
            .filter(|before| before.as_object().is_some_and(|before| !before.is_empty()))
            .ok_or_else(|| {
                ConnectorError::Calendar(
                    "calendar compensation requires the pre-action event snapshot".into(),
                )
            })?;
        let url = google_event_url(source, &action.target)?;
        let restored = google_calendar_json("PATCH", &url, credential, Some(before))?;
        Ok(ToolOutcome {
            status: ToolStatus::Succeeded,
            output: json!({
                "event": writable_google_event_fields(&restored),
                "restored": true,
            }),
            effect: EffectConclusion::Compensated,
        })
    }
}

pub struct LinearAdapter;

impl ConnectorAdapter for LinearAdapter {
    fn id(&self) -> &'static str {
        "linear.graphql"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["linear", "linear_read_only"]
    }

    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            adapter: self.id().into(),
            provider: "linear".into(),
            auth_mode: ConnectorAuthMode::ApiToken,
            scopes: vec!["issues:read".into()],
            read_capabilities: vec![
                "linear.issue.read".into(),
                "linear.milestone.read".into(),
                "linear.blocker.read".into(),
            ],
            write_capabilities: vec!["linear.issue.comment".into(), "linear.issue.update".into()],
            sync_modes: vec![ConnectorSyncMode::Pull, ConnectorSyncMode::Webhook],
            data_sensitivity: "internal".into(),
            retention_policy: "metadata_only".into(),
            health: ConnectorHealth::Healthy,
        }
    }

    fn pull_stream_id(&self) -> &'static str {
        "linear-issues"
    }

    fn pull(
        &self,
        connector_id: ConnectorId,
        source: &str,
        cursor: Option<&SyncCursor>,
        _now: DateTime<Utc>,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ConnectorPullPage> {
        if source.starts_with("https://api.linear.app/") {
            pull_linear_graphql(connector_id, source, cursor, credential)
        } else {
            pull_linear(connector_id, source, credential)
        }
    }

    fn webhook_stream_id(&self) -> Option<&'static str> {
        Some("linear-webhook")
    }

    fn normalize_webhook(
        &self,
        connector_id: &ConnectorId,
        payload: &Value,
    ) -> Result<Vec<ConnectorObservation>> {
        normalize_linear_payload(connector_id, payload)
    }

    fn hydrate_webhook(
        &self,
        _connector_id: &ConnectorId,
        source: &str,
        payload: &Value,
        credential: Option<&ConnectorCredential>,
    ) -> Result<Value> {
        if linear_issue_nodes(payload)
            .iter()
            .any(|issue| string_at(issue, &["title"]).is_some())
        {
            return Ok(payload.clone());
        }
        let issue_id = linear_issue_nodes(payload)
            .into_iter()
            .find_map(|issue| string_at(issue, &["id"]))
            .or_else(|| string_at(payload, &["data", "id"]))
            .ok_or_else(|| ConnectorError::Linear("webhook hydration requires issue id".into()))?;
        hydrate_linear_issue(source, credential, &issue_id, payload)
    }

    fn execute_action(
        &self,
        connector_id: &ConnectorId,
        source: &str,
        capability: &str,
        credential: Option<&ConnectorCredential>,
        input: &Value,
    ) -> Result<ToolOutcome> {
        let target = input["target"]
            .as_str()
            .filter(|target| !target.is_empty())
            .ok_or_else(|| ConnectorError::Linear("issue target is required".into()))?;
        match capability {
            "linear.issue.update" => {
                let changes = linear_writable_issue_fields(&input["changes"]);
                if changes.as_object().is_none_or(|changes| changes.is_empty()) {
                    return Err(ConnectorError::Linear(
                        "linear.issue.update requires non-empty changes".into(),
                    ));
                }
                let before = linear_fetch_issue(source, credential, target)?;
                let after = linear_update_issue(source, credential, target, changes.clone())?;
                Ok(ToolOutcome {
                    status: ToolStatus::Succeeded,
                    output: json!({
                        "connector_id": connector_id,
                        "issue_id": target,
                        "before": linear_writable_issue_fields(&before),
                        "after": linear_writable_issue_fields(&after),
                    }),
                    effect: EffectConclusion::Applied,
                })
            }
            "linear.issue.comment" => {
                let body = input["body"]
                    .as_str()
                    .filter(|body| !body.trim().is_empty())
                    .ok_or_else(|| ConnectorError::Linear("comment body is required".into()))?;
                let comment = linear_create_comment(source, credential, target, body)?;
                Ok(ToolOutcome {
                    status: ToolStatus::Succeeded,
                    output: json!({
                        "connector_id": connector_id,
                        "issue_id": target,
                        "comment": comment,
                    }),
                    effect: EffectConclusion::Applied,
                })
            }
            _ => Err(ConnectorError::ActionUnsupported {
                adapter: self.id().into(),
                capability: capability.into(),
            }),
        }
    }

    fn preview_action(
        &self,
        source: &str,
        capability: &str,
        target: &str,
        credential: Option<&ConnectorCredential>,
        _input: &Value,
    ) -> Result<Value> {
        if !matches!(capability, "linear.issue.update" | "linear.issue.comment") {
            return Err(ConnectorError::ActionUnsupported {
                adapter: self.id().into(),
                capability: capability.into(),
            });
        }
        let issue = linear_fetch_issue(source, credential, target)?;
        Ok(linear_writable_issue_fields(&issue))
    }

    fn action_schema(&self, capability: &str) -> Option<Value> {
        matches!(capability, "linear.issue.update" | "linear.issue.comment")
            .then(|| connector_action_schema(capability))
            .flatten()
    }

    fn verify_action(
        &self,
        source: &str,
        action: &ConnectorAction,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        match action.capability.as_str() {
            "linear.issue.update" => {
                let issue = linear_fetch_issue(source, credential, &action.target)?;
                let expected = linear_writable_issue_fields(&action.input["changes"]);
                let matches = expected.as_object().is_none_or(|fields| {
                    fields
                        .iter()
                        .all(|(key, value)| issue.get(key) == Some(value))
                });
                Ok(ToolOutcome {
                    status: if matches {
                        ToolStatus::Succeeded
                    } else {
                        ToolStatus::Failed
                    },
                    output: json!({
                        "issue": linear_writable_issue_fields(&issue),
                        "matches_expected": matches,
                    }),
                    effect: if matches {
                        EffectConclusion::Applied
                    } else {
                        EffectConclusion::NoEffect
                    },
                })
            }
            "linear.issue.comment" => {
                let comment_id = action
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.output.pointer("/comment/id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ConnectorError::Linear("comment id is missing from action outcome".into())
                    })?;
                let comment = linear_fetch_comment(source, credential, comment_id)?;
                let matches = comment
                    .get("issue")
                    .and_then(|issue| issue.get("id"))
                    .and_then(Value::as_str)
                    == Some(action.target.as_str());
                Ok(ToolOutcome {
                    status: if matches {
                        ToolStatus::Succeeded
                    } else {
                        ToolStatus::Failed
                    },
                    output: json!({"comment": comment, "matches_issue": matches}),
                    effect: if matches {
                        EffectConclusion::Applied
                    } else {
                        EffectConclusion::NoEffect
                    },
                })
            }
            _ => Err(ConnectorError::ActionUnsupported {
                adapter: self.id().into(),
                capability: format!("{}#verify", action.capability),
            }),
        }
    }

    fn compensate_action(
        &self,
        source: &str,
        action: &ConnectorAction,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        match action.capability.as_str() {
            "linear.issue.update" => {
                let before = action
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.output.get("before"))
                    .map(linear_writable_issue_fields)
                    .filter(|before| before.as_object().is_some_and(|before| !before.is_empty()))
                    .ok_or_else(|| {
                        ConnectorError::Linear(
                            "issue compensation requires the pre-action snapshot".into(),
                        )
                    })?;
                let restored = linear_update_issue(source, credential, &action.target, before)?;
                Ok(ToolOutcome {
                    status: ToolStatus::Succeeded,
                    output: json!({"issue": linear_writable_issue_fields(&restored)}),
                    effect: EffectConclusion::Compensated,
                })
            }
            "linear.issue.comment" => {
                let comment_id = action
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.output.pointer("/comment/id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ConnectorError::Linear("comment id is missing from action outcome".into())
                    })?;
                linear_delete_comment(source, credential, comment_id)?;
                Ok(ToolOutcome {
                    status: ToolStatus::Succeeded,
                    output: json!({"comment_id": comment_id, "deleted": true}),
                    effect: EffectConclusion::Compensated,
                })
            }
            _ => Err(ConnectorError::ActionUnsupported {
                adapter: self.id().into(),
                capability: format!("{}#compensate", action.capability),
            }),
        }
    }
}

pub struct NotionAdapter;

impl ConnectorAdapter for NotionAdapter {
    fn id(&self) -> &'static str {
        "notion.api"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["notion", "notion_read_only"]
    }

    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            adapter: self.id().into(),
            provider: "notion".into(),
            auth_mode: ConnectorAuthMode::ApiToken,
            scopes: vec!["pages:read".into(), "databases:read".into()],
            read_capabilities: vec!["notion.page.read".into(), "notion.database.read".into()],
            write_capabilities: vec!["notion.page.update".into(), "notion.database.update".into()],
            sync_modes: vec![ConnectorSyncMode::Pull],
            data_sensitivity: "internal".into(),
            retention_policy: "metadata_and_summary".into(),
            health: ConnectorHealth::Healthy,
        }
    }

    fn pull_stream_id(&self) -> &'static str {
        "notion-pages"
    }

    fn pull(
        &self,
        connector_id: ConnectorId,
        source: &str,
        cursor: Option<&SyncCursor>,
        _now: DateTime<Utc>,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ConnectorPullPage> {
        pull_notion_with_cursor(connector_id, source, cursor, credential)
    }

    fn execute_action(
        &self,
        connector_id: &ConnectorId,
        source: &str,
        capability: &str,
        credential: Option<&ConnectorCredential>,
        input: &Value,
    ) -> Result<ToolOutcome> {
        let (object_type, writable) = match capability {
            "notion.page.update" => ("pages", notion_writable_page_fields(&input["changes"])),
            "notion.database.update" => (
                "databases",
                notion_writable_database_fields(&input["changes"]),
            ),
            _ => {
                return Err(ConnectorError::ActionUnsupported {
                    adapter: self.id().into(),
                    capability: capability.into(),
                })
            }
        };
        if writable
            .as_object()
            .is_none_or(|changes| changes.is_empty())
        {
            return Err(ConnectorError::Notion(format!(
                "{capability} requires non-empty changes"
            )));
        }
        let target = input["target"]
            .as_str()
            .filter(|target| !target.is_empty())
            .ok_or_else(|| ConnectorError::Notion("object target is required".into()))?;
        let url = notion_object_url(source, object_type, target)?;
        let before = notion_object_json("GET", &url, credential, None)?;
        let after = notion_object_json("PATCH", &url, credential, Some(writable.clone()))?;
        Ok(ToolOutcome {
            status: ToolStatus::Succeeded,
            output: json!({
                "connector_id": connector_id,
                "object_id": target,
                "before": notion_writable_fields(&before, &writable),
                "after": notion_writable_fields(&after, &writable),
            }),
            effect: EffectConclusion::Applied,
        })
    }

    fn preview_action(
        &self,
        source: &str,
        capability: &str,
        target: &str,
        credential: Option<&ConnectorCredential>,
        _input: &Value,
    ) -> Result<Value> {
        let (object_type, writable): (&str, fn(&Value) -> Value) = match capability {
            "notion.page.update" => ("pages", notion_writable_page_fields),
            "notion.database.update" => ("databases", notion_writable_database_fields),
            _ => {
                return Err(ConnectorError::ActionUnsupported {
                    adapter: self.id().into(),
                    capability: capability.into(),
                })
            }
        };
        let url = notion_object_url(source, object_type, target)?;
        let current = notion_object_json("GET", &url, credential, None)?;
        Ok(writable(&current))
    }

    fn action_schema(&self, capability: &str) -> Option<Value> {
        matches!(capability, "notion.page.update" | "notion.database.update")
            .then(|| connector_action_schema(capability))
            .flatten()
    }

    fn verify_action(
        &self,
        source: &str,
        action: &ConnectorAction,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        let (object_type, expected) = match action.capability.as_str() {
            "notion.page.update" => (
                "pages",
                notion_writable_page_fields(&action.input["changes"]),
            ),
            "notion.database.update" => (
                "databases",
                notion_writable_database_fields(&action.input["changes"]),
            ),
            _ => {
                return Err(ConnectorError::ActionUnsupported {
                    adapter: self.id().into(),
                    capability: format!("{}#verify", action.capability),
                })
            }
        };
        let url = notion_object_url(source, object_type, &action.target)?;
        let current = notion_object_json("GET", &url, credential, None)?;
        let matches = expected.as_object().is_none_or(|fields| {
            fields
                .iter()
                .all(|(key, value)| current.get(key) == Some(value))
        });
        Ok(ToolOutcome {
            status: if matches {
                ToolStatus::Succeeded
            } else {
                ToolStatus::Failed
            },
            output: json!({
                "object": notion_writable_fields(&current, &expected),
                "matches_expected": matches,
            }),
            effect: if matches {
                EffectConclusion::Applied
            } else {
                EffectConclusion::NoEffect
            },
        })
    }

    fn compensate_action(
        &self,
        source: &str,
        action: &ConnectorAction,
        credential: Option<&ConnectorCredential>,
    ) -> Result<ToolOutcome> {
        let object_type = match action.capability.as_str() {
            "notion.page.update" => "pages",
            "notion.database.update" => "databases",
            _ => {
                return Err(ConnectorError::ActionUnsupported {
                    adapter: self.id().into(),
                    capability: format!("{}#compensate", action.capability),
                })
            }
        };
        let before = action
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.output.get("before"))
            .cloned()
            .filter(|before| before.as_object().is_some_and(|before| !before.is_empty()))
            .ok_or_else(|| {
                ConnectorError::Notion(
                    "Notion compensation requires the pre-action snapshot".into(),
                )
            })?;
        let url = notion_object_url(source, object_type, &action.target)?;
        let restored = notion_object_json("PATCH", &url, credential, Some(before.clone()))?;
        Ok(ToolOutcome {
            status: ToolStatus::Succeeded,
            output: json!({
                "object": notion_writable_fields(&restored, &before),
                "restored": true,
            }),
            effect: EffectConclusion::Compensated,
        })
    }
}

pub fn pull_calendar(
    connector_id: ConnectorId,
    source: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<ConnectorPullPage> {
    let content = load_source(source, None, &[])?;
    let cursor = format!("calendar:{:016x}", stable_hash(&content));
    let observations = normalize_calendar(&connector_id, &content, window_start, window_end)?;
    Ok(ConnectorPullPage {
        stream_id: "calendar-events".into(),
        opaque_cursor: cursor,
        overlap_until: Some(window_start),
        continuation: None,
        observations,
    })
}

pub fn read_calendar(
    connector_id: ConnectorId,
    source: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<ConnectorObservation>> {
    Ok(pull_calendar(connector_id, source, window_start, window_end)?.observations)
}

fn pull_google_calendar(
    connector_id: ConnectorId,
    source: &str,
    cursor: Option<&SyncCursor>,
    credential: Option<&ConnectorCredential>,
) -> Result<ConnectorPullPage> {
    let mut url = format!(
        "{}?singleEvents=true&showDeleted=true&maxResults=2500",
        source.trim_end_matches('/')
    );
    if let Some(page_token) = cursor.and_then(|cursor| cursor.continuation.as_deref()) {
        url.push_str(&format!("&pageToken={}", form_encode(page_token)));
    } else if let Some(sync_token) =
        cursor.and_then(|cursor| cursor.opaque_cursor.strip_prefix("google-sync:"))
    {
        url.push_str(&format!("&syncToken={}", form_encode(sync_token)));
    }
    let payload = google_calendar_json("GET", &url, credential, None)?;
    let items = payload
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let observations = items
        .iter()
        .filter_map(|event| normalize_google_calendar_event(&connector_id, event))
        .collect::<Vec<_>>();
    let continuation = payload
        .get("nextPageToken")
        .and_then(Value::as_str)
        .map(str::to_string);
    let opaque_cursor = payload
        .get("nextSyncToken")
        .and_then(Value::as_str)
        .map(|token| format!("google-sync:{token}"))
        .unwrap_or_else(|| format!("google-calendar:{:016x}", stable_hash(&payload.to_string())));
    Ok(ConnectorPullPage {
        stream_id: "google-calendar-events".into(),
        opaque_cursor,
        overlap_until: None,
        continuation,
        observations,
    })
}

fn normalize_google_calendar_event(
    connector_id: &ConnectorId,
    event: &Value,
) -> Option<ConnectorObservation> {
    let id = string_at(event, &["id"])?;
    let status = string_at(event, &["status"]).unwrap_or_else(|| "confirmed".into());
    let summary = string_at(event, &["summary"]).unwrap_or_else(|| "Untitled event".into());
    let start = string_at(event, &["start", "dateTime"])
        .or_else(|| string_at(event, &["start", "date"]))
        .and_then(|value| parse_datetime_flexible(&value))
        .unwrap_or_else(Utc::now);
    let end = string_at(event, &["end", "dateTime"])
        .or_else(|| string_at(event, &["end", "date"]))
        .and_then(|value| parse_datetime_flexible(&value));
    let updated = string_at(event, &["updated"])
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or(start);
    let source_version = format!(
        "{:016x}",
        stable_hash(&format!(
            "{id}\0{summary}\0{start:?}\0{end:?}\0{status}\0{updated:?}"
        ))
    );
    let deleted = status == "cancelled";
    Some(ConnectorObservation {
        connector_id: connector_id.clone(),
        entity_id: id.clone(),
        source_version: source_version.clone(),
        dedupe_key: format!("google-calendar:{connector_id}:{id}:{source_version}"),
        source: "calendar".into(),
        summary: if deleted {
            format!("日程取消：{summary}")
        } else {
            format!("日程：{summary}（{}）", start.to_rfc3339())
        },
        occurred_at: start,
        valid_until: end,
        entity_kind: "calendar_event".into(),
        canonical_url: string_at(event, &["htmlLink"]),
        object_kind: ObjectKind::CalendarEvent,
        change: if deleted {
            ObservationChange::Delete
        } else {
            ObservationChange::Upsert
        },
        actor: string_at(event, &["organizer", "displayName"])
            .or_else(|| string_at(event, &["organizer", "email"])),
        attributes: json!({
            "title": summary,
            "description": event.get("description"),
            "location": event.get("location"),
            "status": status,
            "start": event.get("start"),
            "end": event.get("end"),
            "attendees": event.get("attendees"),
            "updated": updated.to_rfc3339(),
        }),
        evidence_ref: string_at(event, &["htmlLink"]),
        sensitivity: "internal".into(),
        source_trust: "provider_read_only".into(),
        correlations: vec![],
    })
}

fn parse_datetime_flexible(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date| Utc.from_utc_datetime(&date))
        })
}

fn google_event_url(source: &str, event_id: &str) -> Result<String> {
    let source = source.trim_end_matches('/');
    if !(source.starts_with("https://")
        || source.starts_with("http://127.0.0.1:")
        || source.starts_with("http://localhost:"))
    {
        return Err(ConnectorError::Calendar(
            "Google Calendar API source must use HTTPS or loopback HTTP for tests".into(),
        ));
    }
    Ok(format!("{source}/{}", form_encode(event_id)))
}

fn writable_google_event_fields(event: &Value) -> Value {
    const FIELDS: &[&str] = &[
        "summary",
        "description",
        "location",
        "start",
        "end",
        "status",
        "visibility",
        "transparency",
        "attendees",
        "recurrence",
        "reminders",
        "colorId",
    ];
    let mut writable = serde_json::Map::new();
    for field in FIELDS {
        if let Some(value) = event.get(*field) {
            writable.insert((*field).into(), value.clone());
        }
    }
    Value::Object(writable)
}

fn connector_action_schema(capability: &str) -> Option<Value> {
    let schema = match capability {
        "calendar.event.update" => json!({
            "type": "object",
            "title": "更新日历事件",
            "target_label": "Event ID",
            "preview_supported": true,
            "fields": [
                {"path":"changes.summary","title":"标题","kind":"text"},
                {"path":"changes.description","title":"描述","kind":"multiline"},
                {"path":"changes.location","title":"地点","kind":"text"},
                {"path":"changes.start","title":"开始时间","kind":"datetime"},
                {"path":"changes.end","title":"结束时间","kind":"datetime"},
                {
                    "path":"changes.status",
                    "title":"状态",
                    "kind":"enum",
                    "options":[
                        {"value":"confirmed","label":"已确认"},
                        {"value":"tentative","label":"暂定"},
                        {"value":"cancelled","label":"已取消"}
                    ]
                },
                {
                    "path":"changes.visibility",
                    "title":"可见性",
                    "kind":"enum",
                    "options":[
                        {"value":"default","label":"默认"},
                        {"value":"public","label":"公开"},
                        {"value":"private","label":"私密"},
                        {"value":"confidential","label":"机密"}
                    ]
                }
            ]
        }),
        "linear.issue.update" => json!({
            "type": "object",
            "title": "更新 Linear Issue",
            "target_label": "Issue ID",
            "preview_supported": true,
            "fields": [
                {"path":"changes.title","title":"标题","kind":"text"},
                {"path":"changes.description","title":"描述","kind":"multiline"},
                {"path":"changes.dueDate","title":"截止日期","kind":"date"},
                {
                    "path":"changes.priority",
                    "title":"优先级",
                    "kind":"enum",
                    "options":[
                        {"value":0,"label":"无"},
                        {"value":1,"label":"低"},
                        {"value":2,"label":"中"},
                        {"value":3,"label":"高"},
                        {"value":4,"label":"紧急"}
                    ]
                },
                {"path":"changes.assigneeId","title":"Assignee ID","kind":"text"},
                {"path":"changes.projectId","title":"Project ID","kind":"text"},
                {"path":"changes.stateId","title":"State ID","kind":"text"},
                {"path":"changes.milestoneId","title":"Milestone ID","kind":"text"}
            ]
        }),
        "linear.issue.comment" => json!({
            "type": "object",
            "title": "评论 Linear Issue",
            "target_label": "Issue ID",
            "preview_supported": true,
            "fields": [
                {"path":"body","title":"评论内容","kind":"multiline","required":true}
            ]
        }),
        "notion.page.update" => json!({
            "type": "object",
            "title": "更新 Notion 页面",
            "target_label": "Page ID",
            "preview_supported": true,
            "fields": [
                {"path":"changes.properties","title":"页面属性","kind":"json"},
                {"path":"changes.archived","title":"归档状态","kind":"boolean"},
                {"path":"changes.icon","title":"图标","kind":"json"},
                {"path":"changes.cover","title":"封面","kind":"json"}
            ]
        }),
        "notion.database.update" => json!({
            "type": "object",
            "title": "更新 Notion 数据库",
            "target_label": "Database ID",
            "preview_supported": true,
            "fields": [
                {"path":"changes.title","title":"数据库标题","kind":"json"},
                {"path":"changes.description","title":"数据库描述","kind":"json"},
                {"path":"changes.archived","title":"归档状态","kind":"boolean"}
            ]
        }),
        _ => return None,
    };
    Some(schema)
}

fn google_calendar_json(
    method: &str,
    url: &str,
    credential: Option<&ConnectorCredential>,
    body: Option<Value>,
) -> Result<Value> {
    let credential = credential
        .ok_or_else(|| ConnectorError::Calendar("Google Calendar requires OAuth".into()))?;
    let request = match method {
        "GET" => ureq::get(url),
        "PATCH" => ureq::patch(url),
        _ => {
            return Err(ConnectorError::Calendar(format!(
                "unsupported Google Calendar method {method}"
            )))
        }
    }
    .set("Accept", "application/json")
    .set("Authorization", &authorization_header(credential));
    let response = if let Some(body) = body {
        request
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
    } else {
        request.call()
    }
    .map_err(map_google_calendar_http_error)?;
    let content = response
        .into_string()
        .map_err(|error| ConnectorError::Http(error.to_string()))?;
    serde_json::from_str(&content)
        .ok()
        .or_else(|| (!content.trim().is_empty()).then_some(json!({"raw": content})))
        .ok_or_else(|| ConnectorError::Calendar("empty Google Calendar response".into()))
}

fn map_google_calendar_http_error(error: ureq::Error) -> ConnectorError {
    match error {
        ureq::Error::Status(429, response) => ConnectorError::RateLimited {
            retry_after_seconds: response
                .header("Retry-After")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30),
        },
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|payload| {
                    payload
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.chars().take(512).collect());
            ConnectorError::Calendar(format!("HTTP {status}: {message}"))
        }
        other => ConnectorError::Http(other.to_string()),
    }
}

fn normalize_calendar(
    connector_id: &ConnectorId,
    content: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<ConnectorObservation>> {
    let reader = IcalParser::new(content.as_bytes());
    let mut observations = Vec::new();
    for calendar in reader {
        let calendar = calendar.map_err(|error| ConnectorError::Calendar(error.to_string()))?;
        for event in calendar.events {
            let property = |name: &str| {
                event
                    .properties
                    .iter()
                    .find(|property| property.name == name)
                    .and_then(|property| property.value.clone())
            };
            let Some(summary) = property("SUMMARY") else {
                continue;
            };
            let Some(start) = property("DTSTART").and_then(|value| parse_datetime(&value)) else {
                continue;
            };
            if start < window_start || start > window_end {
                continue;
            }
            let end = property("DTEND").and_then(|value| parse_datetime(&value));
            let categories = property("CATEGORIES")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let attendees = event
                .properties
                .iter()
                .filter(|property| property.name == "ATTENDEE")
                .filter_map(|property| property.value.as_deref())
                .map(|value| {
                    value
                        .strip_prefix("mailto:")
                        .unwrap_or(value)
                        .trim()
                        .to_string()
                })
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            let uid = property("UID").unwrap_or_else(|| format!("{summary}:{start}"));
            let source_version = format!(
                "{:016x}",
                stable_hash(&format!("{summary}\0{start}\0{end:?}"))
            );
            let mut correlations = direct_project_hints(&categories);
            correlations.extend(
                attendees
                    .iter()
                    .filter_map(|attendee| identity_hint(CorrelationKind::Contact, attendee)),
            );
            dedupe_project_hints(&mut correlations);
            observations.push(ConnectorObservation {
                connector_id: connector_id.clone(),
                entity_id: uid.clone(),
                source_version: source_version.clone(),
                dedupe_key: format!("calendar:{connector_id}:{uid}:{source_version}"),
                source: "calendar".into(),
                summary: format!("日程：{summary}（{}）", start.to_rfc3339()),
                occurred_at: start,
                valid_until: end,
                entity_kind: "calendar_event".into(),
                canonical_url: None,
                object_kind: ObjectKind::CalendarEvent,
                change: ObservationChange::Upsert,
                actor: None,
                attributes: json!({
                    "title": summary,
                    "categories": categories,
                }),
                evidence_ref: None,
                sensitivity: "internal".into(),
                source_trust: "deterministic_read_only".into(),
                correlations,
            });
        }
    }
    observations
        .sort_by_key(|observation| (observation.occurred_at, observation.dedupe_key.clone()));
    Ok(observations)
}

pub fn pull_linear(
    connector_id: ConnectorId,
    source: &str,
    credential: Option<&ConnectorCredential>,
) -> Result<ConnectorPullPage> {
    let content = load_source(source, credential, &[])?;
    let payload: Value = serde_json::from_str(&content)
        .map_err(|error| ConnectorError::Linear(error.to_string()))?;
    Ok(ConnectorPullPage {
        stream_id: "linear-issues".into(),
        opaque_cursor: format!("linear:{:016x}", stable_hash(&content)),
        overlap_until: None,
        continuation: None,
        observations: normalize_linear_payload(&connector_id, &payload)?,
    })
}

fn pull_linear_graphql(
    connector_id: ConnectorId,
    source: &str,
    cursor: Option<&SyncCursor>,
    credential: Option<&ConnectorCredential>,
) -> Result<ConnectorPullPage> {
    let continuation = cursor.and_then(|cursor| cursor.continuation.clone());
    let payload = linear_graphql(
        source,
        credential,
        json!({
            "query": "query Issues($after: String) { issues(first: 50, after: $after) { nodes { id identifier title state { name type } updatedAt dueDate url assignee { name } project { name } labels { nodes { name } } blockedBy { nodes { id } } priority } pageInfo { hasNextPage endCursor } } }",
            "variables": { "after": continuation },
        }),
    )?;
    let next = payload
        .pointer("/data/issues/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| {
            payload
                .pointer("/data/issues/pageInfo/endCursor")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .flatten();
    Ok(ConnectorPullPage {
        stream_id: "linear-issues".into(),
        opaque_cursor: format!("linear-page:{:016x}", stable_hash(&payload.to_string())),
        overlap_until: None,
        continuation: next,
        observations: normalize_linear_payload(&connector_id, &payload)?,
    })
}

pub fn pull_notion(
    connector_id: ConnectorId,
    source: &str,
    credential: Option<&ConnectorCredential>,
) -> Result<ConnectorPullPage> {
    pull_notion_with_cursor(connector_id, source, None, credential)
}

fn pull_notion_with_cursor(
    connector_id: ConnectorId,
    source: &str,
    cursor: Option<&SyncCursor>,
    credential: Option<&ConnectorCredential>,
) -> Result<ConnectorPullPage> {
    let continuation = cursor.and_then(|cursor| cursor.continuation.clone());
    let content = if source.starts_with("https://api.notion.com/") {
        notion_api_request(source, continuation.as_deref(), credential)?
    } else {
        load_source(source, credential, &[("Notion-Version", "2022-06-28")])?
    };
    let payload: Value = serde_json::from_str(&content)
        .map_err(|error| ConnectorError::Notion(error.to_string()))?;
    if source.starts_with("https://api.notion.com/")
        && !payload.get("results").is_some_and(Value::is_array)
    {
        return Err(ConnectorError::Notion(
            "Notion API response is missing the results array".into(),
        ));
    }
    let next = source
        .starts_with("https://api.notion.com/")
        .then(|| {
            payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                .then(|| {
                    payload
                        .get("next_cursor")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .flatten()
        })
        .flatten();
    Ok(ConnectorPullPage {
        stream_id: "notion-pages".into(),
        opaque_cursor: format!("notion:{:016x}", stable_hash(&content)),
        overlap_until: None,
        continuation: next,
        observations: normalize_notion_payload(&connector_id, &payload)?,
    })
}

fn notion_api_request(
    source: &str,
    start_cursor: Option<&str>,
    credential: Option<&ConnectorCredential>,
) -> Result<String> {
    let credential = credential
        .ok_or_else(|| ConnectorError::Notion("Notion API requires an API token".into()))?;
    ureq::post(source)
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .set("Notion-Version", "2022-06-28")
        .set("Authorization", &authorization_header(credential))
        .send_string(&notion_request_body(source, start_cursor).to_string())
        .map_err(map_notion_http_error)?
        .into_string()
        .map_err(|error| ConnectorError::Http(error.to_string()))
}

fn notion_object_url(source: &str, object_type: &str, object_id: &str) -> Result<String> {
    let scheme_end = source
        .find("://")
        .ok_or_else(|| ConnectorError::Notion("invalid Notion API source".into()))?
        + 3;
    let path_start = source[scheme_end..]
        .find('/')
        .map(|offset| scheme_end + offset)
        .unwrap_or(source.len());
    let origin = &source[..path_start];
    if !(origin.starts_with("https://")
        || origin.starts_with("http://127.0.0.1:")
        || origin.starts_with("http://localhost:"))
    {
        return Err(ConnectorError::Notion(
            "Notion writes require HTTPS or a loopback test endpoint".into(),
        ));
    }
    Ok(format!(
        "{origin}/v1/{object_type}/{}",
        form_encode(object_id)
    ))
}

fn notion_object_json(
    method: &str,
    url: &str,
    credential: Option<&ConnectorCredential>,
    body: Option<Value>,
) -> Result<Value> {
    let credential = credential
        .ok_or_else(|| ConnectorError::Notion("Notion API requires an API token".into()))?;
    let request = match method {
        "GET" => ureq::get(url),
        "PATCH" => ureq::patch(url),
        _ => {
            return Err(ConnectorError::Notion(format!(
                "unsupported Notion method {method}"
            )))
        }
    }
    .set("Accept", "application/json")
    .set("Notion-Version", "2022-06-28")
    .set("Authorization", &authorization_header(credential));
    let response = if let Some(body) = body {
        request
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
    } else {
        request.call()
    }
    .map_err(map_notion_http_error)?;
    let content = response
        .into_string()
        .map_err(|error| ConnectorError::Http(error.to_string()))?;
    serde_json::from_str(&content).map_err(|error| ConnectorError::Notion(error.to_string()))
}

fn notion_writable_page_fields(page: &Value) -> Value {
    notion_writable_fields(
        page,
        &json!({
            "properties": null,
            "archived": null,
            "icon": null,
            "cover": null,
        }),
    )
}

fn notion_writable_database_fields(database: &Value) -> Value {
    notion_writable_fields(
        database,
        &json!({
            "title": null,
            "description": null,
            "properties": null,
            "archived": null,
            "icon": null,
            "cover": null,
        }),
    )
}

fn notion_writable_fields(source: &Value, expected: &Value) -> Value {
    let mut writable = serde_json::Map::new();
    for field in expected
        .as_object()
        .into_iter()
        .flat_map(|fields| fields.keys())
    {
        if let Some(value) = source.get(field) {
            writable.insert(field.clone(), value.clone());
        }
    }
    Value::Object(writable)
}

fn notion_request_body(source: &str, start_cursor: Option<&str>) -> Value {
    let mut body = json!({
        "page_size": 100,
    });
    if let Some(start_cursor) = start_cursor {
        body["start_cursor"] = json!(start_cursor);
    }
    if source.trim_end_matches('/').ends_with("/search") {
        body["query"] = json!("");
        body["sort"] = json!({
            "direction": "descending",
            "timestamp": "last_edited_time"
        });
    }
    body
}

fn map_notion_http_error(error: ureq::Error) -> ConnectorError {
    match error {
        ureq::Error::Status(429, response) => ConnectorError::RateLimited {
            retry_after_seconds: response
                .header("Retry-After")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30),
        },
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|payload| {
                    payload
                        .get("message")
                        .or_else(|| payload.get("code"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.chars().take(512).collect());
            ConnectorError::Notion(format!("HTTP {status}: {message}"))
        }
        other => ConnectorError::Http(other.to_string()),
    }
}

fn hydrate_linear_issue(
    source: &str,
    credential: Option<&ConnectorCredential>,
    issue_id: &str,
    original_payload: &Value,
) -> Result<Value> {
    let source_payload = if source.starts_with("https://api.linear.app/") {
        linear_graphql(
            source,
            credential,
            json!({
                "query": "query Issue($id: String!) { issue(id: $id) { id identifier title state { name type } updatedAt dueDate url assignee { name } project { name } labels { nodes { name } } blockedBy { nodes { id } } priority } }",
                "variables": { "id": issue_id },
            }),
        )?
    } else {
        serde_json::from_str::<Value>(&load_source(source, credential, &[])?)
            .map_err(|error| ConnectorError::Linear(error.to_string()))?
    };
    let issue = linear_issue_nodes(&source_payload)
        .into_iter()
        .find(|issue| string_at(issue, &["id"]).as_deref() == Some(issue_id))
        .cloned()
        .ok_or_else(|| {
            ConnectorError::Linear(format!(
                "provider payload did not contain issue {issue_id} for hydration"
            ))
        })?;
    Ok(json!({
        "action": original_payload.get("action").cloned().unwrap_or(Value::Null),
        "type": original_payload.get("type").cloned().unwrap_or(Value::Null),
        "data": issue,
    }))
}

fn linear_graphql(
    source: &str,
    credential: Option<&ConnectorCredential>,
    payload: Value,
) -> Result<Value> {
    let credential = credential
        .ok_or_else(|| ConnectorError::Linear("Linear GraphQL requires an API token".into()))?;
    let authorization = authorization_header(credential);
    let content = ureq::post(source)
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .set("Authorization", &authorization)
        .send_string(&payload.to_string())
        .map_err(map_http_error)?
        .into_string()
        .map_err(|error| ConnectorError::Http(error.to_string()))?;
    let payload: Value = serde_json::from_str(&content)
        .map_err(|error| ConnectorError::Linear(error.to_string()))?;
    if let Some(errors) = payload.get("errors") {
        return Err(ConnectorError::Linear(errors.to_string()));
    }
    Ok(payload)
}

fn linear_fetch_issue(
    source: &str,
    credential: Option<&ConnectorCredential>,
    issue_id: &str,
) -> Result<Value> {
    let payload = linear_graphql(
        source,
        credential,
        json!({
            "query": "query Issue($id: String!) { issue(id: $id) { id title description dueDate priority assigneeId projectId stateId milestoneId } }",
            "variables": {"id": issue_id},
        }),
    )?;
    payload
        .pointer("/data/issue")
        .cloned()
        .ok_or_else(|| ConnectorError::Linear(format!("issue {issue_id} was not returned")))
}

fn linear_update_issue(
    source: &str,
    credential: Option<&ConnectorCredential>,
    issue_id: &str,
    changes: Value,
) -> Result<Value> {
    let payload = linear_graphql(
        source,
        credential,
        json!({
            "query": "mutation UpdateIssue($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success issue { id title description dueDate priority assigneeId projectId stateId milestoneId } } }",
            "variables": {"id": issue_id, "input": changes},
        }),
    )?;
    if payload
        .pointer("/data/issueUpdate/success")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(ConnectorError::Linear(format!(
            "issue update failed: {payload}"
        )));
    }
    payload
        .pointer("/data/issueUpdate/issue")
        .cloned()
        .ok_or_else(|| ConnectorError::Linear("updated issue was not returned".into()))
}

fn linear_create_comment(
    source: &str,
    credential: Option<&ConnectorCredential>,
    issue_id: &str,
    body: &str,
) -> Result<Value> {
    let payload = linear_graphql(
        source,
        credential,
        json!({
            "query": "mutation CreateComment($input: CommentCreateInput!) { commentCreate(input: $input) { success comment { id body issue { id } } } }",
            "variables": {"input": {"issueId": issue_id, "body": body}},
        }),
    )?;
    if payload
        .pointer("/data/commentCreate/success")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(ConnectorError::Linear(format!(
            "comment creation failed: {payload}"
        )));
    }
    payload
        .pointer("/data/commentCreate/comment")
        .cloned()
        .ok_or_else(|| ConnectorError::Linear("created comment was not returned".into()))
}

fn linear_fetch_comment(
    source: &str,
    credential: Option<&ConnectorCredential>,
    comment_id: &str,
) -> Result<Value> {
    let payload = linear_graphql(
        source,
        credential,
        json!({
            "query": "query Comment($id: String!) { comment(id: $id) { id body issue { id } } }",
            "variables": {"id": comment_id},
        }),
    )?;
    payload
        .pointer("/data/comment")
        .cloned()
        .ok_or_else(|| ConnectorError::Linear(format!("comment {comment_id} was not returned")))
}

fn linear_delete_comment(
    source: &str,
    credential: Option<&ConnectorCredential>,
    comment_id: &str,
) -> Result<()> {
    let payload = linear_graphql(
        source,
        credential,
        json!({
            "query": "mutation DeleteComment($id: String!) { commentDelete(id: $id) { success } }",
            "variables": {"id": comment_id},
        }),
    )?;
    if payload
        .pointer("/data/commentDelete/success")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(ConnectorError::Linear(format!(
            "comment deletion failed: {payload}"
        )));
    }
    Ok(())
}

fn linear_writable_issue_fields(issue: &Value) -> Value {
    const FIELDS: &[&str] = &[
        "title",
        "description",
        "dueDate",
        "priority",
        "assigneeId",
        "projectId",
        "stateId",
        "milestoneId",
    ];
    let mut writable = serde_json::Map::new();
    for field in FIELDS {
        if let Some(value) = issue.get(*field) {
            writable.insert((*field).into(), value.clone());
        }
    }
    Value::Object(writable)
}

pub fn normalize_notion_payload(
    connector_id: &ConnectorId,
    payload: &Value,
) -> Result<Vec<ConnectorObservation>> {
    let mut observations = notion_nodes(payload)
        .into_iter()
        .filter_map(|node| normalize_notion_node(connector_id, node).transpose())
        .collect::<Result<Vec<_>>>()?;
    observations
        .sort_by_key(|observation| (observation.occurred_at, observation.dedupe_key.clone()));
    Ok(observations)
}

fn normalize_notion_node(
    connector_id: &ConnectorId,
    node: &Value,
) -> Result<Option<ConnectorObservation>> {
    let Some(id) = string_at(node, &["id"]) else {
        return Ok(None);
    };
    let properties = node.get("properties").and_then(Value::as_object);
    let property_facts = notion_property_facts(properties);
    let title = notion_title(node, properties).unwrap_or_else(|| format!("Notion {id}"));
    let url = string_at(node, &["url"]);
    let parent_type = string_at(node, &["parent", "type"]);
    let parent_id = string_at(node, &["parent", "database_id"])
        .or_else(|| string_at(node, &["parent", "page_id"]))
        .or_else(|| string_at(node, &["parent", "workspace"]));
    let database_id = string_at(node, &["parent", "database_id"]);
    let archived = node
        .get("archived")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || node
            .get("in_trash")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let updated_at = string_at(node, &["last_edited_time"])
        .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let labels = property_facts
        .iter()
        .flat_map(
            |(name, fact)| match fact.get("type").and_then(Value::as_str) {
                Some("status" | "select") => fact
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .into_iter()
                    .collect::<Vec<_>>(),
                Some("multi_select") => fact
                    .get("values")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => {
                    let lower = name.to_ascii_lowercase();
                    if lower.contains("label") || lower.contains("tag") {
                        fact.get("value")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .into_iter()
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
            },
        )
        .collect::<Vec<_>>();
    let state_type = property_facts.iter().find_map(|(_, fact)| {
        fact.get("group")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let due_at = property_facts.iter().find_map(|(name, fact)| {
        let lower = name.to_ascii_lowercase();
        (lower.contains("due")
            || lower.contains("deadline")
            || lower.contains("截止")
            || lower.contains("日期"))
        .then(|| {
            fact.get("start")
                .and_then(Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
        })
        .flatten()
    });
    let blocked_by = property_facts
        .iter()
        .filter(|(name, _)| name.to_ascii_lowercase().contains("block"))
        .flat_map(|(_, fact)| {
            fact.get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let work_item =
        database_id.is_some() || state_type.is_some() || due_at.is_some() || !blocked_by.is_empty();
    let object_kind = if work_item {
        ObjectKind::WorkItem
    } else {
        ObjectKind::Document
    };
    let actor = string_at(node, &["last_edited_by", "id"]);
    let source_version = format!(
        "{:016x}",
        stable_hash(&format!(
            "{id}\0{title}\0{updated_at:?}\0{archived}\0{property_facts:?}"
        ))
    );
    let entity_kind = if work_item {
        "notion_database_row"
    } else {
        "notion_page"
    };
    let mut project_labels = property_facts
        .iter()
        .filter(|(name, _)| {
            let name = name.to_ascii_lowercase();
            name.contains("project") || name.contains("workstream") || name.contains("项目")
        })
        .flat_map(|(_, fact)| {
            fact.get("value")
                .and_then(Value::as_str)
                .map(str::to_string)
                .into_iter()
                .chain(
                    fact.get("values")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string),
                )
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if project_labels.is_empty() {
        if let Some(database_id) = &database_id {
            project_labels.push(format!("Notion Database {database_id}"));
        }
    }
    let mut correlations = direct_project_hints(&project_labels);
    correlations.extend(notion_identity_hints(&property_facts));
    dedupe_project_hints(&mut correlations);
    Ok(Some(ConnectorObservation {
        connector_id: connector_id.clone(),
        entity_id: id.clone(),
        source_version: source_version.clone(),
        dedupe_key: format!("notion:{connector_id}:{id}:{source_version}"),
        source: "notion".into(),
        summary: format!("Notion: {title}（{entity_kind}）"),
        occurred_at: updated_at,
        valid_until: None,
        entity_kind: entity_kind.into(),
        canonical_url: url.clone(),
        object_kind,
        change: if archived {
            ObservationChange::Delete
        } else {
            ObservationChange::Upsert
        },
        actor: actor.clone(),
        attributes: json!({
            "page_id": id,
            "title": title,
            "parent_type": parent_type,
            "parent_id": parent_id,
            "database_id": database_id,
            "properties": property_facts,
            "labels": labels,
            "state_type": state_type,
            "due_at": due_at.map(|value| value.to_rfc3339()),
            "blocked_by": blocked_by,
            "last_edited_by": actor,
            "last_edited_time": updated_at.to_rfc3339(),
            "url": url,
        }),
        evidence_ref: string_at(node, &["url"]),
        sensitivity: "internal".into(),
        source_trust: "provider_read_only".into(),
        correlations,
    }))
}

fn notion_nodes(payload: &Value) -> Vec<&Value> {
    for value in [
        &payload["results"],
        &payload["data"]["page"],
        &payload["page"],
    ] {
        if let Some(items) = value.as_array() {
            return items.iter().collect();
        }
        if value.is_object() {
            return vec![value];
        }
    }
    if payload.get("id").is_some() {
        return vec![payload];
    }
    vec![]
}

fn notion_title(
    node: &Value,
    properties: Option<&serde_json::Map<String, Value>>,
) -> Option<String> {
    if let Some(title) = node.get("title").and_then(notion_rich_text) {
        return Some(title);
    }
    properties.and_then(|properties| {
        properties.values().find_map(|property| {
            matches!(property.get("type").and_then(Value::as_str), Some("title"))
                .then(|| {
                    property
                        .get("title")
                        .and_then(notion_rich_text)
                        .or_else(|| property.get("rich_text").and_then(notion_rich_text))
                })
                .flatten()
        })
    })
}

fn notion_rich_text(value: &Value) -> Option<String> {
    let items = value.as_array()?;
    let text = items
        .iter()
        .filter_map(|item| {
            item.get("plain_text").and_then(Value::as_str).or_else(|| {
                item.get("text")
                    .and_then(|text| text.get("content"))
                    .and_then(Value::as_str)
            })
        })
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

fn notion_property_facts(
    properties: Option<&serde_json::Map<String, Value>>,
) -> serde_json::Map<String, Value> {
    let mut facts = serde_json::Map::new();
    for (name, property) in properties.into_iter().flatten() {
        let kind = property.get("type").and_then(Value::as_str);
        let fact = match kind {
            Some("title") => json!({
                "type": "title",
                "value": property.get("title").and_then(notion_rich_text)
            }),
            Some("rich_text") => json!({
                "type": "rich_text",
                "value": property.get("rich_text").and_then(notion_rich_text)
            }),
            Some("status") => json!({
                "type": "status",
                "value": property.pointer("/status/name"),
                "group": property.pointer("/status/group")
            }),
            Some("select") => json!({
                "type": "select",
                "value": property.pointer("/select/name")
            }),
            Some("multi_select") => json!({
                "type": "multi_select",
                "values": property
                    .pointer("/multi_select")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.get("name").cloned())
                    .collect::<Vec<_>>()
            }),
            Some("date") => json!({
                "type": "date",
                "start": property.pointer("/date/start"),
                "end": property.pointer("/date/end")
            }),
            Some("relation") => json!({
                "type": "relation",
                "values": property
                    .pointer("/relation")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.get("id").cloned())
                    .collect::<Vec<_>>()
            }),
            Some("checkbox") => json!({
                "type": "checkbox",
                "value": property.get("checkbox")
            }),
            Some("number") => json!({
                "type": "number",
                "value": property.get("number")
            }),
            _ => json!({ "type": kind }),
        };
        facts.insert(name.clone(), fact);
    }
    facts
}

fn notion_identity_hints(properties: &serde_json::Map<String, Value>) -> Vec<CorrelationHint> {
    let mut hints = Vec::new();
    for (name, fact) in properties {
        let lower = name.to_ascii_lowercase();
        let kind = if lower.contains("customer") || lower.contains("客户") {
            Some(CorrelationKind::Customer)
        } else if lower.contains("contact")
            || lower.contains("assignee")
            || lower.contains("owner")
            || lower.contains("联系人")
        {
            Some(CorrelationKind::Contact)
        } else if lower.contains("thread")
            || lower.contains("conversation")
            || lower.contains("线程")
        {
            Some(CorrelationKind::Thread)
        } else if lower.contains("milestone")
            || lower.contains("release")
            || lower.contains("里程碑")
        {
            Some(CorrelationKind::Milestone)
        } else {
            None
        };
        let Some(kind) = kind else {
            continue;
        };
        for label in notion_fact_values(fact) {
            if let Some(hint) = identity_hint(kind, &label) {
                hints.push(hint);
            }
        }
    }
    hints
}

fn notion_fact_values(fact: &Value) -> Vec<String> {
    if let Some(value) = fact.get("value").and_then(Value::as_str) {
        return vec![value.to_string()];
    }
    fact.get("values")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

pub fn normalize_linear_payload(
    connector_id: &ConnectorId,
    payload: &Value,
) -> Result<Vec<ConnectorObservation>> {
    let issues = linear_issue_nodes(payload);
    let mut observations = issues
        .into_iter()
        .filter_map(|issue| normalize_linear_issue(connector_id, issue).transpose())
        .collect::<Result<Vec<_>>>()?;
    observations
        .sort_by_key(|observation| (observation.occurred_at, observation.dedupe_key.clone()));
    Ok(observations)
}

fn normalize_linear_issue(
    connector_id: &ConnectorId,
    issue: &Value,
) -> Result<Option<ConnectorObservation>> {
    let Some(id) = string_at(issue, &["id"]).or_else(|| string_at(issue, &["identifier"])) else {
        return Ok(None);
    };
    let Some(title) = string_at(issue, &["title"]) else {
        return Ok(None);
    };
    let identifier = string_at(issue, &["identifier"]).unwrap_or_else(|| id.clone());
    let state = string_at(issue, &["state", "name"]).unwrap_or_else(|| "unknown".into());
    let state_type = string_at(issue, &["state", "type"]).unwrap_or_else(|| "unstarted".into());
    let labels = string_array_at(issue, &["labels", "nodes"]);
    let blocked_by = string_array_at(issue, &["blockedBy", "nodes"]);
    let due_at = string_at(issue, &["dueDate"]).and_then(|raw| parse_date(&raw));
    let updated_at = string_at(issue, &["updatedAt"])
        .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let completed = matches!(state_type.as_str(), "completed" | "canceled");
    let assignee = string_at(issue, &["assignee", "name"])
        .or_else(|| string_at(issue, &["assignee", "displayName"]));
    let project = string_at(issue, &["project", "name"]);
    let milestone = string_at(issue, &["milestone", "name"]);
    let thread = string_at(issue, &["identifier"]).or_else(|| Some(identifier.clone()));
    let url = string_at(issue, &["url"]);
    let mut sorted_labels = labels.clone();
    sorted_labels.sort();
    let mut sorted_blockers = blocked_by.clone();
    sorted_blockers.sort();
    let source_version = format!(
        "{:016x}",
        stable_hash(&format!(
            "{id}\0{identifier}\0{title}\0{state}\0{state_type}\0{due_at:?}\0{sorted_labels:?}\0{sorted_blockers:?}\0{updated_at:?}\0{assignee:?}\0{project:?}\0{:?}\0{url:?}",
            issue.get("priority")
        ))
    );
    let due_text = due_at
        .map(|value| format!("，截止 {}", value.date_naive()))
        .unwrap_or_default();
    let blocked_text = if blocked_by.is_empty() {
        String::new()
    } else {
        format!("，被 {} 个事项阻塞", blocked_by.len())
    };
    let summary = format!("Linear {identifier}: {title}（{state}{due_text}{blocked_text}）");
    let mut correlations = project_hints_from_labels(&labels);
    if let Some(project) = &project {
        correlations.extend(direct_project_hints(std::slice::from_ref(project)));
    }
    if let Some(assignee) = &assignee {
        if let Some(hint) = identity_hint(CorrelationKind::Contact, assignee) {
            correlations.push(hint);
        }
    }
    if let Some(milestone) = &milestone {
        if let Some(hint) = identity_hint(CorrelationKind::Milestone, milestone) {
            correlations.push(hint);
        }
    }
    if let Some(thread) = &thread {
        if let Some(hint) = identity_hint(CorrelationKind::Thread, thread) {
            correlations.push(hint);
        }
    }
    dedupe_project_hints(&mut correlations);
    Ok(Some(ConnectorObservation {
        connector_id: connector_id.clone(),
        entity_id: id.clone(),
        source_version: source_version.clone(),
        dedupe_key: format!("linear:{connector_id}:{id}:{source_version}"),
        source: "linear".into(),
        summary,
        occurred_at: updated_at,
        valid_until: None,
        entity_kind: "linear_issue".into(),
        canonical_url: url.clone(),
        object_kind: ObjectKind::WorkItem,
        change: if completed {
            ObservationChange::Delete
        } else {
            ObservationChange::Upsert
        },
        actor: assignee.clone(),
        attributes: json!({
            "issue_id": id,
            "identifier": identifier,
            "title": title,
            "state": state,
            "state_type": state_type,
            "due_at": due_at.map(|value| value.to_rfc3339()),
            "project": project,
            "labels": labels,
            "assignee": assignee,
            "blocked_by": blocked_by,
            "priority": issue.get("priority").cloned().unwrap_or(Value::Null),
            "url": url,
        }),
        evidence_ref: string_at(issue, &["url"]),
        sensitivity: "internal".into(),
        source_trust: "provider_read_only".into(),
        correlations,
    }))
}

pub trait InterpretationRule: Send + Sync {
    fn id(&self) -> &'static str;
    fn interpret(&self, signal: &WorldSignal, now: DateTime<Utc>) -> Option<SignalInterpretation>;
}

pub struct SignalInterpreter {
    rules: Vec<Box<dyn InterpretationRule>>,
}

impl SignalInterpreter {
    pub fn builtin() -> Self {
        Self {
            rules: vec![
                Box::new(LegacySemanticRule),
                Box::new(WorkItemRule),
                Box::new(CalendarRule),
                Box::new(MilestoneRule),
                Box::new(FallbackRule),
            ],
        }
    }

    pub fn with_rules(rules: Vec<Box<dyn InterpretationRule>>) -> Self {
        Self { rules }
    }

    pub fn interpret(&self, signal: &WorldSignal, now: DateTime<Utc>) -> Vec<SignalInterpretation> {
        if signal.status != SignalStatus::Active {
            return vec![];
        }
        self.rules
            .iter()
            .find_map(|rule| rule.interpret(signal, now))
            .into_iter()
            .collect()
    }
}

struct LegacySemanticRule;

impl InterpretationRule for LegacySemanticRule {
    fn id(&self) -> &'static str {
        "legacy-semantic"
    }

    fn interpret(&self, signal: &WorldSignal, _now: DateTime<Utc>) -> Option<SignalInterpretation> {
        let meaning = match signal.legacy_signal_type? {
            SignalType::Commitment => SignalMeaning::Commitment,
            SignalType::WaitingExternal => SignalMeaning::WaitingExternal,
            SignalType::ActionItem | SignalType::Issue | SignalType::CalendarEvent => {
                SignalMeaning::ActionItem
            }
            SignalType::Message | SignalType::Page => SignalMeaning::Context,
        };
        Some(make_interpretation(
            self.id(),
            signal,
            meaning,
            vec!["legacy_signal_type".into()],
        ))
    }
}

struct WorkItemRule;

impl InterpretationRule for WorkItemRule {
    fn id(&self) -> &'static str {
        "work-item-v1"
    }

    fn interpret(&self, signal: &WorldSignal, _now: DateTime<Utc>) -> Option<SignalInterpretation> {
        if signal.object_kind != ObjectKind::WorkItem {
            return None;
        }
        let labels = interpretation_labels(signal);
        let blocked = signal
            .structured_facts
            .get("blocked_by")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty());
        let state_type = signal
            .structured_facts
            .get("state_type")
            .and_then(Value::as_str);
        let meaning = if matches!(state_type, Some("completed" | "canceled")) {
            SignalMeaning::Context
        } else if blocked
            || labels.iter().any(|label| {
                matches!(
                    label.to_ascii_lowercase().as_str(),
                    "waiting" | "waiting_external" | "blocked"
                )
            })
        {
            SignalMeaning::WaitingExternal
        } else if labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("commitment"))
        {
            SignalMeaning::Commitment
        } else if labels.iter().any(|label| {
            matches!(
                label.to_ascii_lowercase().as_str(),
                "risk" | "at_risk" | "urgent"
            )
        }) {
            SignalMeaning::Risk
        } else {
            SignalMeaning::ActionItem
        };
        let mut basis = Vec::new();
        if blocked {
            basis.push("blocked_by".into());
        }
        if labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("commitment"))
        {
            basis.push("label=commitment".into());
        }
        if labels.iter().any(|label| {
            matches!(
                label.to_ascii_lowercase().as_str(),
                "risk" | "at_risk" | "urgent"
            )
        }) {
            basis.push("label=risk".into());
        }
        if state_type.is_some() {
            basis.push("work_item_state".into());
        }
        Some(make_interpretation(self.id(), signal, meaning, basis))
    }
}

struct CalendarRule;

impl InterpretationRule for CalendarRule {
    fn id(&self) -> &'static str {
        "calendar-recency-v1"
    }

    fn interpret(&self, signal: &WorldSignal, now: DateTime<Utc>) -> Option<SignalInterpretation> {
        if signal.object_kind != ObjectKind::CalendarEvent {
            return None;
        }
        let horizon = now + Duration::hours(24);
        let meaning = if signal.occurred_at >= now
            && signal.occurred_at <= horizon
            && signal
                .valid_until
                .is_none_or(|valid_until| valid_until >= now)
        {
            SignalMeaning::ActionItem
        } else {
            SignalMeaning::Context
        };
        Some(make_interpretation(
            self.id(),
            signal,
            meaning,
            vec!["calendar_recency".into()],
        ))
    }
}

struct MilestoneRule;

impl InterpretationRule for MilestoneRule {
    fn id(&self) -> &'static str {
        "milestone-v1"
    }

    fn interpret(&self, signal: &WorldSignal, _now: DateTime<Utc>) -> Option<SignalInterpretation> {
        (signal.object_kind == ObjectKind::Milestone).then(|| {
            make_interpretation(
                self.id(),
                signal,
                SignalMeaning::Commitment,
                vec!["milestone".into()],
            )
        })
    }
}

struct FallbackRule;

impl InterpretationRule for FallbackRule {
    fn id(&self) -> &'static str {
        "fallback-v1"
    }

    fn interpret(&self, signal: &WorldSignal, _now: DateTime<Utc>) -> Option<SignalInterpretation> {
        Some(make_interpretation(
            self.id(),
            signal,
            SignalMeaning::Context,
            vec![],
        ))
    }
}

fn make_interpretation(
    rule_id: &str,
    signal: &WorldSignal,
    meaning: SignalMeaning,
    mut basis: Vec<String>,
) -> SignalInterpretation {
    let deadline = interpretation_deadline(signal);
    if deadline.is_some() && !basis.iter().any(|item| item == "deadline") {
        basis.push("deadline".into());
    }
    let confidence = match meaning {
        SignalMeaning::Commitment => 85,
        SignalMeaning::WaitingExternal => 80,
        SignalMeaning::ActionItem if signal.object_kind == ObjectKind::CalendarEvent => 65,
        SignalMeaning::ActionItem => 70,
        SignalMeaning::Risk => 75,
        SignalMeaning::Context => 0,
    };
    SignalInterpretation {
        signal_id: signal.id.clone(),
        meaning,
        confidence,
        basis,
        deadline,
        ruleset_version: format!("builtin/{rule_id}"),
    }
}

fn interpretation_labels(signal: &WorldSignal) -> Vec<String> {
    signal
        .structured_facts
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            value.as_str().map(str::to_string).or_else(|| {
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .collect()
}

fn interpretation_deadline(signal: &WorldSignal) -> Option<DateTime<Utc>> {
    if signal.object_kind == ObjectKind::CalendarEvent {
        return Some(signal.occurred_at);
    }
    signal
        .structured_facts
        .get("due_at")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn linear_issue_nodes(payload: &Value) -> Vec<&Value> {
    for value in [
        &payload["data"]["issues"]["nodes"],
        &payload["issues"],
        &payload["data"]["issue"],
        &payload["issue"],
    ] {
        if let Some(items) = value.as_array() {
            return items.iter().collect();
        }
        if value.is_object() {
            return vec![value];
        }
    }
    if payload["data"].is_object() && payload["data"].get("id").is_some() {
        return vec![&payload["data"]];
    }
    vec![payload]
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str().map(str::to_string)
}

fn string_array_at(value: &Value, path: &[&str]) -> Vec<String> {
    if path.len() == 2 {
        if let Some(items) = value.get(path[0]).and_then(Value::as_array) {
            return items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .or_else(|| item.get("name").and_then(Value::as_str).map(str::to_string))
                })
                .collect();
        }
    }
    let mut current = value;
    for segment in path {
        let Some(next) = current.get(*segment) else {
            return vec![];
        };
        current = next;
    }
    current
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str()
                .map(str::to_string)
                .or_else(|| item.get("name").and_then(Value::as_str).map(str::to_string))
        })
        .collect()
}

fn load_source(
    source: &str,
    credential: Option<&ConnectorCredential>,
    headers: &[(&str, &str)],
) -> Result<String> {
    if source.starts_with("https://") {
        let mut request = ureq::get(source).set("Accept", "application/json");
        if let Some(credential) = credential {
            request = request.set("Authorization", &authorization_header(credential));
        }
        for (name, value) in headers {
            request = request.set(name, value);
        }
        Ok(request
            .call()
            .map_err(map_http_error)?
            .into_string()
            .map_err(|error| ConnectorError::Http(error.to_string()))?)
    } else {
        let path = PathBuf::from(source);
        let mut file = std::fs::File::open(&path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        Ok(content)
    }
}

fn authorization_header(credential: &ConnectorCredential) -> String {
    format!(
        "{} {}",
        if credential.token_type.eq_ignore_ascii_case("bearer") {
            "Bearer"
        } else {
            &credential.token_type
        },
        credential.access_token
    )
}

fn map_http_error(error: ureq::Error) -> ConnectorError {
    match error {
        ureq::Error::Status(429, response) => ConnectorError::RateLimited {
            retry_after_seconds: response
                .header("Retry-After")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30),
        },
        other => ConnectorError::Http(other.to_string()),
    }
}

fn direct_project_hints(labels: &[String]) -> Vec<CorrelationHint> {
    let mut hints = labels
        .iter()
        .filter_map(|label| project_hint(label))
        .collect::<Vec<_>>();
    dedupe_project_hints(&mut hints);
    hints
}

fn project_hints_from_labels(labels: &[String]) -> Vec<CorrelationHint> {
    let mut hints = labels
        .iter()
        .filter_map(|label| {
            let lower = label.to_ascii_lowercase();
            ["project:", "project/", "project=", "项目:"]
                .iter()
                .find_map(|prefix| {
                    lower
                        .starts_with(prefix)
                        .then(|| project_hint(label[prefix.len()..].trim()))
                        .flatten()
                })
        })
        .collect::<Vec<_>>();
    dedupe_project_hints(&mut hints);
    hints
}

fn project_hint(label: &str) -> Option<CorrelationHint> {
    identity_hint(CorrelationKind::Project, label)
}

fn identity_hint(kind: CorrelationKind, label: &str) -> Option<CorrelationHint> {
    let label = label.trim();
    if label.is_empty() {
        return None;
    }
    let mut key = String::new();
    let mut separator = false;
    for character in label.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            key.push(character);
            separator = false;
        } else if !separator && !key.is_empty() {
            key.push('-');
            separator = true;
        }
    }
    while key.ends_with('-') {
        key.pop();
    }
    let prefix = match kind {
        CorrelationKind::Project => "project",
        CorrelationKind::Contact => "contact",
        CorrelationKind::Customer => "customer",
        CorrelationKind::Thread => "thread",
        CorrelationKind::Milestone => "milestone",
    };
    (!key.is_empty()).then(|| CorrelationHint {
        kind,
        key: format!("{prefix}:{key}"),
        label: label.into(),
    })
}

fn dedupe_project_hints(hints: &mut Vec<CorrelationHint>) {
    let mut seen = std::collections::BTreeSet::new();
    hints.retain(|hint| seen.insert(hint.key.clone()));
}

fn parse_datetime(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Some(value.with_timezone(&Utc));
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ") {
        return Some(Utc.from_utc_datetime(&value));
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
        return Some(Utc.from_utc_datetime(&value));
    }
    if let Ok(value) = NaiveDate::parse_from_str(value, "%Y%m%d") {
        return Some(Utc.from_utc_datetime(&value.and_hms_opt(0, 0, 0)?));
    }
    None
}

fn parse_date(value: &str) -> Option<DateTime<Utc>> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|date| Utc.from_utc_datetime(&date))
}

fn form_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notion_search_body_requests_newest_matching_objects() {
        let body = notion_request_body("https://api.notion.com/v1/search", Some("cursor-2"));
        assert_eq!(body["page_size"], 100);
        assert_eq!(body["start_cursor"], "cursor-2");
        assert_eq!(body["query"], "");
        assert_eq!(body["sort"]["timestamp"], "last_edited_time");
        assert_eq!(body["sort"]["direction"], "descending");
    }

    #[test]
    fn notion_database_body_uses_same_pagination_without_search_fields() {
        let body = notion_request_body("https://api.notion.com/v1/databases/db-id/query", None);
        assert_eq!(body["page_size"], 100);
        assert!(body.get("start_cursor").is_none());
        assert!(body.get("query").is_none());
        assert!(body.get("sort").is_none());
    }
}
