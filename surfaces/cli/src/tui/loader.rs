//! 后台加载、用户动作与 Runtime 事件流。
//!
//! 加载和动作使用独立 worker：一次长时间 `advance` 不会阻止界面刷新。

use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use else_client::{ClientError, RuntimeClient};
use else_protocol::attention::DecisionChoice;
use else_protocol::ids::{AttentionId, ConnectorId, IntentId, WorldItemId};
use else_protocol::world::WorldItemResolutionKind;

use super::app_event::AppEvent;
use super::data::{ConnectorRow, Data, DecisionsData, IntentDetailData, OverviewData};

/// 一次加载请求。
#[derive(Debug, Clone)]
pub enum LoadRequest {
    Overview {
        generation: u64,
    },
    Intents {
        generation: u64,
    },
    IntentDetail {
        generation: u64,
        intent_id: IntentId,
    },
    Decisions {
        generation: u64,
    },
    Connectors {
        generation: u64,
        intent_id: IntentId,
    },
}

impl LoadRequest {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Overview { generation }
            | Self::Intents { generation }
            | Self::IntentDetail { generation, .. }
            | Self::Decisions { generation }
            | Self::Connectors { generation, .. } => *generation,
        }
    }
}

/// 一次用户动作。
#[derive(Debug, Clone)]
pub enum ActionRequest {
    CreateIntent {
        outcome: String,
    },
    AdvanceIntent {
        intent_id: IntentId,
    },
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

impl ActionRequest {
    pub fn pending_label(&self) -> &'static str {
        match self {
            Self::CreateIntent { .. } => "Creating intent…",
            Self::AdvanceIntent { .. } => "Advancing intent…",
            Self::Decide { .. } => "Saving decision…",
            Self::ResolveWorld { .. } => "Updating item…",
            Self::SyncConnector { .. } => "Syncing connector…",
        }
    }
}

/// 启动只负责读取的 worker。
pub fn spawn_load_worker(
    client: RuntimeClient,
    events: Sender<AppEvent>,
) -> (Sender<LoadRequest>, JoinHandle<()>) {
    let (requests, receiver) = std::sync::mpsc::channel::<LoadRequest>();
    let handle = thread::spawn(move || {
        while let Ok(request) = receiver.recv() {
            let generation = request.generation();
            let event = match fetch(&client, &request) {
                Ok(data) => AppEvent::Loaded { generation, data },
                Err(error) => AppEvent::LoadFailed {
                    generation,
                    error: error.to_string(),
                },
            };
            if events.send(event).is_err() {
                break;
            }
        }
    });
    (requests, handle)
}

/// 启动只负责写操作的 worker。
pub fn spawn_action_worker(
    client: RuntimeClient,
    events: Sender<AppEvent>,
) -> (Sender<ActionRequest>, JoinHandle<()>) {
    let (requests, receiver) = std::sync::mpsc::channel::<ActionRequest>();
    let handle = thread::spawn(move || {
        while let Ok(action) = receiver.recv() {
            let outcome = perform(&client, &action);
            if events.send(outcome).is_err() {
                break;
            }
        }
    });
    (requests, handle)
}

/// 起事件流线程。
pub fn spawn_event_stream(client: RuntimeClient, events: Sender<AppEvent>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut cursor = 0u64;
        loop {
            let mut stream = match client.event_stream(cursor) {
                Ok(stream) => stream,
                Err(_) => {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            while let Ok(Some(event)) = stream.next_event() {
                cursor = event.seq;
                if events.send(AppEvent::RuntimeEvent(event)).is_err() {
                    return;
                }
            }
            // 正常 EOF（服务端定期关闭 SSE）和异常断开都不能形成紧密重连循环。
            thread::sleep(Duration::from_millis(500));
        }
    })
}

fn fetch(client: &RuntimeClient, request: &LoadRequest) -> else_client::Result<Data> {
    match request {
        LoadRequest::Overview { .. } => fetch_overview(client),
        LoadRequest::Intents { .. } => Ok(Data::Intents(client.list_intents()?)),
        LoadRequest::IntentDetail { intent_id, .. } => {
            let intent = client.intent(intent_id)?;
            let (scopes, plan, resources, events) = thread::scope(|scope| {
                let scopes = scope.spawn(|| client.scopes(intent_id));
                let plan = scope.spawn(|| client.plan(intent_id));
                let resources = scope.spawn(|| client.resources(intent_id));
                let events = scope.spawn(|| client.events(intent_id));
                (join(scopes), join(plan), join(resources), join(events))
            });
            Ok(Data::IntentDetail(Box::new(IntentDetailData {
                intent,
                scopes: scopes?,
                plan: plan?,
                resources: resources?,
                events: events?,
            })))
        }
        LoadRequest::Decisions { .. } => {
            let (windows, intents) = thread::scope(|scope| {
                let windows = scope.spawn(|| client.attention_windows());
                let intents = scope.spawn(|| client.list_intents());
                (join(windows), join(intents))
            });
            let windows = windows?;
            let grouped: Vec<_> = windows
                .iter()
                .flat_map(|window| window.requests.iter().map(|request| request.id.clone()))
                .collect();
            let mut loose = Vec::new();
            for intent in intents? {
                for request in &intent.decisions {
                    if !grouped.contains(&request.id) {
                        loose.push((intent.spec.id.clone(), request.clone()));
                    }
                }
            }
            Ok(Data::Decisions(Box::new(DecisionsData { windows, loose })))
        }
        LoadRequest::Connectors { intent_id, .. } => {
            let connectors = client.connectors(intent_id)?;
            let rows = thread::scope(|scope| {
                let jobs: Vec<_> = connectors
                    .into_iter()
                    .map(|connector| {
                        scope.spawn(move || {
                            let health = client.connector_health(intent_id, &connector.id)?;
                            Ok::<ConnectorRow, ClientError>(ConnectorRow { connector, health })
                        })
                    })
                    .collect();
                jobs.into_iter().map(join).collect::<Vec<_>>()
            });
            let mut collected = Vec::with_capacity(rows.len());
            for row in rows {
                collected.push(row?);
            }
            Ok(Data::Connectors(collected))
        }
    }
}

fn fetch_overview(client: &RuntimeClient) -> else_client::Result<Data> {
    let (brief, attention, intents, projects, identities) = thread::scope(|scope| {
        let brief = scope.spawn(|| client.brief());
        let attention = scope.spawn(|| client.attention_windows());
        let intents = scope.spawn(|| client.list_intents());
        let projects = scope.spawn(|| client.project_worlds(None, false));
        let identities = scope.spawn(|| client.identity_worlds(None, false));
        (
            join(brief),
            join(attention),
            join(intents),
            join(projects),
            join(identities),
        )
    });
    let mut identities = identities?;
    identities.truncate(5);
    Ok(Data::Overview(Box::new(OverviewData {
        brief: brief?,
        attention: attention?,
        intents: intents?,
        projects: projects?,
        identities,
    })))
}

fn join<T>(handle: thread::ScopedJoinHandle<'_, else_client::Result<T>>) -> else_client::Result<T> {
    handle
        .join()
        .map_err(|_| ClientError::Malformed("background request panicked".into()))?
}

fn perform(client: &RuntimeClient, action: &ActionRequest) -> AppEvent {
    let result: else_client::Result<(String, String, Option<IntentId>)> = match action {
        ActionRequest::CreateIntent { outcome } => {
            client.create_intent(outcome, None).map(|snapshot| {
                (
                    "Intent created".to_string(),
                    snapshot.spec.outcome.clone(),
                    Some(snapshot.spec.id),
                )
            })
        }
        ActionRequest::AdvanceIntent { intent_id } => {
            client.advance(intent_id, Some("auto")).map(|snapshot| {
                (
                    "Intent updated".to_string(),
                    super::views::intent_state_label(snapshot.state).to_string(),
                    Some(intent_id.clone()),
                )
            })
        }
        ActionRequest::Decide {
            intent_id,
            request_id,
            choice,
        } => client
            .decide(intent_id, request_id, choice.clone())
            .map(|_| {
                (
                    "Decision saved".to_string(),
                    "The runtime is continuing from your choice".to_string(),
                    Some(intent_id.clone()),
                )
            }),
        ActionRequest::ResolveWorld {
            intent_id,
            item_id,
            kind,
        } => client
            .resolve_world_item(intent_id, item_id, *kind, None)
            .map(|item| {
                (
                    "Item updated".to_string(),
                    item.title.clone(),
                    Some(intent_id.clone()),
                )
            }),
        ActionRequest::SyncConnector {
            intent_id,
            connector_id,
        } => client
            .sync_connector(intent_id, connector_id)
            .map(|result| {
                (
                    "Sync complete".to_string(),
                    format!(
                        "{} observed · {} new signals",
                        result.observed, result.inserted
                    ),
                    Some(intent_id.clone()),
                )
            }),
    };
    match result {
        Ok((label, detail, intent_id)) => AppEvent::ActionDone {
            label,
            detail,
            intent_id,
        },
        Err(error) => AppEvent::ActionFailed {
            error: error.to_string(),
        },
    }
}
