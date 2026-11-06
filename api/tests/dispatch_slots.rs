//! MVP-3 固定槽位：活跃 Run 到顶则拒绝启动（等待/降级由上层选择）。
//! 独立测试文件：需要修改进程级环境变量，避免与同文件并行测试互相干扰。

use else_api::Runtime;
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::effect::SideEffectKind;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::ids::ThreadId;

struct NullParser;
impl LineParser for NullParser {
    fn parse(&self, _line: &str) -> Vec<BridgeEvent> {
        vec![]
    }
}

fn fake_adapter() -> ProcessHarness<NullParser> {
    ProcessHarness {
        dynamic_egress_hosts: Vec::new,
        name: "fake-slot-harness",
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
                command: "true".into(),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: NullParser,
    }
}

fn root_scope(runtime: &Runtime, thread_id: &ThreadId) -> else_protocol::ids::ScopeId {
    runtime
        .projection(thread_id)
        .unwrap()
        .scopes
        .iter()
        .find(|(_, scope)| scope.parent.is_none())
        .map(|(id, _)| id.clone())
        .unwrap()
}

#[test]
fn slot_exhaustion_rejects_run_before_any_event() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread_with_done_when("槽位测试", None, Default::default(), plan_done())
        .unwrap();
    runtime.run(&spec.id).unwrap();
    let root = root_scope(&runtime, &spec.id);
    let child = runtime
        .spawn_subagent(&spec.id, &root, "子任务", SideEffectKind::ReadOnly)
        .unwrap();

    // 根 Run 结束后槽位为 0：新的派发必须被准入拒绝。
    unsafe { std::env::set_var("ELSE_MAX_RUN_SLOTS", "0") };
    let before = runtime.events(&spec.id).unwrap().len();
    let error = runtime
        .dispatch_scope_adapter(&spec.id, &child, &fake_adapter())
        .unwrap_err();
    assert!(error.to_string().contains("run_slots"), "实际错误：{error}");
    // 准入拒绝：不产生任何新事件（无 RunStarted、无 Lease）
    assert_eq!(runtime.events(&spec.id).unwrap().len(), before);
    unsafe { std::env::remove_var("ELSE_MAX_RUN_SLOTS") };
}

/// 夹具 Harness 写 plan.md：用机器可查的验收标准，让用例专注在它要验证的机制上。
fn plan_done() -> Vec<else_protocol::thread::Criterion> {
    vec![else_protocol::thread::Criterion::file_exists(
        "计划产物存在",
        "plan.md",
    )]
}
