//! 首批只读 Personal Agent 连接器。

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use else_protocol::connector::ExternalFact;
use else_protocol::ids::ConnectorId;
use ical::IcalParser;

#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("calendar: {0}")]
    Calendar(String),
}

pub type Result<T> = std::result::Result<T, ConnectorError>;

pub fn read_calendar(
    connector_id: ConnectorId,
    source: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<ExternalFact>> {
    let content = if source.starts_with("https://") {
        ureq::get(source)
            .call()
            .map_err(|error| ConnectorError::Http(error.to_string()))?
            .into_string()
            .map_err(|error| ConnectorError::Http(error.to_string()))?
    } else {
        let path = PathBuf::from(source);
        let mut file = std::fs::File::open(&path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        content
    };
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
            });
        }
    }
    facts.sort_by_key(|fact| (fact.occurred_at, fact.fact_id.clone()));
    Ok(facts)
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

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
