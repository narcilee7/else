//! MVP-2 故障矩阵：启动失败、崩溃、超时、晚到输出、用量缺失。
//! 每种故障都必须安全收敛：RunFinished 落账、不冒充成功、不产生半截状态。

use else_api::Runtime;
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::event::EventPayload;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::ids::ThreadId;
use else_protocol::resource::{ModelUsage, UsageBasis};
use else_protocol::thread::ThreadState;

struct JsonParser;

impl LineParser for JsonParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent> {
        let event = (|| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            match value["type"].as_str()? {
                "output" => Some(BridgeEvent::Output {
                    text: value["text"].as_str()?.into(),
                }),
                "completed" => Some(BridgeEvent::Completed {
                    summary: value["summary"].as_str().unwrap_or_default().into(),
                }),
                _ => None,
            }
        })();
        event.into_iter().collect()
    }
}

fn fake_harness(
    script: &'static str,
    timeout: Option<std::time::Duration>,
) -> ProcessHarness<JsonParser> {
    ProcessHarness {
        name: "fake-failure-harness",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Observable,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        egress_allowlist: &[],
        timeout,
        prepare: Box::new(move |_, _, _| {
            Ok(HarnessPrep {
                command: script.into(),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: JsonParser,
    }
}

fn payloads(runtime: &Runtime, thread_id: &ThreadId) -> Vec<String> {
    runtime
        .events(thread_id)
        .unwrap()
        .iter()
        .map(|e| serde_json::to_string(&e.envelope.payload).unwrap())
        .collect()
}

/// 启动失败：准备阶段就失败，RunFinished{false} 落账，不冒充成功。
#[test]
fn prepare_failure_is_startup_failure() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("启动失败", None, Default::default())
        .unwrap();
    let mut harness = fake_harness("true", None);
    harness.prepare = Box::new(|_, _, _| {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "harness binary codex not found on PATH",
        ))
    });
    let error = runtime.run_adapter(&spec.id, &harness).unwrap_err();
    assert!(error.to_string().contains("not found"));

    let events = payloads(&runtime, &spec.id);
    assert!(events
        .iter()
        .any(|p| p.contains("run_finished") && p.contains("\"success\":false")));
    assert!(!events.iter().any(|p| p.contains("outcome_verified")));
}

/// 崩溃：非零退出且无 Completed，stderr 尾部成为失败证据。
#[test]
fn crash_exit_carries_stderr_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("崩溃证据", None, Default::default())
        .unwrap();
    let error = runtime
        .run_adapter(
            &spec.id,
            &fake_harness("echo 'fatal: sandbox died' >&2; exit 3", None),
        )
        .unwrap_err();
    assert!(error.to_string().contains("without completion"));
    assert!(error.to_string().contains("sandbox died"));
}

/// 超时：看门狗杀进程判失败，不无限等待。
#[test]
fn timeout_kills_process_and_fails() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("超时收敛", None, Default::default())
        .unwrap();
    let started = std::time::Instant::now();
    let error = runtime
        .run_adapter(
            &spec.id,
            &fake_harness("sleep 30", Some(std::time::Duration::from_secs(1))),
        )
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "超时必须在秒级收敛"
    );
    let events = payloads(&runtime, &spec.id);
    assert!(events
        .iter()
        .any(|p| p.contains("run_finished") && p.contains("\"success\":false")));
    let report = runtime.resources(&spec.id).unwrap();
    assert_eq!(
        report.sandboxes[0].elapsed_millis,
        report.usage.sandbox_actual_millis
    );
    assert_eq!(
        report.sandboxes[0].termination,
        else_protocol::resource::SandboxTerminationReason::TimedOut
    );
    assert_eq!(
        report.sandboxes[0].basis,
        else_protocol::resource::SandboxUsageBasis::Reported
    );
}

/// Abort 后晚到输出不得写回：执行中从另一个连接取消，
/// 围栏拒绝后续桥接事件，Harness 进程被杀。
#[test]
fn abort_mid_run_fences_late_output() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("运行中取消", None, Default::default())
        .unwrap();
    let thread_id = spec.id.clone();

    // 慢速 Harness：先发一行输出，睡 3 秒后才报告完成
    let script = "printf '%s\n' '{\"type\":\"output\",\"text\":\"第一步\"}'; sleep 3; \
                  printf '%s\n' '{\"type\":\"completed\",\"summary\":\"完成\"}'";
    let abort_dir = dir.path().to_path_buf();
    let abort_thread = thread_id.clone();
    let aborter = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        let runtime = Runtime::open(&abort_dir).unwrap();
        for _ in 0..20 {
            if runtime.abort(&abort_thread, "测试中途取消").is_ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        panic!("abort 未成功");
    });

    let result = runtime.run_adapter(&thread_id, &fake_harness(script, None));
    aborter.join().unwrap();

    // 执行被围栏：返回错误，绝不容忍晚到的 Completed 验收
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("generation fenced"),
        "实际错误：{error}"
    );
    let events = runtime.events(&thread_id).unwrap();
    let outputs = events
        .iter()
        .filter(|e| matches!(e.envelope.payload, EventPayload::HarnessOutput { .. }))
        .count();
    assert_eq!(outputs, 1, "只有 Abort 前的一行输出允许落账");
    assert!(!events
        .iter()
        .any(|e| matches!(e.envelope.payload, EventPayload::OutcomeVerified { .. })));
    assert_eq!(
        runtime.projection(&thread_id).unwrap().state,
        Some(ThreadState::Cancelled)
    );
}

/// 用量缺失：不能记成零成本，按 Unknown + 保守预算入账。
#[test]
fn missing_usage_is_unknown_not_zero() {
    use else_harness::real::CodexParser;
    let events = CodexParser.parse(r#"{"type":"turn.completed"}"#);
    let [BridgeEvent::Usage { usage, .. }, BridgeEvent::Completed { .. }] = events.as_slice()
    else {
        panic!("turn.completed 必须产生 Usage + Completed，实际 {events:?}");
    };
    assert_eq!(usage.basis, UsageBasis::Unknown);
    assert!(
        usage.input_tokens > 0,
        "Unknown 用量必须按保守预算记账，实际 {usage:?}"
    );

    let parser_usage = ModelUsage::unknown();
    assert_eq!(parser_usage.basis, UsageBasis::Unknown);
}

#[test]
fn completion_without_done_when_evidence_is_not_verified() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_thread("必须真实产出文件", None, Default::default())
        .unwrap();
    let script = r#"printf '%s\n' '{"type":"completed","summary":"我完成了"}'"#;
    let error = runtime
        .run_adapter(&spec.id, &fake_harness(script, None))
        .unwrap_err();
    assert!(
        error.to_string().contains("outcome verification failed"),
        "实际错误：{error}"
    );
    let events = runtime.events(&spec.id).unwrap();
    assert!(!events
        .iter()
        .any(|event| matches!(event.envelope.payload, EventPayload::OutcomeVerified { .. })));
    assert_ne!(
        runtime.projection(&spec.id).unwrap().state,
        Some(ThreadState::Completed)
    );
}
