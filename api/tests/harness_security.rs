//! MVP-4 安全回归：环境凭据不泄漏、网络默认拒绝、无 Sandbox 回退。

use else_api::Runtime;
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};

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

fn probe_harness(script: &'static str) -> ProcessHarness<JsonParser> {
    ProcessHarness {
        name: "probe-harness",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Opaque,
            interactive_input: false,
        },
        network: else_environment::NetworkPolicy::Deny,
        timeout: None,
        prepare: Box::new(move |_, _| {
            Ok(HarnessPrep {
                command: script.into(),
                env: vec![],
                read_paths: vec![],
            })
        }),
        parser: JsonParser,
    }
}

/// 宿主环境变量不得随进程继承：Harness 只能看到显式注入的白名单。
#[test]
fn host_env_credentials_do_not_leak_into_sandbox() {
    unsafe { std::env::set_var("ELSE_TEST_SECRET_SENTINEL", "super-secret-value") };
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("凭据泄漏探针", None).unwrap();
    // Harness 把环境变量值写进输出；若泄漏，事件流里会出现哨兵值
    runtime
        .run_adapter(
            &spec.id,
            &probe_harness(
                r#"printf '%s\n' "{\"type\":\"output\",\"text\":\"secret=[$ELSE_TEST_SECRET_SENTINEL]\"}"
printf '%s\n' '{"type":"completed","summary":"probe"}'"#,
            ),
        )
        .unwrap();
    unsafe { std::env::remove_var("ELSE_TEST_SECRET_SENTINEL") };

    let events = runtime.events(&spec.id).unwrap();
    let output = events
        .iter()
        .find_map(|event| match &event.envelope.payload {
            else_protocol::event::EventPayload::HarnessOutput { text } => Some(text.clone()),
            _ => None,
        })
        .expect("probe output");
    assert!(
        output.contains("secret=[]"),
        "沙箱内不得看到宿主环境变量，实际输出：{output}"
    );
    assert!(!output.contains("super-secret-value"));
}

/// 网络默认拒绝：Harness 未显式放通时出站必须失败。
#[test]
fn network_is_denied_by_default_for_harnesses() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("网络拒绝探针", None).unwrap();
    let error = runtime
        .run_adapter(
            &spec.id,
            &probe_harness(
                "curl -sS --max-time 5 https://example.com -o /dev/null && \
                 printf '{\"type\":\"completed\",\"summary\":\"network open\"}\n'",
            ),
        )
        .unwrap_err();
    // curl 失败 → 无 Completed → 判失败（不冒充成功）
    assert!(error.to_string().contains("without completion"));
}
