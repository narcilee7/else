//! 重放一致性是 M0 的永久回归测试：
//! 事件写入 → 模拟进程崩溃（关闭连接）→ 重开 → 重放，投影必须一致。

use else_ledger::{IntentProjection, Ledger};
use else_protocol::attention::{
    AttentionKind, AttentionRequest, Decision, DecisionChoice, DecisionOption, ExpectedLoss,
};
use else_protocol::effect::{SideEffectKind, ToolCall};
use else_protocol::event::{Actor, EventEnvelope, EventPayload};
use else_protocol::ids::*;
use else_protocol::intent::{AttentionPolicy, Authority, Criterion, IntentSpec, IntentState};

fn spec(intent_id: &IntentId) -> IntentSpec {
    IntentSpec {
        id: intent_id.clone(),
        revision: 1,
        outcome: "今天达到可对外发布".into(),
        done_when: vec![Criterion {
            description: "构建通过且产物可运行".into(),
        }],
        constraints: vec!["不牺牲现有稳定性".into()],
        authority: Authority {
            max_autonomous_effect: SideEffectKind::Reversible,
            forbidden: vec!["未经批准不得对外发送消息".into()],
        },
        attention_policy: AttentionPolicy {
            allow_interrupt: true,
            batch_interval_minutes: 30,
        },
        deadline: None,
        assumptions: vec![],
        project: None,
    }
}

fn request() -> AttentionRequest {
    AttentionRequest {
        id: AttentionId::new(),
        kind: AttentionKind::Authorization,
        summary: "是否批准对外发布公告？".into(),
        blocked_reason: "对外发送属于不可逆操作，超出当前授权".into(),
        expected_loss: ExpectedLoss::High,
        blocking: true,
        reversible: false,
        decide_by: None,
        options: vec![DecisionOption {
            id: "approve".into(),
            label: "批准发送".into(),
            recommended: true,
        }],
        default_option: Some("approve".into()),
        related_call_key: None,
    }
}

/// 一条最小闭环：创建 → 执行 → 工具调用 → 本人决策 → 完成。
fn closed_loop_events(intent_id: &IntentId, scope_id: &ScopeId) -> Vec<EventEnvelope> {
    let run_id = RunId::new();
    let req = request();
    let decision = Decision {
        request_id: req.id.clone(),
        choice: DecisionChoice::Option("approve".into()),
        rationale: None,
        decided_by: Actor::User,
    };
    vec![
        EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            1,
            1,
            Actor::User,
            EventPayload::IntentCreated {
                spec: spec(intent_id),
            },
        ),
        EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            1,
            1,
            Actor::System,
            EventPayload::RunStarted {
                run_id: run_id.clone(),
                harness: "dummy-harness".into(),
                capabilities: None,
            },
        ),
        EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            1,
            1,
            Actor::Agent,
            EventPayload::ToolCalled {
                call: ToolCall {
                    id: ToolCallId::new(),
                    tool: "fs.write".into(),
                    effect: SideEffectKind::Reversible,
                    input: serde_json::json!({"path": "release-notes.txt"}),
                    idempotency_key: "k-1".into(),
                },
            },
        ),
        EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            1,
            1,
            Actor::Agent,
            EventPayload::AttentionRequested { request: req },
        ),
        EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            1,
            1,
            Actor::User,
            EventPayload::DecisionResolved { decision },
        ),
        EventEnvelope::new(
            intent_id.clone(),
            scope_id.clone(),
            1,
            1,
            Actor::System,
            EventPayload::IntentStateChanged {
                state: IntentState::Completed,
            },
        ),
    ]
}

fn replay_projection(ledger: &Ledger) -> IntentProjection {
    let mut projection = IntentProjection::default();
    ledger.replay(&mut projection).unwrap();
    projection
}

#[test]
fn replay_after_crash_is_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("else.db");

    let before = {
        let ledger = Ledger::open(&db_path).unwrap();
        let intent_id = IntentId::new();
        let scope_id = ScopeId::new();
        for event in closed_loop_events(&intent_id, &scope_id) {
            ledger.append(&event).unwrap();
        }
        replay_projection(&ledger)
        // ledger 在此 drop：模拟进程崩溃/退出
    };

    // 重开同一数据库，重放必须得到完全一致的投影
    let ledger = Ledger::open(&db_path).unwrap();
    let after = replay_projection(&ledger);
    assert_eq!(before, after);

    assert_eq!(after.state, Some(IntentState::Completed));
    assert!(after.open_requests.is_empty(), "决策后队列必须清空");
    assert_eq!(after.cursor, 6);
    assert_eq!(after.spec.unwrap().revision, 1);
}

#[test]
fn seq_is_monotonic_and_cursor_tracks_watermark() {
    let ledger = Ledger::open_in_memory().unwrap();
    let intent_id = IntentId::new();
    let scope_id = ScopeId::new();
    let events = closed_loop_events(&intent_id, &scope_id);

    for (i, event) in events.iter().enumerate() {
        let stored = ledger.append(event).unwrap();
        assert_eq!(stored.seq, (i + 1) as u64);
    }
    assert_eq!(ledger.cursor().unwrap(), 6);

    // 增量重放：检查点水位之后的事件
    assert_eq!(ledger.read_after(3).unwrap().len(), 3);
    assert_eq!(ledger.read_intent(&intent_id).unwrap().len(), 6);
}

#[test]
fn decision_queue_tracks_open_requests() {
    let ledger = Ledger::open_in_memory().unwrap();
    let intent_id = IntentId::new();
    let scope_id = ScopeId::new();
    let events = closed_loop_events(&intent_id, &scope_id);

    // 只重放到决策之前：队列里应有一条待本人决策
    for event in &events[..4] {
        ledger.append(event).unwrap();
    }
    let projection = replay_projection(&ledger);
    assert_eq!(projection.open_requests.len(), 1);
}

#[test]
fn incompatible_protocol_version_is_rejected() {
    let ledger = Ledger::open_in_memory().unwrap();
    let intent_id = IntentId::new();
    let scope_id = ScopeId::new();
    let mut event = EventEnvelope::new(
        intent_id,
        scope_id,
        1,
        1,
        Actor::System,
        EventPayload::IntentStateChanged {
            state: IntentState::Active,
        },
    );
    event.protocol_version = "0.0.0".into();
    ledger.append(&event).unwrap();

    let mut projection = IntentProjection::default();
    let err = ledger.replay(&mut projection).unwrap_err();
    assert!(matches!(
        err,
        else_ledger::LedgerError::UnsupportedVersion { .. }
    ));
}
