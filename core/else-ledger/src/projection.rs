//! 基础投影示例：从事件重建 Intent 的当前状态。
//! 投影可丢弃、可重建；UI 与 Decision Queue 只读投影。

use std::collections::BTreeMap;

use else_protocol::artifact::{Artifact, Checkpoint};
use else_protocol::attention::AttentionRequest;
use else_protocol::event::EventPayload;
use else_protocol::ids::AttentionId;
use else_protocol::intent::{IntentSpec, IntentState};

use crate::{Projection, StoredEvent};

/// Intent 的基础投影：目标契约、状态、待本人决策、产物与检查点。
#[derive(Debug, Default, PartialEq)]
pub struct IntentProjection {
    pub spec: Option<IntentSpec>,
    pub state: Option<IntentState>,
    /// Decision Queue：已请求但未决策的注意力请求。
    pub open_requests: BTreeMap<AttentionId, AttentionRequest>,
    pub artifacts: Vec<Artifact>,
    pub checkpoints: Vec<Checkpoint>,
    /// 已重放到的事件水位。
    pub cursor: u64,
}

impl Projection for IntentProjection {
    fn apply(&mut self, event: &StoredEvent) {
        self.cursor = event.seq;
        match &event.envelope.payload {
            EventPayload::IntentCreated { spec } | EventPayload::IntentRevised { spec, .. } => {
                self.spec = Some(spec.clone());
            }
            EventPayload::IntentStateChanged { state } => {
                self.state = Some(*state);
            }
            EventPayload::AttentionRequested { request } => {
                self.open_requests
                    .insert(request.id.clone(), request.clone());
            }
            EventPayload::DecisionResolved { decision } => {
                self.open_requests.remove(&decision.request_id);
            }
            EventPayload::ArtifactProduced { artifact } => {
                self.artifacts.push(artifact.clone());
            }
            EventPayload::CheckpointCreated { checkpoint } => {
                self.checkpoints.push(checkpoint.clone());
            }
            _ => {}
        }
    }
}
