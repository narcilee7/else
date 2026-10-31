//! Connector Adapter Registry 与 Provider 无关的 Signal Interpreter。
//!
//! Adapter 只负责认证、拉取、校验与 Normalize，输出中性 Observation；
//! 业务含义由 Interpreter 独立产生，核心 Runtime 不感知 Provider 细节。

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use else_protocol::connector::{
    ConnectorAuthMode, ConnectorHealth, ConnectorManifest, ConnectorObservation, ConnectorPullPage,
    ConnectorSyncMode, CorrelationHint, CorrelationKind, ObjectKind, ObservationChange,
    SignalInterpretation, SignalMeaning, SignalStatus, SignalType, SyncCursor, WorldSignal,
};
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
    ) -> Result<ConnectorPullPage>;
    fn webhook_stream_id(&self) -> Option<&'static str> {
        None
    }
    fn normalize_webhook(
        &self,
        _connector_id: &ConnectorId,
        _payload: &Value,
    ) -> Result<Vec<ConnectorObservation>> {
        Err(ConnectorError::WebhookUnsupported(self.id().into()))
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
    ) -> Result<ConnectorPullPage> {
        pull_calendar(
            connector_id,
            source,
            now - Duration::days(1),
            now + Duration::days(14),
        )
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
            auth_mode: ConnectorAuthMode::None,
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
        _cursor: Option<&SyncCursor>,
        _now: DateTime<Utc>,
    ) -> Result<ConnectorPullPage> {
        pull_linear(connector_id, source)
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
        _cursor: Option<&SyncCursor>,
        _now: DateTime<Utc>,
    ) -> Result<ConnectorPullPage> {
        pull_notion(connector_id, source)
    }
}

pub fn pull_calendar(
    connector_id: ConnectorId,
    source: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<ConnectorPullPage> {
    let content = load_source(source)?;
    let cursor = format!("calendar:{:016x}", stable_hash(&content));
    let observations = normalize_calendar(&connector_id, &content, window_start, window_end)?;
    Ok(ConnectorPullPage {
        stream_id: "calendar-events".into(),
        opaque_cursor: cursor,
        overlap_until: Some(window_start),
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
            let uid = property("UID").unwrap_or_else(|| format!("{summary}:{start}"));
            let source_version = format!(
                "{:016x}",
                stable_hash(&format!("{summary}\0{start}\0{end:?}"))
            );
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
                correlations: direct_project_hints(&categories),
            });
        }
    }
    observations
        .sort_by_key(|observation| (observation.occurred_at, observation.dedupe_key.clone()));
    Ok(observations)
}

pub fn pull_linear(connector_id: ConnectorId, source: &str) -> Result<ConnectorPullPage> {
    let content = load_source(source)?;
    let payload: Value = serde_json::from_str(&content)
        .map_err(|error| ConnectorError::Linear(error.to_string()))?;
    Ok(ConnectorPullPage {
        stream_id: "linear-issues".into(),
        opaque_cursor: format!("linear:{:016x}", stable_hash(&content)),
        overlap_until: None,
        observations: normalize_linear_payload(&connector_id, &payload)?,
    })
}

pub fn pull_notion(connector_id: ConnectorId, source: &str) -> Result<ConnectorPullPage> {
    let content = load_source(source)?;
    let payload: Value = serde_json::from_str(&content)
        .map_err(|error| ConnectorError::Notion(error.to_string()))?;
    Ok(ConnectorPullPage {
        stream_id: "notion-pages".into(),
        opaque_cursor: format!("notion:{:016x}", stable_hash(&content)),
        overlap_until: None,
        observations: normalize_notion_payload(&connector_id, &payload)?,
    })
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
            name.contains("project")
                || name.contains("client")
                || name.contains("customer")
                || name.contains("workstream")
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
    let correlations = direct_project_hints(&project_labels);
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

fn load_source(source: &str) -> Result<String> {
    if source.starts_with("https://") {
        Ok(ureq::get(source)
            .call()
            .map_err(|error| ConnectorError::Http(error.to_string()))?
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
    (!key.is_empty()).then(|| CorrelationHint {
        kind: CorrelationKind::Project,
        key: format!("project:{key}"),
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

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
