//! 后台工作线程。
//!
//! 界面线程只做两件事：收事件、画帧。所有 HTTP 往返都发生在后台，
//! 结果通过消息总线送回来。这样即使 Runtime 正忙，界面也不会卡住 ——
//! 用户仍然能滚动、能看上一帧、能退出。
//!
//! 后台线程同时守着事件流：Runtime 有新事件时推给界面，界面决定要不要刷新。

use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use else_client::RuntimeClient;
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
    World {
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
    /// 请求的代次：用来丢弃过期响应。
    pub fn generation(&self) -> u64 {
        match self {
            Self::Overview { generation }
            | Self::Intents { generation }
            | Self::IntentDetail { generation, .. }
            | Self::World { generation, .. }
            | Self::Decisions { generation }
            | Self::Connectors { generation, .. } => *generation,
        }
    }
}

/// 一次用户动作。
#[derive(Debug, Clone)]
pub enum ActionRequest {
    /// 创建一个 Intent。
    CreateIntent { outcome: String },
    /// 推进一个 Intent。
    AdvanceIntent { intent_id: IntentId },
    /// 对一条注意力请求做出裁决。
    Decide {
        intent_id: IntentId,
        request_id: AttentionId,
        choice: DecisionChoice,
    },
    /// 解决一个 World 条目。
    ResolveWorld {
        intent_id: IntentId,
        item_id: WorldItemId,
        kind: WorldItemResolutionKind,
    },
    /// 立即同步一个 Connector。
    SyncConnector {
        intent_id: IntentId,
        connector_id: ConnectorId,
    },
}

/// 后台要干的一件事。
#[derive(Debug, Clone)]
pub enum Work {
    Load(LoadRequest),
    Action(ActionRequest),
}

/// 起后台工作线程。
pub fn spawn_worker(
    client: RuntimeClient,
    events: Sender<AppEvent>,
) -> (Sender<Work>, JoinHandle<()>) {
    let (requests, receiver): (Sender<Work>, Receiver<Work>) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        while let Ok(work) = receiver.recv() {
            match work {
                Work::Load(request) => {
                    let generation = request.generation();
                    match fetch(&client, &request) {
                        Ok(data) => {
                            if events.send(AppEvent::Loaded { generation, data }).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            if events
                                .send(AppEvent::LoadFailed {
                                    generation,
                                    error: error.to_string(),
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                Work::Action(action) => {
                    if events.send(AppEvent::ActionStarted).is_err() {
                        break;
                    }
                    let outcome = perform(&client, &action);
                    if events.send(outcome).is_err() {
                        break;
                    }
                }
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
                    // Runtime 还没起来或断开了：等一会儿再连。
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            loop {
                match stream.next_event() {
                    Ok(Some(event)) => {
                        cursor = event.seq;
                        if events.send(AppEvent::RuntimeEvent(event)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    })
}

/// 按请求取数据。
fn fetch(client: &RuntimeClient, request: &LoadRequest) -> else_client::Result<Data> {
    match request {
        LoadRequest::Overview { .. } => {
            let mut data = OverviewData {
                brief: client.brief()?,
                attention: client.attention_windows()?,
                intents: client.list_intents()?,
                projects: client.project_worlds(None, false)?,
                identities: client.identity_worlds(None, false)?,
            };
            // 身份世界通常很少；截断一点，别让首屏被它挤满。
            data.identities.truncate(5);
            Ok(Data::Overview(Box::new(data)))
        }
        LoadRequest::Intents { .. } => Ok(Data::Intents(client.list_intents()?)),
        LoadRequest::IntentDetail { intent_id, .. } => {
            let intent = client.intent(intent_id)?;
            Ok(Data::IntentDetail(Box::new(IntentDetailData {
                scopes: client.scopes(intent_id)?,
                plan: client.plan(intent_id)?,
                resources: client.resources(intent_id)?,
                events: client.events(intent_id)?,
                intent,
            })))
        }
        LoadRequest::World { intent_id, .. } => {
            Ok(Data::World(client.world_items(intent_id, false)?))
        }
        LoadRequest::Decisions { .. } => {
            let windows = client.attention_windows()?;
            // 窗口外的请求同样在等人 —— 它们必须出现在队列里，
            // 否则用户会以为「没有要决定的事」。
            let grouped: Vec<_> = windows
                .iter()
                .flat_map(|window| window.requests.iter().map(|request| request.id.clone()))
                .collect();
            let mut loose = Vec::new();
            for intent in client.list_intents()? {
                for request in &intent.decisions {
                    if !grouped.contains(&request.id) {
                        loose.push((intent.spec.id.clone(), request.clone()));
                    }
                }
            }
            Ok(Data::Decisions(Box::new(DecisionsData { windows, loose })))
        }
        LoadRequest::Connectors { intent_id, .. } => {
            let mut rows = Vec::new();
            for connector in client.connectors(intent_id)? {
                let health = client.connector_health(intent_id, &connector.id)?;
                rows.push(ConnectorRow { connector, health });
            }
            Ok(Data::Connectors(rows))
        }
    }
}

/// 执行一次用户动作。
fn perform(client: &RuntimeClient, action: &ActionRequest) -> AppEvent {
    let result: else_client::Result<(String, String, Option<IntentId>)> = match action {
        ActionRequest::CreateIntent { outcome } => {
            client.create_intent(outcome, None).map(|snapshot| {
                (
                    "intent created".to_string(),
                    snapshot.spec.outcome.clone(),
                    Some(snapshot.spec.id),
                )
            })
        }
        ActionRequest::AdvanceIntent { intent_id } => {
            client.advance(intent_id, Some("auto")).map(|snapshot| {
                (
                    "advanced".to_string(),
                    format!("{:?}", snapshot.state),
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
                    "decided".to_string(),
                    format!("{request_id}"),
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
                    "world updated".to_string(),
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
                    "synced".to_string(),
                    format!("observed={} inserted={}", result.observed, result.inserted),
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
