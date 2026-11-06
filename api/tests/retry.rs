use else_api::Runtime;
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::event::ScopeState;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::thread::ThreadState;

struct EmptyParser;

impl LineParser for EmptyParser {
    fn parse(&self, _line: &str) -> Vec<BridgeEvent> {
        vec![]
    }
}

fn failing_harness() -> ProcessHarness<EmptyParser> {
    ProcessHarness {
        name: "retry-failure",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Observable,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        egress_allowlist: &[],
        timeout: None,
        prepare: Box::new(|_, _, _| {
            Ok(HarnessPrep {
                command: "exit 9".into(),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: EmptyParser,
    }
}

#[test]
fn explicit_retry_resets_failed_scope_and_returns_to_kernel() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("retry failed scope", None, Default::default())
        .unwrap();
    let root = runtime.projection(&spec.id).unwrap().root_scope_id.unwrap();
    let harness = failing_harness();
    for _ in 0..3 {
        let _ = runtime.dispatch_scope_adapter(&spec.id, &root, &harness);
    }
    let failed = runtime.projection(&spec.id).unwrap();
    assert_eq!(failed.scopes[&root].state, ScopeState::Failed);
    assert_eq!(failed.state, Some(ThreadState::Failed));

    let (retried, projection) = runtime.retry_failed_scopes(&spec.id).unwrap();
    assert_eq!(retried, vec![root.clone()]);
    assert_ne!(projection.state, Some(ThreadState::Failed));
    assert_eq!(projection.scopes[&root].failure_count, 0);
    assert!(projection.scopes[&root].generation > failed.scopes[&root].generation);
}
