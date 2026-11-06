//! M1-W3：准入、根预算聚合、资源账重放与 Lease 回收。

use else_api::{ApiError, Runtime};
use else_gateway::GatewayError;
use else_protocol::effect::SideEffectKind;
use else_protocol::event::EventPayload;
use else_protocol::resource::{
    ResourceBudget, ResourceUsage, SandboxTerminationReason, SandboxUsageBasis,
};

#[test]
fn admission_rejects_over_budget_run_before_lease_or_execution() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(dir.path()).unwrap();
    let budget = ResourceBudget {
        max_model_spend_usd_micros: 1_000,
        ..Default::default()
    };
    runtime.set_default_budget(budget);
    let spec = runtime
        .create_thread("预算必须失败关闭", None, Default::default())
        .unwrap();

    let error = runtime.run(&spec.id).unwrap_err();
    assert!(matches!(
        error,
        ApiError::Gateway(GatewayError::ResourceExhausted { .. })
    ));
    let events = runtime.events(&spec.id).unwrap();
    assert!(!events.iter().any(|event| matches!(
        event.envelope.payload,
        EventPayload::LeaseGranted { .. }
            | EventPayload::RunStarted { .. }
            | EventPayload::ModelCalled { .. }
            | EventPayload::ToolCalled { .. }
    )));
    assert_eq!(
        runtime.resources(&spec.id).unwrap().usage,
        ResourceUsage::default()
    );
    let projection = runtime.projection(&spec.id).unwrap();
    assert_eq!(
        projection.state,
        Some(else_protocol::thread::ThreadState::Failed)
    );
    let root = projection.root_scope_id.as_ref().unwrap();
    assert_eq!(
        projection.scopes[root].state,
        else_protocol::event::ScopeState::Failed
    );
    assert_eq!(
        projection.scopes[root].last_failure.as_ref().unwrap().class,
        else_protocol::failure::FailureClass::Terminal
    );

    drop(runtime);
    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(reopened.resources(&spec.id).unwrap().budget, budget);
}

#[test]
fn accounting_and_lease_closure_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("资源账重放", None, Default::default())
        .unwrap();
    runtime.run(&spec.id).unwrap();

    let report = runtime.resources(&spec.id).unwrap();
    assert!(report.usage.model_tokens > 0);
    assert_eq!(report.usage.model_spend_usd_micros, 1_500);
    assert_eq!(report.usage.tool_calls, 2);
    assert_eq!(report.usage.sandbox_seconds, 1);
    assert_eq!(report.usage.sandbox_reserved_seconds, 0);
    assert_eq!(report.usage.sandbox_reported_seconds, 1);
    assert_eq!(report.usage.sandbox_estimated_seconds, 0);
    assert_eq!(report.usage.sandbox_unknown_seconds, 0);
    assert_eq!(report.sandboxes.len(), 1);
    let sandbox = &report.sandboxes[0];
    assert!(sandbox.finished_at >= sandbox.started_at);
    assert_eq!(sandbox.basis, SandboxUsageBasis::Reported);
    assert_eq!(
        sandbox.termination,
        SandboxTerminationReason::PausedForDecision
    );
    assert!(sandbox.backend == "seatbelt" || sandbox.backend == "bubblewrap");
    let events = runtime.events(&spec.id).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.envelope.payload, EventPayload::LeaseGranted { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.envelope.payload, EventPayload::LeaseRevoked { .. }))
            .count(),
        1
    );
    drop(runtime);

    let reopened = Runtime::open(dir.path()).unwrap();
    assert_eq!(reopened.resources(&spec.id).unwrap(), report);
}

#[test]
fn descendant_consumption_counts_against_root_admission() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(dir.path()).unwrap();
    runtime.set_default_budget(ResourceBudget {
        max_tool_calls: 5,
        ..Default::default()
    });
    let spec = runtime
        .create_thread("父子共享根预算", None, Default::default())
        .unwrap();
    runtime.run(&spec.id).unwrap();
    let root = runtime
        .projection(&spec.id)
        .unwrap()
        .scopes
        .iter()
        .find(|(_, scope)| scope.parent.is_none())
        .map(|(id, _)| id.clone())
        .unwrap();
    let child = runtime
        .spawn_subagent(&spec.id, &root, "子任务", SideEffectKind::Reversible)
        .unwrap();
    runtime.run_scope(&spec.id, &child).unwrap();
    assert_eq!(runtime.resources(&spec.id).unwrap().usage.tool_calls, 4);

    let grandchild = runtime
        .spawn_subagent(&spec.id, &child, "超额孙任务", SideEffectKind::Reversible)
        .unwrap();
    let error = runtime.run_scope(&spec.id, &grandchild).unwrap_err();
    assert!(matches!(
        error,
        ApiError::Gateway(GatewayError::ResourceExhausted { .. })
    ));
    assert_eq!(runtime.resources(&spec.id).unwrap().usage.tool_calls, 4);
    assert_eq!(
        runtime
            .resources(&spec.id)
            .unwrap()
            .usage
            .model_spend_usd_micros,
        3_000
    );
}

#[test]
fn abort_revokes_lease_left_by_crashed_run() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("崩溃租约回收", None, Default::default())
        .unwrap();
    runtime.crash_after_appends(4); // ContextCompiled → LeaseGranted → RunStarted → SandboxStarted。
    assert!(runtime.run(&spec.id).is_err());
    drop(runtime);

    let runtime = Runtime::open(dir.path()).unwrap();
    let reserved = runtime.resources(&spec.id).unwrap();
    assert_eq!(reserved.usage.sandbox_reserved_seconds, 900);
    assert_eq!(reserved.usage.sandbox_seconds, 0);
    runtime.abort(&spec.id, "回收崩溃执行").unwrap();
    let events = runtime.events(&spec.id).unwrap();
    let granted: std::collections::BTreeSet<_> = events
        .iter()
        .filter_map(|event| match &event.envelope.payload {
            EventPayload::LeaseGranted { lease } => Some(lease.id.clone()),
            _ => None,
        })
        .collect();
    let revoked: std::collections::BTreeSet<_> = events
        .iter()
        .filter_map(|event| match &event.envelope.payload {
            EventPayload::LeaseRevoked { lease_id } => Some(lease_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(granted.len(), 1);
    assert_eq!(granted, revoked);
    let report = runtime.resources(&spec.id).unwrap();
    assert_eq!(report.usage.sandbox_reserved_seconds, 0);
    assert_eq!(report.usage.sandbox_unknown_seconds, 1);
    assert_eq!(report.sandboxes[0].basis, SandboxUsageBasis::Unknown);
    assert_eq!(
        report.sandboxes[0].termination,
        SandboxTerminationReason::Aborted
    );
}

#[test]
fn crash_before_sandbox_start_uses_estimated_elapsed_time() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("启动前崩溃估算", None, Default::default())
        .unwrap();
    runtime.crash_after_appends(2); // ContextCompiled → LeaseGranted，随后 kill。
    assert!(runtime.run(&spec.id).is_err());
    drop(runtime);

    let runtime = Runtime::open(dir.path()).unwrap();
    runtime.abort(&spec.id, "回收未启动沙箱").unwrap();
    let report = runtime.resources(&spec.id).unwrap();
    assert_eq!(report.usage.sandbox_reserved_seconds, 0);
    assert_eq!(report.usage.sandbox_estimated_seconds, 1);
    assert_eq!(report.usage.sandbox_unknown_seconds, 0);
    assert_eq!(report.sandboxes[0].basis, SandboxUsageBasis::Estimated);
}

#[test]
fn consumed_actual_seconds_are_checked_conservatively_on_the_next_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(dir.path()).unwrap();
    runtime.set_default_budget(ResourceBudget {
        max_sandbox_seconds: 900,
        ..Default::default()
    });
    let spec = runtime
        .create_thread("沙箱预算熔断", None, Default::default())
        .unwrap();
    runtime.run(&spec.id).unwrap();

    let error = runtime.run(&spec.id).unwrap_err();
    assert!(matches!(
        error,
        ApiError::Gateway(GatewayError::ResourceExhausted {
            ref resource,
            ..
        }) if resource == "sandbox_seconds"
    ));
    assert_eq!(
        runtime.resources(&spec.id).unwrap().usage.sandbox_seconds,
        1
    );
}
