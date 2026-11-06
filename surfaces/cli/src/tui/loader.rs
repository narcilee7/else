//! 后台加载、用户动作与 Runtime 事件流。
//!
//! 加载和动作使用独立 worker：一次长时间 `advance` 不会阻止界面刷新。

use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use else_client::{ClientError, RuntimeClient};
use else_protocol::attention::DecisionChoice;
use else_protocol::ids::{AttentionId, ConnectorId, ThreadId, WorldItemId};
use else_protocol::world::WorldItemResolutionKind;

use super::app_event::AppEvent;
use super::data::{ConnectorRow, Data, DecisionsData, OverviewData, ThreadDetailData};

/// 一次加载请求。
#[derive(Debug, Clone)]
pub enum LoadRequest {
    Overview {
        generation: u64,
    },
    Threads {
        generation: u64,
    },
    ThreadDetail {
        generation: u64,
        thread_id: ThreadId,
    },
    Decisions {
        generation: u64,
    },
    Connectors {
        generation: u64,
        thread_id: ThreadId,
    },
}

impl LoadRequest {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Overview { generation }
            | Self::Threads { generation }
            | Self::ThreadDetail { generation, .. }
            | Self::Decisions { generation }
            | Self::Connectors { generation, .. } => *generation,
        }
    }
}

/// 一次用户动作。
#[derive(Debug, Clone)]
pub enum ActionRequest {
    CreateThread {
        outcome: String,
    },
    AdvanceThread {
        thread_id: ThreadId,
    },
    Decide {
        thread_id: ThreadId,
        request_id: AttentionId,
        choice: DecisionChoice,
    },
    ResolveWorld {
        thread_id: ThreadId,
        item_id: WorldItemId,
        kind: WorldItemResolutionKind,
    },
    SyncConnector {
        thread_id: ThreadId,
        connector_id: ConnectorId,
    },
}

impl ActionRequest {
    pub fn pending_label(&self) -> &'static str {
        match self {
            Self::CreateThread { .. } => "Creating thread…",
            Self::AdvanceThread { .. } => "Advancing thread…",
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
        LoadRequest::Threads { .. } => Ok(Data::Threads(client.list_threads()?)),
        LoadRequest::ThreadDetail { thread_id, .. } => {
            let thread = client.thread(thread_id)?;
            let (scopes, plan, resources, events) = thread::scope(|scope| {
                let scopes = scope.spawn(|| client.scopes(thread_id));
                let plan = scope.spawn(|| client.plan(thread_id));
                let resources = scope.spawn(|| client.resources(thread_id));
                let events = scope.spawn(|| client.events(thread_id));
                (join(scopes), join(plan), join(resources), join(events))
            });
            Ok(Data::ThreadDetail(Box::new(ThreadDetailData {
                thread,
                scopes: scopes?,
                plan: plan?,
                resources: resources?,
                events: events?,
            })))
        }
        LoadRequest::Decisions { .. } => {
            let (windows, threads) = thread::scope(|scope| {
                let windows = scope.spawn(|| client.attention_windows());
                let threads = scope.spawn(|| client.list_threads());
                (join(windows), join(threads))
            });
            let windows = windows?;
            let grouped: Vec<_> = windows
                .iter()
                .flat_map(|window| window.requests.iter().map(|request| request.id.clone()))
                .collect();
            let mut loose = Vec::new();
            for thread in threads? {
                for request in &thread.decisions {
                    if !grouped.contains(&request.id) {
                        loose.push((thread.spec.id.clone(), request.clone()));
                    }
                }
            }
            Ok(Data::Decisions(Box::new(DecisionsData { windows, loose })))
        }
        LoadRequest::Connectors { thread_id, .. } => {
            let connectors = client.connectors(thread_id)?;
            let rows = thread::scope(|scope| {
                let jobs: Vec<_> = connectors
                    .into_iter()
                    .map(|connector| {
                        scope.spawn(move || {
                            let health = client.connector_health(thread_id, &connector.id)?;
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
    let (brief, attention, threads, projects, identities) = thread::scope(|scope| {
        let brief = scope.spawn(|| client.brief());
        let attention = scope.spawn(|| client.attention_windows());
        let threads = scope.spawn(|| client.list_threads());
        let projects = scope.spawn(|| client.project_worlds(None, false));
        let identities = scope.spawn(|| client.identity_worlds(None, false));
        (
            join(brief),
            join(attention),
            join(threads),
            join(projects),
            join(identities),
        )
    });
    let mut identities = identities?;
    identities.truncate(5);
    Ok(Data::Overview(Box::new(OverviewData {
        brief: brief?,
        attention: attention?,
        threads: threads?,
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
    let result: else_client::Result<(String, String, Option<ThreadId>)> = match action {
        ActionRequest::CreateThread { outcome } => client
            .create_thread(outcome, None, &Default::default())
            .map(|snapshot| {
                (
                    "Thread created".to_string(),
                    snapshot.spec.outcome.clone(),
                    Some(snapshot.spec.id),
                )
            }),
        ActionRequest::AdvanceThread { thread_id } => {
            client.advance(thread_id, Some("auto")).map(|snapshot| {
                (
                    "Thread updated".to_string(),
                    super::views::thread_state_label(snapshot.state).to_string(),
                    Some(thread_id.clone()),
                )
            })
        }
        ActionRequest::Decide {
            thread_id,
            request_id,
            choice,
        } => client
            .decide(thread_id, request_id, choice.clone())
            .map(|_| {
                (
                    "Decision saved".to_string(),
                    "The runtime is continuing from your choice".to_string(),
                    Some(thread_id.clone()),
                )
            }),
        ActionRequest::ResolveWorld {
            thread_id,
            item_id,
            kind,
        } => client
            .resolve_world_item(thread_id, item_id, *kind, None)
            .map(|item| {
                (
                    "Item updated".to_string(),
                    item.title.clone(),
                    Some(thread_id.clone()),
                )
            }),
        ActionRequest::SyncConnector {
            thread_id,
            connector_id,
        } => client
            .sync_connector(thread_id, connector_id)
            .map(|result| {
                (
                    "Sync complete".to_string(),
                    format!(
                        "{} observed · {} new signals",
                        result.observed, result.inserted
                    ),
                    Some(thread_id.clone()),
                )
            }),
    };
    match result {
        Ok((label, detail, thread_id)) => AppEvent::ActionDone {
            label,
            detail,
            thread_id,
        },
        Err(error) => AppEvent::ActionFailed {
            error: error.to_string(),
        },
    }
}
