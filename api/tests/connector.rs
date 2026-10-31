//! 只读连接器：World Signal、Resolver、Candidate 与 Intent 血缘。

use chrono::{DateTime, Duration, Utc};
use else_api::{ApiError, Runtime};
use else_protocol::connector::{
    IntentCandidate, IntentCandidateState, IntentOriginKind, SignalStatus,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::intent::{AttentionPolicy, Authority, Criterion};
use else_protocol::memory::SnapshotAudience;

fn calendar_ics(uid: &str, summary: &str, start: DateTime<Utc>) -> String {
    let end = start + Duration::hours(1);
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:{summary}\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        start.format("%Y%m%dT%H%M%SZ"),
        end.format("%Y%m%dT%H%M%SZ")
    )
}

fn manual_candidate(
    outcome: &str,
    dedupe_key: &str,
    max_effect: SideEffectKind,
) -> IntentCandidate {
    IntentCandidate {
        id: else_protocol::ids::IntentCandidateId::new(),
        origin: IntentOriginKind::AgentProposal,
        proposed_outcome: outcome.into(),
        done_when: vec![Criterion::manual("确认完成")],
        constraints: vec![],
        authority: Authority {
            max_autonomous_effect: max_effect,
            forbidden: vec![],
        },
        attention_policy: AttentionPolicy {
            allow_interrupt: false,
            batch_interval_minutes: 60,
        },
        deadline: None,
        related_signal_ids: vec![],
        related_intent_id: None,
        confidence: 70,
        dedupe_key: dedupe_key.into(),
        state: IntentCandidateState::Proposed,
        accepted_intent_id: None,
    }
}

#[test]
fn calendar_signals_are_versioned_deduplicated_and_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("准备明天的会议", None).unwrap();
    let start = Utc::now() + Duration::hours(2);
    let path = dir.path().join("calendar.ics");
    std::fs::write(&path, calendar_ics("meeting-1", "产品评审", start)).unwrap();
    let connector = runtime
        .register_calendar_connector(&spec.id, "work", path.to_str().unwrap())
        .unwrap();

    let first = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(first.observed, 1);
    assert_eq!(first.inserted, 1);
    assert_eq!(first.candidates_proposed, 1);
    let second = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(second.inserted, 0);
    assert_eq!(second.skipped, 1);
    assert_eq!(second.candidates_proposed, 0);

    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.world_signals.len(), 1);
    assert_eq!(projection.external_events.len(), 1);
    assert_eq!(projection.intent_candidates.len(), 1);
    assert_eq!(
        projection.connectors[&connector.id].kind,
        else_protocol::connector::ConnectorKind::CalendarReadOnly
    );
    let snapshot = runtime
        .compile_snapshot(&spec.id, SnapshotAudience::Planner)
        .unwrap();
    assert_eq!(snapshot.external_events.len(), 1);
    assert!(snapshot.external_events[0].summary.contains("产品评审"));

    std::fs::write(&path, calendar_ics("meeting-1", "产品评审（改期）", start)).unwrap();
    let edited = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(edited.inserted, 1, "{edited:?}");
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.world_signals.len(), 2);
    assert_eq!(
        projection
            .world_signals
            .values()
            .filter(|signal| signal.status == SignalStatus::Active)
            .count(),
        1
    );
    assert_eq!(projection.external_events.len(), 1);
    assert!(projection.external_events[0]
        .summary
        .contains("产品评审（改期）"));
    assert!(projection
        .intent_candidates
        .values()
        .any(|candidate| candidate.state == IntentCandidateState::Proposed));
    assert!(projection
        .intent_candidates
        .values()
        .any(|candidate| candidate.state == IntentCandidateState::Superseded));
    let proposed = projection
        .intent_candidates
        .values()
        .find(|candidate| candidate.state == IntentCandidateState::Proposed)
        .unwrap()
        .clone();
    let accepted = runtime
        .resolve_intent_candidate(&spec.id, &proposed.id, true)
        .unwrap()
        .unwrap();
    let origin = accepted.origin.as_ref().unwrap();
    assert_eq!(origin.candidate_id.as_ref(), Some(&proposed.id));
    assert_eq!(origin.connector_ids, vec![connector.id.clone()]);
    assert_eq!(origin.signal_ids, proposed.related_signal_ids);

    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    let replayed = reopened.projection(&spec.id).unwrap();
    assert_eq!(replayed.world_signals, projection.world_signals);
    assert_eq!(
        replayed.intent_candidates[&proposed.id].state,
        IntentCandidateState::Accepted
    );
    assert_eq!(replayed.external_events, projection.external_events);
    assert_eq!(
        reopened
            .projection(&accepted.id)
            .unwrap()
            .spec
            .unwrap()
            .origin,
        accepted.origin
    );
}

#[test]
fn signal_tombstone_removes_context_and_supersedes_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("跟进会议", None).unwrap();
    let path = dir.path().join("calendar.ics");
    let start = Utc::now() + Duration::hours(3);
    std::fs::write(&path, calendar_ics("meeting-2", "客户同步", start)).unwrap();
    let connector = runtime
        .register_calendar_connector(&spec.id, "work", path.to_str().unwrap())
        .unwrap();
    runtime.sync_connector(&spec.id, &connector.id).unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    let (signal_id, candidate_id) = (
        projection.world_signals.keys().next().unwrap().clone(),
        projection.intent_candidates.keys().next().unwrap().clone(),
    );

    runtime
        .set_world_signal_status(&spec.id, &signal_id, SignalStatus::Deleted)
        .unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        projection.world_signals[&signal_id].status,
        SignalStatus::Deleted
    );
    assert_eq!(
        projection.intent_candidates[&candidate_id].state,
        IntentCandidateState::Superseded
    );
    assert!(projection.external_events.is_empty());
    assert!(runtime
        .compile_snapshot(&spec.id, SnapshotAudience::Planner)
        .unwrap()
        .external_events
        .is_empty());
}

#[test]
fn candidate_acceptance_preserves_lineage_and_rejects_duplicate_proposals() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("准备客户评审", None).unwrap();
    let first = runtime
        .propose_intent_candidate(
            &spec.id,
            manual_candidate(
                "整理客户评审材料",
                "same-proposal",
                SideEffectKind::ReadOnly,
            ),
        )
        .unwrap();
    let duplicate = runtime
        .propose_intent_candidate(
            &spec.id,
            manual_candidate(
                "另一个描述但同一语义",
                "same-proposal",
                SideEffectKind::ReadOnly,
            ),
        )
        .unwrap();
    assert_eq!(duplicate.id, first.id);
    assert_eq!(
        runtime
            .projection(&spec.id)
            .unwrap()
            .intent_candidates
            .len(),
        1
    );

    let accepted = runtime
        .resolve_intent_candidate(&spec.id, &first.id, true)
        .unwrap()
        .unwrap();
    let origin = accepted.origin.as_ref().unwrap();
    assert_eq!(origin.kind, IntentOriginKind::AgentProposal);
    assert_eq!(origin.candidate_id.as_ref(), Some(&first.id));
    assert_eq!(
        origin.accepted_by,
        else_protocol::connector::IntentAcceptedBy::User
    );
    let source = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        source.intent_candidates[&first.id].state,
        IntentCandidateState::Accepted
    );
    assert_eq!(
        source.intent_candidates[&first.id]
            .accepted_intent_id
            .as_ref(),
        Some(&accepted.id)
    );
    assert_eq!(runtime.list_intents().unwrap().len(), 2);
    assert_eq!(accepted.project, spec.project);

    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(
        reopened.projection(&spec.id).unwrap().intent_candidates[&first.id].state,
        IntentCandidateState::Accepted
    );
    assert_eq!(
        reopened
            .projection(&accepted.id)
            .unwrap()
            .spec
            .unwrap()
            .origin,
        accepted.origin
    );
}

#[test]
fn candidate_rejection_creates_no_intent() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("整理材料", None).unwrap();
    let candidate = runtime
        .propose_intent_candidate(
            &spec.id,
            manual_candidate("发送外部邮件", "reject-me", SideEffectKind::ReadOnly),
        )
        .unwrap();
    assert!(runtime
        .resolve_intent_candidate(&spec.id, &candidate.id, false)
        .unwrap()
        .is_none());
    assert_eq!(runtime.list_intents().unwrap().len(), 1);
    assert_eq!(
        runtime.projection(&spec.id).unwrap().intent_candidates[&candidate.id].state,
        IntentCandidateState::Rejected
    );
}

#[test]
fn candidate_cannot_escalate_source_intent_authority() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("只读分析", None).unwrap();
    let error = runtime
        .propose_intent_candidate(
            &spec.id,
            manual_candidate(
                "直接发布",
                "authority-escalation",
                SideEffectKind::Irreversible,
            ),
        )
        .unwrap_err();
    assert!(matches!(error, ApiError::PermissionEscalation(_)));
    assert!(runtime
        .projection(&spec.id)
        .unwrap()
        .intent_candidates
        .is_empty());
}
