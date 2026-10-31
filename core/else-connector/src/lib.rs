//! 只读 Personal Agent 连接器：Provider payload -> 标准化事实。

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use else_protocol::connector::{ConnectorPullPage, ExternalFact, SignalStatus, SignalType};
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
}

pub type Result<T> = std::result::Result<T, ConnectorError>;

pub fn read_calendar(
    connector_id: ConnectorId,
    source: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<ExternalFact>> {
    Ok(pull_calendar(connector_id, source, window_start, window_end)?.facts)
}

pub fn pull_calendar(
    connector_id: ConnectorId,
    source: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<ConnectorPullPage> {
    let content = load_source(source)?;
    let cursor = format!("calendar:{:016x}", stable_hash(&content));
    let facts = normalize_calendar(&connector_id, &content, window_start, window_end)?;
    Ok(ConnectorPullPage {
        stream_id: "calendar-events".into(),
        opaque_cursor: cursor,
        overlap_until: Some(window_start),
        facts,
    })
}

fn normalize_calendar(
    connector_id: &ConnectorId,
    content: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<ExternalFact>> {
    let reader = IcalParser::new(content.as_bytes());
    let mut facts = Vec::new();
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
            let uid = property("UID").unwrap_or_else(|| format!("{summary}:{start}"));
            let source_version = format!(
                "{:016x}",
                stable_hash(&format!("{summary}\0{start}\0{end:?}"))
            );
            facts.push(ExternalFact {
                connector_id: connector_id.clone(),
                entity_id: uid.clone(),
                source_version: source_version.clone(),
                fact_id: format!("calendar:{connector_id}:{uid}:{source_version}"),
                source: "calendar".into(),
                summary: format!("日程：{summary}（{}）", start.to_rfc3339()),
                occurred_at: start,
                valid_until: end,
                entity_kind: "calendar_event".into(),
                canonical_url: None,
                signal_type: SignalType::CalendarEvent,
                actor: None,
                structured_facts: json!({ "title": summary }),
                evidence_ref: None,
                sensitivity: "internal".into(),
                source_trust: "deterministic_read_only".into(),
                status: SignalStatus::Active,
            });
        }
    }
    facts.sort_by_key(|fact| (fact.occurred_at, fact.fact_id.clone()));
    Ok(facts)
}

pub fn pull_linear(connector_id: ConnectorId, source: &str) -> Result<ConnectorPullPage> {
    let content = load_source(source)?;
    let payload: Value = serde_json::from_str(&content)
        .map_err(|error| ConnectorError::Linear(error.to_string()))?;
    Ok(ConnectorPullPage {
        stream_id: "linear-issues".into(),
        opaque_cursor: format!("linear:{:016x}", stable_hash(&content)),
        overlap_until: None,
        facts: normalize_linear_payload(&connector_id, &payload)?,
    })
}

pub fn normalize_linear_payload(
    connector_id: &ConnectorId,
    payload: &Value,
) -> Result<Vec<ExternalFact>> {
    let issues = linear_issue_nodes(payload);
    let mut facts = issues
        .into_iter()
        .filter_map(|issue| normalize_linear_issue(connector_id, issue).transpose())
        .collect::<Result<Vec<_>>>()?;
    facts.sort_by_key(|fact| (fact.occurred_at, fact.fact_id.clone()));
    Ok(facts)
}

fn normalize_linear_issue(
    connector_id: &ConnectorId,
    issue: &Value,
) -> Result<Option<ExternalFact>> {
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
    let waiting = labels
        .iter()
        .any(|label| matches!(label.as_str(), "waiting" | "waiting_external" | "blocked"))
        || !blocked_by.is_empty();
    let commitment = labels.iter().any(|label| label == "commitment");
    let signal_type = if waiting {
        SignalType::WaitingExternal
    } else if commitment {
        SignalType::Commitment
    } else if due_at.is_some() {
        SignalType::ActionItem
    } else {
        SignalType::Issue
    };
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
    Ok(Some(ExternalFact {
        connector_id: connector_id.clone(),
        entity_id: id.clone(),
        source_version: source_version.clone(),
        fact_id: format!("linear:{connector_id}:{id}:{source_version}"),
        source: "linear".into(),
        summary,
        occurred_at: updated_at,
        valid_until: None,
        entity_kind: "linear_issue".into(),
        canonical_url: url.clone(),
        signal_type,
        actor: assignee.clone(),
        structured_facts: json!({
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
        status: if completed {
            SignalStatus::Deleted
        } else {
            SignalStatus::Active
        },
    }))
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
