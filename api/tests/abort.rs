//! M1-W1：Abort 状态机、在途副作用结论、逐事件边界 kill 恢复。

use else_api::Runtime;
use else_ledger::LedgerError;
use else_protocol::attention::DecisionChoice;
use else_protocol::effect::EffectConclusion;
use else_protocol::event::{EventPayload, ScopeState};
use else_protocol::intent::IntentState;

fn waiting_intent(runtime: &Runtime) -> else_protocol::intent::IntentSpec {
    let spec = runtime.create_intent("验证取消语义", None).unwrap();
    let projection = runtime.run(&spec.id).unwrap();
    assert_eq!(projection.state, Some(IntentState::NeedsDecision));
    spec
}

fn assert_converged(runtime: &Runtime, intent_id: &else_protocol::ids::IntentId) {
    let projection = runtime.projection(intent_id).unwrap();
    assert_eq!(projection.state, Some(IntentState::Cancelled));
    assert!(projection.open_requests.is_empty());

    let events = runtime.events(intent_id).unwrap();
    let states: Vec<_> = events
        .iter()
        .filter_map(|event| match event.envelope.payload {
            EventPayload::ScopeStateChanged { state } => Some(state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        vec![
            ScopeState::Cancelling,
            ScopeState::Fenced,
            ScopeState::Compensating,
            ScopeState::Cancelled,
        ]
    );
    let conclusions: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.envelope.payload {
            EventPayload::SideEffectConcluded { conclusion, .. } => Some(*conclusion),
            _ => None,
        })
        .collect();
    assert_eq!(conclusions, vec![EffectConclusion::Unknown]);
}

#[test]
fn abort_converges_inflight_closes_decisions_and_never_resurrects() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = waiting_intent(&runtime);
    let request_id = runtime
        .projection(&spec.id)
        .unwrap()
        .open_requests
        .keys()
        .next()
        .unwrap()
        .clone();

    let cancelled = runtime.abort(&spec.id, "测试取消").unwrap();
    assert_eq!(cancelled.state, Some(IntentState::Cancelled));
    assert_converged(&runtime, &spec.id);

    // 终态 run/abort 幂等；旧 Decision 不得让执行复活。
    let event_count = runtime.events(&spec.id).unwrap().len();
    runtime.run(&spec.id).unwrap();
    runtime.abort(&spec.id, "重复取消").unwrap();
    assert!(runtime
        .decide(
            &spec.id,
            &request_id,
            DecisionChoice::Option("approve".into()),
        )
        .is_err());
    assert_eq!(runtime.events(&spec.id).unwrap().len(), event_count);

    // 重启只重放，不追加重复收尾事件。
    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_converged(&reopened, &spec.id);
    assert_eq!(reopened.events(&spec.id).unwrap().len(), event_count);
}

#[test]
fn kill_at_every_abort_event_boundary_recovers_exactly_once() {
    // 当前路径共 8 个 Abort 事件；每个持久化边界都模拟一次进程死亡。
    for cut in 1..=8 {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = waiting_intent(&runtime);
        runtime.crash_after_appends(cut);
        let error = runtime.abort(&spec.id, "kill 测试").unwrap_err();
        assert!(matches!(
            error,
            else_api::ApiError::Ledger(LedgerError::CrashInjected)
        ));
        drop(runtime);

        let recovered = Runtime::open(dir.path()).unwrap();
        assert_converged(&recovered, &spec.id);
        let count = recovered.events(&spec.id).unwrap().len();
        drop(recovered);
        let reopened = Runtime::open(dir.path()).unwrap();
        assert_eq!(reopened.events(&spec.id).unwrap().len(), count, "cut={cut}");
    }
}

#[test]
fn seeded_random_kill_points_always_recover() {
    // 固定种子的 LCG：覆盖随机 kill 点，同时保证失败可复现。
    let mut seed = 0x5eed_u64;
    for _ in 0..24 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let cut = seed % 8 + 1;
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(dir.path()).unwrap();
        let spec = waiting_intent(&runtime);
        runtime.crash_after_appends(cut);
        assert!(runtime.abort(&spec.id, "随机 kill").is_err());
        drop(runtime);

        let recovered = Runtime::open(dir.path()).unwrap();
        assert_converged(&recovered, &spec.id);
        assert!(recovered.run(&spec.id).is_ok());
        assert_converged(&recovered, &spec.id);
    }
}
