//! MVP-2-W1：Harness 适配契约——进程 Harness 桥接事件、能力落账、
//! 失败收敛、终态幂等。真实 Codex/Claude 适配器复用同一契约。

use else_api::Runtime;
use else_harness::adapter::{shell_quote, BridgeEvent, LineParser, ProcessHarness};
use else_protocol::event::EventPayload;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::intent::IntentState;
use else_protocol::resource::{ModelUsage, UsageBasis};

/// 测试用 NDJSON 行解析器：真实适配器（claude/codex）的同构缩小版。
struct JsonParser;

impl LineParser for JsonParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent> {
        let event = (|| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            match value["type"].as_str()? {
                "output" => Some(BridgeEvent::Output {
                    text: value["text"].as_str()?.into(),
                }),
                "tool" => Some(BridgeEvent::ToolObserved {
                    tool: value["tool"].as_str()?.into(),
                    effect: serde_json::from_value(value["effect"].clone()).ok()?,
                    summary: value["summary"].as_str()?.into(),
                    success: value["success"].as_bool()?,
                }),
                "artifact" => Some(BridgeEvent::Artifact {
                    locator: value["locator"].as_str()?.into(),
                    note: value["note"].as_str()?.into(),
                }),
                "usage" => Some(BridgeEvent::Usage {
                    model: value["model"].as_str()?.into(),
                    usage: ModelUsage {
                        input_tokens: value["input"].as_u64()?,
                        output_tokens: value["output"].as_u64()?,
                        cost_usd_micros: value["cost"].as_u64()?,
                        basis: UsageBasis::Reported,
                    },
                }),
                "completed" => Some(BridgeEvent::Completed {
                    summary: value["summary"].as_str()?.into(),
                }),
                "failed" => Some(BridgeEvent::Failed {
                    reason: value["reason"].as_str()?.into(),
                }),
                _ => None,
            }
        })();
        event.into_iter().collect()
    }
}

fn fake_harness(script: &'static str) -> ProcessHarness<JsonParser> {
    ProcessHarness {
        name: "fake-harness",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::Reported,
            tool_effects: ToolEffectVisibility::Observable,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        timeout: None,
        prepare: Box::new(move |_, _, _| {
            Ok(else_harness::adapter::HarnessPrep {
                command: format!("touch plan.md; printf '%s' {}", shell_quote(script)),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: JsonParser,
    }
}

const SUCCESS_SCRIPT: &str = r#"{"type":"output","text":"开始执行任务"}
{"type":"tool","tool":"fs.write","effect":"reversible","summary":"写入报告","success":true}
{"type":"artifact","locator":"report.md","note":"最终报告"}
{"type":"usage","model":"fake-1","input":120,"output":80,"cost":500}
{"type":"completed","summary":"任务完成"}
"#;

#[test]
fn process_harness_bridges_events_and_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("桥接契约验证", None).unwrap();

    let projection = runtime
        .run_adapter(&spec.id, &fake_harness(SUCCESS_SCRIPT))
        .unwrap();
    assert_eq!(projection.state, Some(IntentState::Completed));

    let events = runtime.events(&spec.id).unwrap();
    // 能力随 RunStarted 落账（Compare 可见能力差异）
    let run_started = events
        .iter()
        .find_map(|event| match &event.envelope.payload {
            EventPayload::RunStarted {
                harness,
                capabilities,
                ..
            } => Some((harness.clone(), *capabilities)),
            _ => None,
        })
        .expect("RunStarted");
    assert_eq!(run_started.0, "fake-harness");
    let capabilities = run_started.1.expect("能力必须落账");
    assert_eq!(capabilities.cancel, CancelSupport::KillOnly);
    assert_eq!(capabilities.tool_effects, ToolEffectVisibility::Observable);

    let payloads: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(&e.envelope.payload).unwrap())
        .collect();
    assert!(payloads.iter().any(|p| p.contains("harness_output")));
    assert!(payloads.iter().any(|p| p.contains("tool_called")));
    assert!(payloads.iter().any(|p| p.contains("tool_completed")));
    assert!(payloads.iter().any(|p| p.contains("artifact_produced")));
    assert!(payloads.iter().any(|p| p.contains("outcome_verified")));

    // 用量按 Reported 入账，不是零成本 Unknown
    let usage = events
        .iter()
        .find_map(|event| match &event.envelope.payload {
            EventPayload::ModelCompleted { usage, .. } => Some(*usage),
            _ => None,
        })
        .expect("ModelCompleted");
    assert_eq!(usage.basis, UsageBasis::Reported);
    assert_eq!(usage.input_tokens, 120);

    // 执行事件携带 Context Snapshot
    assert!(events.iter().any(|e| e.envelope.snapshot_id.is_some()));
}

#[test]
fn failed_harness_converges_without_completing() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("失败收敛", None).unwrap();
    let script = r#"{"type":"output","text":"尝试中"}
{"type":"failed","reason":"依赖服务不可达"}
"#;
    let error = runtime
        .run_adapter(&spec.id, &fake_harness(script))
        .unwrap_err();
    assert!(error.to_string().contains("依赖服务不可达"));

    let events = runtime.events(&spec.id).unwrap();
    // RunFinished{success:false} 落账；没有验收，Intent 不进入 Completed
    let finished = events.iter().any(|event| {
        matches!(
            &event.envelope.payload,
            EventPayload::RunFinished { success: false, .. }
        )
    });
    assert!(finished);
    assert!(!events
        .iter()
        .any(|e| matches!(e.envelope.payload, EventPayload::OutcomeVerified { .. })));
    // Plan/Harness 失败不等于 Intent 失败：状态保持可重跑，不被标记终态
    let state = runtime.projection(&spec.id).unwrap().state;
    assert!(
        !matches!(
            state,
            Some(IntentState::Completed | IntentState::Failed | IntentState::Cancelled)
        ),
        "Harness 失败不得把 Intent 标记为终态，实际 {state:?}"
    );
}

#[test]
fn harness_exit_without_completion_is_failure() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("静默退出", None).unwrap();
    // 进程正常退出但从未报告 Completed：不能当成成功
    let error = runtime
        .run_adapter(
            &spec.id,
            &fake_harness("{\"type\":\"output\",\"text\":\"...\"}\n"),
        )
        .unwrap_err();
    assert!(error.to_string().contains("without completion"));
    let state = runtime.projection(&spec.id).unwrap().state;
    assert!(!matches!(state, Some(IntentState::Completed)));
}

#[test]
fn terminal_intent_rerun_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("终态幂等", None).unwrap();
    runtime
        .run_adapter(&spec.id, &fake_harness(SUCCESS_SCRIPT))
        .unwrap();
    let before = runtime.events(&spec.id).unwrap().len();
    let again = runtime
        .run_adapter(&spec.id, &fake_harness(SUCCESS_SCRIPT))
        .unwrap();
    assert_eq!(again.state, Some(IntentState::Completed));
    assert_eq!(runtime.events(&spec.id).unwrap().len(), before);
}
