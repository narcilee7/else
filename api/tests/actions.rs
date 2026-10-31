use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use else_api::{ApiError, Runtime};
use else_connector::{AdapterRegistry, ConnectorAdapter, SignalInterpreter};
use else_protocol::attention::DecisionChoice;
use else_protocol::connector::{
    ConnectorActionStatus, ConnectorAuthMode, ConnectorCredential, ConnectorHealth,
    ConnectorManifest, ConnectorPullPage, ConnectorSyncMode,
};
use else_protocol::effect::{EffectConclusion, ToolOutcome, ToolStatus};
use else_protocol::ids::ConnectorId;
use serde_json::{json, Value};

struct WriteAdapter {
    calls: Arc<AtomicU64>,
}

impl ConnectorAdapter for WriteAdapter {
    fn id(&self) -> &'static str {
        "fixture.write"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["fixture-write"]
    }

    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            adapter: self.id().into(),
            provider: "fixture".into(),
            auth_mode: ConnectorAuthMode::None,
            scopes: vec![],
            read_capabilities: vec![],
            write_capabilities: vec!["fixture.item.update".into()],
            sync_modes: vec![ConnectorSyncMode::Pull],
            data_sensitivity: "internal".into(),
            retention_policy: "metadata_only".into(),
            health: ConnectorHealth::Healthy,
        }
    }

    fn pull_stream_id(&self) -> &'static str {
        "fixture-write-items"
    }

    fn pull(
        &self,
        _connector_id: ConnectorId,
        _source: &str,
        _cursor: Option<&else_protocol::connector::SyncCursor>,
        _now: DateTime<Utc>,
        _credential: Option<&ConnectorCredential>,
    ) -> else_connector::Result<ConnectorPullPage> {
        Ok(ConnectorPullPage {
            stream_id: self.pull_stream_id().into(),
            opaque_cursor: "fixture:write".into(),
            overlap_until: None,
            continuation: None,
            observations: vec![],
        })
    }

    fn execute_action(
        &self,
        _connector_id: &ConnectorId,
        _source: &str,
        _capability: &str,
        _credential: Option<&ConnectorCredential>,
        input: &Value,
    ) -> else_connector::Result<ToolOutcome> {
        let calls = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if input["result"] == "unknown" {
            return Ok(ToolOutcome {
                status: ToolStatus::Unknown,
                output: json!({"request_id": "provider-request-unknown"}),
                effect: EffectConclusion::Unknown,
            });
        }
        Ok(ToolOutcome {
            status: ToolStatus::Succeeded,
            output: json!({"target": input["target"], "calls": calls}),
            effect: EffectConclusion::Applied,
        })
    }
}

fn runtime_with_adapter(dir: &std::path::Path, calls: Arc<AtomicU64>) -> Runtime {
    let adapter = Box::new(WriteAdapter { calls });
    Runtime::open_with_connector_runtime(
        dir,
        AdapterRegistry::new(vec![adapter]),
        SignalInterpreter::builtin(),
    )
    .unwrap()
}

#[test]
fn external_action_requires_approval_executes_once_and_preserves_idempotency() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    let runtime = runtime_with_adapter(dir.path(), calls.clone());
    let source = dir.path().join("write.json");
    std::fs::write(&source, "{}").unwrap();
    let spec = runtime.create_intent("更新外部事项", None).unwrap();
    let connector = runtime
        .register_connector_adapter(
            &spec.id,
            "writer",
            source.to_str().unwrap(),
            "fixture.write",
        )
        .unwrap();
    let action = runtime
        .propose_connector_action(
            &spec.id,
            &connector.id,
            "fixture.item.update",
            "item-1",
            json!({"title": "更新后的标题"}),
            Some("stable-action-key".into()),
        )
        .unwrap();

    let pending = runtime
        .execute_connector_action(&spec.id, &action.id)
        .unwrap_err();
    assert!(matches!(pending, ApiError::Gateway(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let action = runtime
        .connector_actions(&spec.id)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.id == action.id)
        .unwrap();
    assert_eq!(action.status, ConnectorActionStatus::NeedsApproval);
    let request = action.attention_id.clone().unwrap();

    runtime
        .decide(&spec.id, &request, DecisionChoice::Option("approve".into()))
        .unwrap();
    let applied = runtime
        .execute_connector_action(&spec.id, &action.id)
        .unwrap();
    assert_eq!(applied.status, ConnectorActionStatus::Applied);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let replay = runtime
        .execute_connector_action(&spec.id, &action.id)
        .unwrap();
    assert_eq!(replay.status, ConnectorActionStatus::Applied);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn unknown_external_result_is_not_retried_blindly() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    let runtime = runtime_with_adapter(dir.path(), calls.clone());
    let source = dir.path().join("write.json");
    std::fs::write(&source, "{}").unwrap();
    let spec = runtime.create_intent("更新外部事项", None).unwrap();
    let connector = runtime
        .register_connector_adapter(
            &spec.id,
            "writer",
            source.to_str().unwrap(),
            "fixture.write",
        )
        .unwrap();
    let action = runtime
        .propose_connector_action(
            &spec.id,
            &connector.id,
            "fixture.item.update",
            "item-unknown",
            json!({"result": "unknown"}),
            Some("unknown-action-key".into()),
        )
        .unwrap();
    runtime
        .execute_connector_action(&spec.id, &action.id)
        .unwrap_err();
    let pending = runtime
        .connector_actions(&spec.id)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.id == action.id)
        .unwrap();
    runtime
        .decide(
            &spec.id,
            pending.attention_id.as_ref().unwrap(),
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    let unknown = runtime
        .execute_connector_action(&spec.id, &action.id)
        .unwrap();
    assert_eq!(unknown.status, ConnectorActionStatus::Unknown);
    assert!(matches!(
        runtime.execute_connector_action(&spec.id, &action.id),
        Err(ApiError::Conflict(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
