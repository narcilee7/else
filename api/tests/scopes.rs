//! M1-W2：委托权限收窄、子 Scope 独立终态、Abort 沿树传播与恢复。

use else_api::{ApiError, Runtime};
use else_protocol::attention::DecisionChoice;
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{EventPayload, ScopeState};
use else_protocol::ids::{IntentId, ScopeId};
use else_protocol::intent::IntentState;

fn root_scope(runtime: &Runtime, intent_id: &IntentId) -> ScopeId {
    runtime
        .projection(intent_id)
        .unwrap()
        .scopes
        .iter()
        .find(|(_, scope)| scope.parent.is_none())
        .map(|(id, _)| id.clone())
        .unwrap()
}

fn setup_tree(runtime: &Runtime) -> (else_protocol::intent::IntentSpec, ScopeId, ScopeId, ScopeId) {
    let spec = runtime.create_intent("验证委托树", None).unwrap();
    runtime.run(&spec.id).unwrap();
    let root = root_scope(runtime, &spec.id);
    let child = runtime
        .spawn_subagent(&spec.id, &root, "子任务", SideEffectKind::ReadOnly)
        .unwrap();
    let grandchild = runtime
        .spawn_subagent(&spec.id, &child, "孙任务", SideEffectKind::ReadOnly)
        .unwrap();
    runtime.run_scope(&spec.id, &child).unwrap();
    (spec, root, child, grandchild)
}

fn assert_tree_cancelled(
    runtime: &Runtime,
    intent_id: &IntentId,
    root: &ScopeId,
    child: &ScopeId,
    grandchild: &ScopeId,
) {
    let projection = runtime.projection(intent_id).unwrap();
    assert_eq!(projection.state, Some(IntentState::Cancelled));
    assert!(projection.open_requests.is_empty());
    for scope_id in [root, child, grandchild] {
        assert_eq!(
            projection.scopes[scope_id].state,
            ScopeState::Cancelled,
            "scope {scope_id} 未被取消"
        );
    }
    let events = runtime.events(intent_id).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.envelope.payload,
                EventPayload::ScopeAbortRequested { .. }
            ))
            .count(),
        3
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.envelope.payload,
                EventPayload::SideEffectConcluded { .. }
            ))
            .count(),
        2,
        "根 shell 与子 fs.write 都必须有结论"
    );
}

#[test]
fn delegation_is_replayable_and_cannot_widen_authority() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("权限收窄", None).unwrap();
    runtime.run(&spec.id).unwrap();
    let root = root_scope(&runtime, &spec.id);

    let error = runtime
        .spawn_subagent(&spec.id, &root, "越权任务", SideEffectKind::Unknown)
        .unwrap_err();
    assert!(matches!(error, ApiError::PermissionEscalation(_)));

    let child = runtime
        .spawn_subagent(&spec.id, &root, "只读研究", SideEffectKind::ReadOnly)
        .unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    let delegated = &projection.scopes[&child];
    assert_eq!(delegated.parent, Some(root));
    assert_eq!(
        delegated
            .spec
            .as_ref()
            .unwrap()
            .authority
            .max_autonomous_effect,
        SideEffectKind::ReadOnly
    );
    drop(runtime);

    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(reopened.projection(&spec.id).unwrap(), projection);
}

#[test]
fn completing_subagent_does_not_complete_root_intent() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("父子终态隔离", None).unwrap();
    runtime.run(&spec.id).unwrap();
    let root = root_scope(&runtime, &spec.id);
    let child = runtime
        .spawn_subagent(&spec.id, &root, "独立子任务", SideEffectKind::Reversible)
        .unwrap();
    runtime.run_scope(&spec.id, &child).unwrap();

    let request_id = runtime
        .events(&spec.id)
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.envelope.payload {
            EventPayload::AttentionRequested { request } if event.envelope.scope_id == child => {
                Some(request.id.clone())
            }
            _ => None,
        })
        .unwrap();
    let projection = runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("approve".into()),
        )
        .unwrap();
    assert_eq!(projection.scopes[&child].state, ScopeState::Completed);
    assert_eq!(projection.scopes[&root].state, ScopeState::Running);
    assert_eq!(projection.state, Some(IntentState::Active));
    assert_eq!(
        projection.artifacts.len(),
        2,
        "父子必须各自规划，不能共享进度"
    );
}

#[test]
fn abort_propagates_to_children_and_grandchildren() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let (spec, root, child, grandchild) = setup_tree(&runtime);
    runtime.abort(&spec.id, "取消整棵树").unwrap();
    assert_tree_cancelled(&runtime, &spec.id, &root, &child, &grandchild);

    let count = runtime.events(&spec.id).unwrap().len();
    runtime.run_scope(&spec.id, &child).unwrap();
    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_tree_cancelled(&reopened, &spec.id, &root, &child, &grandchild);
    assert_eq!(reopened.events(&spec.id).unwrap().len(), count);
}

#[test]
fn kill_at_every_tree_abort_boundary_recovers_all_descendants() {
    // 先测出该树完整取消的事件数，再逐点 kill，避免测试硬编码实现细节。
    let baseline_dir = tempfile::tempdir().unwrap();
    let baseline = Runtime::open(baseline_dir.path()).unwrap();
    let (spec, ..) = setup_tree(&baseline);
    let before = baseline.events(&spec.id).unwrap().len();
    baseline.abort(&spec.id, "测量边界").unwrap();
    let abort_events = baseline.events(&spec.id).unwrap().len() - before;
    drop(baseline);

    for cut in 1..=abort_events as u64 {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let (spec, root, child, grandchild) = setup_tree(&runtime);
        runtime.crash_after_appends(cut);
        assert!(runtime.abort(&spec.id, "树取消 kill").is_err());
        drop(runtime);

        let recovered = Runtime::open(dir.path()).unwrap();
        assert_tree_cancelled(&recovered, &spec.id, &root, &child, &grandchild);
        let count = recovered.events(&spec.id).unwrap().len();
        drop(recovered);
        let reopened = Runtime::open(dir.path()).unwrap();
        assert_eq!(reopened.events(&spec.id).unwrap().len(), count, "cut={cut}");
    }
}
