//! 只读连接器：World Signal、Resolver、Candidate 与 Thread 血缘。

use chrono::{DateTime, Duration, Utc};
use else_api::{ApiError, Runtime};
use else_connector::{AdapterRegistry, ConnectorAdapter, InterpretationRule, SignalInterpreter};
use else_protocol::connector::{
    ConnectorAuthMode, ConnectorHealth, ConnectorManifest, ConnectorObservation, ConnectorPolicy,
    ConnectorPullPage, ConnectorSyncMode, ObjectKind, ObservationChange, SignalInterpretation,
    SignalMeaning, SignalStatus, SyncCursor, ThreadAcceptedBy, ThreadCandidate,
    ThreadCandidateState, ThreadOriginKind, WorldSignal,
};
use else_protocol::effect::SideEffectKind;
use else_protocol::memory::SnapshotAudience;
use else_protocol::thread::{AttentionPolicy, Authority, Criterion};

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
) -> ThreadCandidate {
    ThreadCandidate {
        id: else_protocol::ids::ThreadCandidateId::new(),
        origin: ThreadOriginKind::AgentProposal,
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
        related_thread_id: None,
        confidence: 70,
        dedupe_key: dedupe_key.into(),
        state: ThreadCandidateState::Proposed,
        accepted_thread_id: None,
        accepted_by: None,
    }
}

fn linear_payload() -> serde_json::Value {
    let updated = Utc::now().to_rfc3339();
    let due = (Utc::now() + Duration::days(2))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    serde_json::json!({
        "data": {
            "issues": {
                "nodes": [
                    {
                        "id": "issue-commit",
                        "identifier": "ENG-101",
                        "title": "向客户交付评审材料",
                        "state": {"name": "In Progress", "type": "started"},
                        "updatedAt": updated,
                        "dueDate": due,
                        "url": "https://linear.app/acme/issue/ENG-101",
                        "assignee": {"name": "Zhuanz"},
                        "project": {"name": "Launch"},
                        "labels": {"nodes": [{"name": "commitment"}]},
                        "blockedBy": {"nodes": []},
                        "priority": 1
                    },
                    {
                        "id": "issue-wait",
                        "identifier": "ENG-102",
                        "title": "等待安全团队审批",
                        "state": {"name": "Blocked", "type": "started"},
                        "updatedAt": updated,
                        "dueDate": due,
                        "url": "https://linear.app/acme/issue/ENG-102",
                        "labels": {"nodes": [{"name": "waiting_external"}]},
                        "blockedBy": {"nodes": [{"id": "issue-security"}]},
                        "priority": 2
                    },
                    {
                        "id": "issue-action",
                        "identifier": "ENG-103",
                        "title": "补充发布说明",
                        "state": {"name": "Todo", "type": "unstarted"},
                        "updatedAt": updated,
                        "dueDate": due,
                        "url": "https://linear.app/acme/issue/ENG-103",
                        "labels": {"nodes": []},
                        "blockedBy": {"nodes": []},
                        "priority": 3
                    },
                    {
                        "id": "issue-done",
                        "identifier": "ENG-104",
                        "title": "归档旧文档",
                        "state": {"name": "Done", "type": "completed"},
                        "updatedAt": updated,
                        "labels": {"nodes": []},
                        "blockedBy": {"nodes": []}
                    }
                ]
            }
        }
    })
}

fn notion_payload() -> serde_json::Value {
    let updated = Utc::now().to_rfc3339();
    let due = (Utc::now() + Duration::days(2)).to_rfc3339();
    serde_json::json!({
        "object": "list",
        "results": [
            {
                "object": "page",
                "id": "notion-page-1",
                "created_time": updated,
                "last_edited_time": updated,
                "archived": false,
                "url": "https://notion.so/notion-page-1",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {
                    "Name": {
                        "type": "title",
                        "title": [{"plain_text": "发布说明文档"}]
                    }
                }
            },
            {
                "object": "page",
                "id": "notion-row-1",
                "created_time": updated,
                "last_edited_time": updated,
                "archived": false,
                "url": "https://notion.so/notion-row-1",
                "parent": {"type": "database_id", "database_id": "db-1"},
                "properties": {
                    "Name": {
                        "type": "title",
                        "title": [{"plain_text": "等待安全审批"}]
                    },
                    "Status": {
                        "type": "status",
                        "status": {"name": "Blocked", "group": "in_progress"}
                    },
                    "Due": {
                        "type": "date",
                        "date": {"start": due, "end": null}
                    },
                    "Blocked by": {
                        "type": "relation",
                        "relation": [{"id": "security-team"}]
                    }
                }
            },
            {
                "object": "page",
                "id": "notion-page-old",
                "created_time": updated,
                "last_edited_time": updated,
                "archived": true,
                "url": "https://notion.so/notion-page-old",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {
                    "Name": {
                        "type": "title",
                        "title": [{"plain_text": "旧文档"}]
                    }
                }
            }
        ]
    })
}

#[test]
fn calendar_signals_are_versioned_deduplicated_and_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("准备明天的会议", None, Default::default())
        .unwrap();
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
    assert_eq!(projection.thread_candidates.len(), 1);
    assert_eq!(projection.connectors[&connector.id].adapter, "calendar.ics");
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
        .thread_candidates
        .values()
        .any(|candidate| candidate.state == ThreadCandidateState::Proposed));
    assert!(projection
        .thread_candidates
        .values()
        .any(|candidate| candidate.state == ThreadCandidateState::Superseded));
    let proposed = projection
        .thread_candidates
        .values()
        .find(|candidate| candidate.state == ThreadCandidateState::Proposed)
        .unwrap()
        .clone();
    let accepted = runtime
        .resolve_thread_candidate(&spec.id, &proposed.id, true)
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
        replayed.thread_candidates[&proposed.id].state,
        ThreadCandidateState::Accepted
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
    let spec = runtime
        .create_thread("跟进会议", None, Default::default())
        .unwrap();
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
        projection.thread_candidates.keys().next().unwrap().clone(),
    );
    let signal_seq = projection.signal_seqs[&signal_id];

    runtime
        .set_world_signal_status(&spec.id, &signal_id, SignalStatus::Deleted)
        .unwrap();
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        projection.world_signals[&signal_id].status,
        SignalStatus::Deleted
    );
    assert_eq!(
        projection.thread_candidates[&candidate_id].state,
        ThreadCandidateState::Superseded
    );
    assert!(projection.external_events.is_empty());
    let updates = runtime
        .world_signals(signal_seq, Some("calendar"), None)
        .unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].signal.status, SignalStatus::Deleted);
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
    let spec = runtime
        .create_thread("准备客户评审", None, Default::default())
        .unwrap();
    let first = runtime
        .propose_thread_candidate(
            &spec.id,
            manual_candidate(
                "整理客户评审材料",
                "same-proposal",
                SideEffectKind::ReadOnly,
            ),
        )
        .unwrap();
    let duplicate = runtime
        .propose_thread_candidate(
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
            .thread_candidates
            .len(),
        1
    );

    let candidate_seq = runtime
        .thread_candidates(0)
        .unwrap()
        .into_iter()
        .find(|record| record.candidate.id == first.id)
        .unwrap()
        .seq;
    let accepted = runtime
        .resolve_thread_candidate(&spec.id, &first.id, true)
        .unwrap()
        .unwrap();
    let origin = accepted.origin.as_ref().unwrap();
    assert_eq!(origin.kind, ThreadOriginKind::AgentProposal);
    assert_eq!(origin.candidate_id.as_ref(), Some(&first.id));
    assert_eq!(
        origin.accepted_by,
        else_protocol::connector::ThreadAcceptedBy::User
    );
    let source = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        source.thread_candidates[&first.id].state,
        ThreadCandidateState::Accepted
    );
    assert_eq!(
        source.thread_candidates[&first.id]
            .accepted_thread_id
            .as_ref(),
        Some(&accepted.id)
    );
    let feed = runtime.thread_candidates(candidate_seq).unwrap();
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0].candidate.state, ThreadCandidateState::Accepted);
    assert_eq!(feed[0].candidate.accepted_by, Some(ThreadAcceptedBy::User));
    assert_eq!(runtime.list_threads().unwrap().len(), 2);
    assert_eq!(accepted.project, spec.project);

    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(
        reopened.projection(&spec.id).unwrap().thread_candidates[&first.id].state,
        ThreadCandidateState::Accepted
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
fn candidate_rejection_creates_no_thread() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("整理材料", None, Default::default())
        .unwrap();
    let candidate = runtime
        .propose_thread_candidate(
            &spec.id,
            manual_candidate("发送外部邮件", "reject-me", SideEffectKind::ReadOnly),
        )
        .unwrap();
    assert!(runtime
        .resolve_thread_candidate(&spec.id, &candidate.id, false)
        .unwrap()
        .is_none());
    assert_eq!(runtime.list_threads().unwrap().len(), 1);
    assert_eq!(
        runtime.projection(&spec.id).unwrap().thread_candidates[&candidate.id].state,
        ThreadCandidateState::Rejected
    );
}

#[test]
fn candidate_cannot_escalate_source_thread_authority() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("只读分析", None, Default::default())
        .unwrap();
    let error = runtime
        .propose_thread_candidate(
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
        .thread_candidates
        .is_empty());
}

#[test]
fn linear_pull_classifies_work_and_persists_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("推进发布", None, Default::default())
        .unwrap();
    let path = dir.path().join("linear.json");
    std::fs::write(&path, linear_payload().to_string()).unwrap();
    let connector = runtime
        .register_linear_connector(&spec.id, "engineering", path.to_str().unwrap())
        .unwrap();
    assert_eq!(connector.adapter, "linear.graphql");
    assert_eq!(connector.manifest.provider, "linear");
    assert!(connector
        .manifest
        .sync_modes
        .contains(&ConnectorSyncMode::Webhook));
    assert!(connector.webhook_session.is_some());
    let adapters = runtime.connector_manifests();
    assert!(adapters
        .iter()
        .any(|manifest| manifest.adapter == "calendar.ics"));
    assert!(adapters
        .iter()
        .any(|manifest| manifest.adapter == "linear.graphql"));
    runtime
        .wait_for_external(&spec.id, "等待 Linear 工作项变化")
        .unwrap();

    let first = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(first.observed, 4);
    assert_eq!(first.inserted, 4);
    assert_eq!(first.candidates_proposed, 3);
    assert!(first.cursor.is_some());
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        projection.state,
        Some(else_protocol::thread::ThreadState::Active)
    );
    assert_eq!(projection.world_signals.len(), 4);
    assert_eq!(projection.external_events.len(), 3);
    assert_eq!(
        projection
            .signal_interpretations
            .values()
            .flatten()
            .filter(|interpretation| interpretation.meaning == SignalMeaning::Commitment)
            .count(),
        1
    );
    assert_eq!(
        projection
            .signal_interpretations
            .values()
            .flatten()
            .filter(|interpretation| { interpretation.meaning == SignalMeaning::WaitingExternal })
            .count(),
        1
    );
    assert!(projection
        .world_signals
        .values()
        .filter(|signal| signal.status == SignalStatus::Active)
        .all(|signal| signal.object_kind == ObjectKind::WorkItem));
    assert_eq!(
        projection
            .world_signals
            .values()
            .filter(|signal| signal.status == SignalStatus::Deleted)
            .count(),
        1
    );
    assert_eq!(projection.connector_cursors.len(), 1);
    assert_eq!(
        projection.signal_interpretations.values().flatten().count(),
        3
    );
    let cursor = projection
        .connector_cursors
        .values()
        .next()
        .unwrap()
        .clone();
    let global_signals = runtime
        .world_signals(0, Some("linear"), Some("issue-commit"))
        .unwrap();
    assert_eq!(global_signals.len(), 1);
    assert_eq!(global_signals[0].thread_id, spec.id);
    let candidates = runtime.thread_candidates(0).unwrap();
    assert_eq!(candidates.len(), 3);
    let last_candidate_seq = candidates.last().unwrap().seq;
    assert!(runtime
        .thread_candidates(last_candidate_seq)
        .unwrap()
        .is_empty());
    assert!(runtime
        .world_signals(global_signals[0].seq, Some("linear"), None)
        .unwrap()
        .iter()
        .all(|record| record.seq > global_signals[0].seq));

    let second = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(second.inserted, 0);
    assert_eq!(second.skipped, 4);
    assert_eq!(second.candidates_proposed, 0);
    assert_eq!(second.cursor, first.cursor);

    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(
        reopened
            .projection(&spec.id)
            .unwrap()
            .connector_cursors
            .values()
            .next(),
        Some(&cursor)
    );
    assert_eq!(
        reopened
            .projection(&spec.id)
            .unwrap()
            .signal_interpretations
            .values()
            .flatten()
            .count(),
        3
    );
}

#[test]
fn linear_webhook_is_session_scoped_deduplicated_and_handles_completion() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("跟进外部依赖", None, Default::default())
        .unwrap();
    let path = dir.path().join("linear.json");
    std::fs::write(&path, r#"{"issues":[]}"#).unwrap();
    let connector = runtime
        .register_linear_connector(&spec.id, "engineering", path.to_str().unwrap())
        .unwrap();
    let session = connector.webhook_session.clone().unwrap();
    let issue = linear_payload()["data"]["issues"]["nodes"][1].clone();

    let error = runtime
        .ingest_connector_webhook(&spec.id, &connector.id, "wrong-session", issue.clone())
        .unwrap_err();
    assert!(matches!(error, ApiError::Unauthorized(_)));
    let first = runtime
        .ingest_connector_webhook(&spec.id, &connector.id, &session, issue.clone())
        .unwrap();
    assert_eq!(first.inserted, 1);
    assert_eq!(first.candidates_proposed, 1);
    let duplicate = runtime
        .ingest_connector_webhook(&spec.id, &connector.id, &session, issue.clone())
        .unwrap();
    assert_eq!(duplicate.inserted, 0);
    assert_eq!(duplicate.skipped, 1);
    assert_eq!(duplicate.candidates_proposed, 0);

    let mut completed = issue;
    completed["state"] = serde_json::json!({"name": "Done", "type": "completed"});
    completed["updatedAt"] = serde_json::json!((Utc::now() + Duration::seconds(1)).to_rfc3339());
    let completion = runtime
        .ingest_connector_webhook(&spec.id, &connector.id, &session, completed)
        .unwrap();
    assert_eq!(completion.inserted, 1);
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        projection
            .world_signals
            .values()
            .filter(|signal| signal.status == SignalStatus::Active)
            .count(),
        0
    );
    assert!(projection
        .thread_candidates
        .values()
        .all(|candidate| candidate.state == ThreadCandidateState::Superseded));
    assert!(runtime
        .compile_snapshot(&spec.id, SnapshotAudience::Planner)
        .unwrap()
        .external_events
        .is_empty());
}

#[test]
fn connector_policy_only_auto_accepts_explicit_low_risk_origins() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("推进发布", None, Default::default())
        .unwrap();
    let policy = ConnectorPolicy {
        auto_accept_low_risk: true,
        allowed_origins: vec![ThreadOriginKind::Commitment],
        min_confidence: 80,
        max_autonomous_effect: SideEffectKind::ReadOnly,
    };
    let revised = runtime
        .configure_connector_policy(&spec.id, policy.clone())
        .unwrap();
    assert_eq!(revised.revision, 2);
    assert_eq!(revised.connector_policy, policy);

    let path = dir.path().join("linear.json");
    std::fs::write(&path, linear_payload().to_string()).unwrap();
    let connector = runtime
        .register_linear_connector(&spec.id, "engineering", path.to_str().unwrap())
        .unwrap();
    let result = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(result.auto_accepted, 1);
    assert_eq!(runtime.list_threads().unwrap().len(), 2);
    let projection = runtime.projection(&spec.id).unwrap();
    let accepted = projection
        .thread_candidates
        .values()
        .find(|candidate| candidate.state == ThreadCandidateState::Accepted)
        .unwrap();
    assert_eq!(accepted.origin, ThreadOriginKind::Commitment);
    assert_eq!(accepted.accepted_by, Some(ThreadAcceptedBy::AutonomousRule));
    let child_id = accepted.accepted_thread_id.as_ref().unwrap();
    let child = runtime.projection(child_id).unwrap().spec.unwrap();
    assert_eq!(
        child.origin.unwrap().accepted_by,
        ThreadAcceptedBy::AutonomousRule
    );
    assert_eq!(
        projection
            .thread_candidates
            .values()
            .filter(|candidate| candidate.state == ThreadCandidateState::Proposed)
            .count(),
        2
    );
}

struct FixtureAdapter;

impl ConnectorAdapter for FixtureAdapter {
    fn id(&self) -> &'static str {
        "fixture.v1"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["fixture"]
    }

    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            adapter: self.id().into(),
            provider: "fixture".into(),
            auth_mode: ConnectorAuthMode::None,
            scopes: vec!["fixture.read".into()],
            read_capabilities: vec!["fixture.item.read".into()],
            write_capabilities: vec![],
            sync_modes: vec![ConnectorSyncMode::Pull],
            data_sensitivity: "internal".into(),
            retention_policy: "metadata_only".into(),
            health: ConnectorHealth::Healthy,
        }
    }

    fn pull_stream_id(&self) -> &'static str {
        "fixture-items"
    }

    fn pull(
        &self,
        connector_id: else_protocol::ids::ConnectorId,
        _source: &str,
        _cursor: Option<&SyncCursor>,
        now: DateTime<Utc>,
        _credential: Option<&else_protocol::connector::ConnectorCredential>,
    ) -> else_connector::Result<ConnectorPullPage> {
        Ok(ConnectorPullPage {
            stream_id: self.pull_stream_id().into(),
            opaque_cursor: "fixture:1".into(),
            overlap_until: None,
            continuation: None,
            observations: vec![ConnectorObservation {
                connector_id,
                entity_id: "fixture-1".into(),
                source_version: "1".into(),
                dedupe_key: "fixture:item:1".into(),
                source: "fixture".into(),
                summary: "外部承诺".into(),
                occurred_at: now,
                valid_until: None,
                entity_kind: "fixture_item".into(),
                canonical_url: None,
                object_kind: ObjectKind::Other,
                change: ObservationChange::Upsert,
                actor: None,
                attributes: serde_json::json!({}),
                evidence_ref: None,
                sensitivity: "internal".into(),
                source_trust: "test".into(),
                correlations: vec![],
            }],
        })
    }
}

struct PaginatedFixtureAdapter;

impl ConnectorAdapter for PaginatedFixtureAdapter {
    fn id(&self) -> &'static str {
        "fixture.paged"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["paged"]
    }

    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            adapter: self.id().into(),
            provider: "fixture".into(),
            auth_mode: ConnectorAuthMode::None,
            scopes: vec![],
            read_capabilities: vec!["fixture.item.read".into()],
            write_capabilities: vec![],
            sync_modes: vec![ConnectorSyncMode::Pull],
            data_sensitivity: "internal".into(),
            retention_policy: "metadata_only".into(),
            health: ConnectorHealth::Healthy,
        }
    }

    fn pull_stream_id(&self) -> &'static str {
        "fixture-paged-items"
    }

    fn pull(
        &self,
        connector_id: else_protocol::ids::ConnectorId,
        source: &str,
        cursor: Option<&SyncCursor>,
        now: DateTime<Utc>,
        _credential: Option<&else_protocol::connector::ConnectorCredential>,
    ) -> else_connector::Result<ConnectorPullPage> {
        let continuation = cursor.and_then(|cursor| cursor.continuation.as_deref());
        let (entity, page, next) = match continuation {
            None => ("page-1", 1, Some("page-2")),
            Some("page-2") => {
                if std::fs::read_to_string(source).unwrap().trim() != "ready" {
                    return Err(else_connector::ConnectorError::Http(
                        "simulated page two outage".into(),
                    ));
                }
                ("page-2", 2, None)
            }
            Some(other) => {
                return Err(else_connector::ConnectorError::Http(format!(
                    "unexpected continuation {other}"
                )))
            }
        };
        Ok(ConnectorPullPage {
            stream_id: self.pull_stream_id().into(),
            opaque_cursor: format!("fixture:page:{page}"),
            overlap_until: None,
            continuation: next.map(str::to_string),
            observations: vec![ConnectorObservation {
                connector_id,
                entity_id: entity.into(),
                source_version: "1".into(),
                dedupe_key: format!("fixture:{entity}:1"),
                source: "fixture".into(),
                summary: format!("分页事项 {entity}"),
                occurred_at: now,
                valid_until: None,
                entity_kind: "fixture_item".into(),
                canonical_url: None,
                object_kind: ObjectKind::Other,
                change: ObservationChange::Upsert,
                actor: None,
                attributes: serde_json::json!({}),
                evidence_ref: None,
                sensitivity: "internal".into(),
                source_trust: "test".into(),
                correlations: vec![],
            }],
        })
    }
}

struct FixtureRule;

impl InterpretationRule for FixtureRule {
    fn id(&self) -> &'static str {
        "fixture-test-rule"
    }

    fn interpret(&self, signal: &WorldSignal, _now: DateTime<Utc>) -> Option<SignalInterpretation> {
        (signal.entity_ref.provider == "fixture").then(|| SignalInterpretation {
            signal_id: signal.id.clone(),
            meaning: SignalMeaning::Commitment,
            confidence: 90,
            basis: vec!["fixture_contract".into()],
            deadline: None,
            ruleset_version: "fixture-v1".into(),
        })
    }
}

#[test]
fn injected_adapter_and_interpreter_do_not_require_runtime_changes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("fixture.json");
    std::fs::write(&source, "{}").unwrap();
    let runtime = Runtime::open_with_connector_runtime(
        dir.path(),
        AdapterRegistry::new(vec![Box::new(FixtureAdapter)]),
        SignalInterpreter::with_rules(vec![Box::new(FixtureRule)]),
    )
    .unwrap();
    let spec = runtime
        .create_thread("处理外部承诺", None, Default::default())
        .unwrap();
    let connector = runtime
        .register_connector_adapter(&spec.id, "fixture", source.to_str().unwrap(), "fixture")
        .unwrap();
    let result = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(result.inserted, 1);
    assert_eq!(result.candidates_proposed, 1);
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        projection
            .signal_interpretations
            .values()
            .flatten()
            .next()
            .unwrap()
            .meaning,
        SignalMeaning::Commitment
    );
    assert_eq!(
        projection.thread_candidates.values().next().unwrap().origin,
        ThreadOriginKind::Commitment
    );
}

#[test]
fn paginated_backfill_resumes_from_continuation_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("page-state");
    std::fs::write(&source, "fail").unwrap();
    let open_runtime = || {
        Runtime::open_with_connector_runtime(
            dir.path(),
            AdapterRegistry::new(vec![Box::new(PaginatedFixtureAdapter)]),
            SignalInterpreter::builtin(),
        )
        .unwrap()
    };
    let runtime = open_runtime();
    let spec = runtime
        .create_thread("分页同步外部事项", None, Default::default())
        .unwrap();
    let connector = runtime
        .register_connector_adapter(&spec.id, "paged", source.to_str().unwrap(), "fixture.paged")
        .unwrap();

    let interrupted = runtime.sync_connector(&spec.id, &connector.id).unwrap_err();
    assert!(interrupted
        .to_string()
        .contains("simulated page two outage"));
    let projection = runtime.projection(&spec.id).unwrap();
    let cursor = projection
        .connector_cursors
        .values()
        .next()
        .unwrap()
        .clone();
    assert_eq!(cursor.continuation.as_deref(), Some("page-2"));
    assert_eq!(projection.world_signals.len(), 1);
    let health = runtime.connector_health(&spec.id, &connector.id).unwrap();
    assert!(health.backfill_in_progress);
    assert_eq!(health.health, ConnectorHealth::Degraded);

    drop(runtime);
    std::fs::write(&source, "ready").unwrap();
    let reopened = open_runtime();
    let completed = reopened.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(completed.inserted, 1);
    assert!(completed.backfill_complete);
    assert!(!completed.has_more);
    let projection = reopened.projection(&spec.id).unwrap();
    assert_eq!(projection.world_signals.len(), 2);
    let cursor = projection.connector_cursors.values().next().unwrap();
    assert!(cursor.continuation.is_none());
    assert!(cursor.backfill_completed_at.is_some());
    assert_eq!(
        reopened
            .connector_health(&spec.id, &connector.id)
            .unwrap()
            .health,
        ConnectorHealth::Healthy
    );
}

#[test]
fn linear_webhook_id_only_hydrates_from_connector_source() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("处理 Linear 更新", None, Default::default())
        .unwrap();
    let source = dir.path().join("linear.json");
    std::fs::write(&source, linear_payload().to_string()).unwrap();
    let connector = runtime
        .register_linear_connector(&spec.id, "linear", source.to_str().unwrap())
        .unwrap();
    let result = runtime
        .ingest_connector_webhook(
            &spec.id,
            &connector.id,
            connector.webhook_session.as_deref().unwrap(),
            serde_json::json!({
                "action": "update",
                "type": "Issue",
                "data": {"id": "issue-commit"}
            }),
        )
        .unwrap();
    assert!(result.hydrated);
    assert_eq!(result.inserted, 1);
    let projection = runtime.projection(&spec.id).unwrap();
    let signal = projection.world_signals.values().next().unwrap();
    assert_eq!(signal.entity_ref.external_id, "issue-commit");
    assert!(signal.summary.contains("向客户交付评审材料"));
}

#[test]
fn notion_documents_and_database_rows_flow_through_generic_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("准备发布资料", None, Default::default())
        .unwrap();
    let path = dir.path().join("notion.json");
    std::fs::write(&path, notion_payload().to_string()).unwrap();
    let connector = runtime
        .register_connector_adapter(&spec.id, "docs", path.to_str().unwrap(), "notion.api")
        .unwrap();
    assert_eq!(connector.adapter, "notion.api");
    assert_eq!(connector.manifest.provider, "notion");

    let result = runtime.sync_connector(&spec.id, &connector.id).unwrap();
    assert_eq!(result.observed, 3);
    assert_eq!(result.inserted, 3);
    assert_eq!(result.candidates_proposed, 1);
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(projection.world_signals.len(), 3);
    assert_eq!(projection.external_events.len(), 2);
    assert_eq!(
        projection
            .world_signals
            .values()
            .filter(|signal| signal.object_kind == ObjectKind::Document)
            .count(),
        2
    );
    assert_eq!(
        projection
            .world_signals
            .values()
            .filter(|signal| signal.object_kind == ObjectKind::WorkItem)
            .count(),
        1
    );
    assert_eq!(
        projection
            .signal_interpretations
            .values()
            .flatten()
            .filter(|interpretation| interpretation.meaning == SignalMeaning::WaitingExternal)
            .count(),
        1
    );
    let candidate = projection.thread_candidates.values().next().unwrap();
    assert_eq!(candidate.origin, ThreadOriginKind::InferredNeed);
    assert!(candidate.proposed_outcome.contains("等待安全审批"));
    let snapshot = runtime
        .compile_snapshot(&spec.id, SnapshotAudience::Planner)
        .unwrap();
    assert_eq!(snapshot.external_events.len(), 2);
    assert!(snapshot
        .external_events
        .iter()
        .any(|event| event.summary.contains("发布说明文档")));
    assert!(snapshot
        .external_events
        .iter()
        .any(|event| event.summary.contains("等待安全审批")));
    assert!(runtime
        .connector_manifests()
        .iter()
        .any(|manifest| manifest.adapter == "notion.api"));
}
