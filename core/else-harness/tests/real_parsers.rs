//! 真实 Harness 行解析器： fixture 来自本机 codex-cli 0.166.0 与
//! Claude Code 2.1.284 的真实输出（探测捕获），不是手写想象格式。

use else_harness::adapter::{BridgeEvent, LineParser};
use else_harness::real::{ClaudeParser, CodexParser};
use else_protocol::effect::SideEffectKind;
use else_protocol::resource::UsageBasis;

#[test]
fn codex_parser_handles_real_probe_stream() {
    let parser = CodexParser;
    // 非 JSON 行（进度提示）必须跳过
    assert!(parser
        .parse("Reading additional input from stdin...")
        .is_empty());
    assert!(parser
        .parse(r#"{"type":"thread.started","thread_id":"01a18cc8"}"#)
        .is_empty());
    assert!(parser.parse(r#"{"type":"turn.started"}"#).is_empty());

    // 配置告警的 error item 不判失败，只记输出
    let events = parser.parse(
        r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 1 unrecognized configuration setting."}}"#,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], BridgeEvent::Output { text } if text.contains("ignoring")));

    let events = parser.parse(
        r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"ELSE_PROBE_OK"}}"#,
    );
    assert!(matches!(&events[0], BridgeEvent::Output { text } if text == "ELSE_PROBE_OK"));

    let events = parser.parse(
        r#"{"type":"turn.completed","usage":{"input_tokens":11535,"cached_input_tokens":6912,"output_tokens":6}}"#,
    );
    let [BridgeEvent::Usage { model, usage }, BridgeEvent::Completed { .. }] = events.as_slice()
    else {
        panic!("turn.completed 必须产生 Usage + Completed，实际 {events:?}");
    };
    assert_eq!(model, "codex");
    assert_eq!(usage.input_tokens, 11535);
    assert_eq!(usage.output_tokens, 6);
    // Token 实测、费用缺测：混合证据记 Estimated，不记零成本 Reported
    assert_eq!(usage.basis, UsageBasis::Estimated);
}

#[test]
fn codex_parser_maps_tools_and_failures() {
    let parser = CodexParser;
    let events = parser.parse(
        r#"{"type":"item.completed","item":{"type":"command_execution","command":"ls -la","exit_code":0}}"#,
    );
    assert!(matches!(
        &events[0],
        BridgeEvent::ToolObserved { tool, effect: SideEffectKind::Unknown, success: true, .. }
        if tool == "shell"
    ));

    let events = parser.parse(
        r#"{"type":"item.completed","item":{"type":"file_change","changes":[{"path":"src/main.rs","kind":"update"}]}}"#,
    );
    assert!(matches!(
        &events[0],
        BridgeEvent::Artifact { locator, .. } if locator == "src/main.rs"
    ));

    let events =
        parser.parse(r#"{"type":"turn.failed","error":{"message":"stream disconnected"}}"#);
    assert!(matches!(
        &events[0],
        BridgeEvent::Failed { reason } if reason == "stream disconnected"
    ));
}

#[test]
fn claude_parser_handles_real_probe_stream() {
    let parser = ClaudeParser;
    // system init 行不产生事件
    assert!(parser
        .parse(r#"{"type":"system","subtype":"init","cwd":"/private/tmp","session_id":"ac5e0412"}"#)
        .is_empty());

    // api_retry 记为可观察输出
    let events = parser.parse(
        r#"{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"error_status":401,"error":"authentication_failed"}"#,
    );
    assert!(matches!(&events[0], BridgeEvent::Output { text } if text.contains("1/10")));

    // result 行同时产生 Usage（Reported）与终局事件
    let events = parser.parse(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"已完成","total_cost_usd":0.0123,"usage":{"input_tokens":100,"cache_read_input_tokens":50,"cache_creation_input_tokens":10,"output_tokens":42},"modelUsage":{"claude-opus-5-5":{}}}"#,
    );
    assert_eq!(events.len(), 2);
    let BridgeEvent::Usage { model, usage } = &events[0] else {
        panic!("首枚必须是 Usage，实际 {events:?}");
    };
    assert_eq!(model, "claude-opus-5-5");
    assert_eq!(usage.input_tokens, 160);
    assert_eq!(usage.output_tokens, 42);
    assert_eq!(usage.cost_usd_micros, 12_300);
    assert_eq!(usage.basis, UsageBasis::Reported);
    assert!(matches!(&events[1], BridgeEvent::Completed { summary } if summary == "已完成"));

    // is_error 的 result → Failed（真实 401 探测的形态）
    let events = parser.parse(
        r#"{"type":"result","subtype":"success","is_error":true,"result":"Failed to authenticate. API Error: 401","total_cost_usd":0,"usage":{"input_tokens":0,"output_tokens":0},"modelUsage":{}}"#,
    );
    assert!(matches!(
        &events[1],
        BridgeEvent::Failed { reason } if reason.contains("401")
    ));
}

#[test]
fn claude_parser_maps_tool_effects_conservatively() {
    let parser = ClaudeParser;
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"先读文件"},{"type":"tool_use","name":"Read","input":{"file_path":"/ws/a.md"}},{"type":"tool_use","name":"Bash","input":{"command":"rm -rf /"}}]}}"#;
    let events = parser.parse(line);
    assert_eq!(events.len(), 3);
    assert!(matches!(&events[0], BridgeEvent::Output { .. }));
    assert!(matches!(
        &events[1],
        BridgeEvent::ToolObserved { tool, effect: SideEffectKind::ReadOnly, summary, .. }
        if tool == "Read" && summary == "/ws/a.md"
    ));
    // Bash 一律 Unknown 保守记账
    assert!(matches!(
        &events[2],
        BridgeEvent::ToolObserved { tool, effect: SideEffectKind::Unknown, .. }
        if tool == "Bash"
    ));
}
