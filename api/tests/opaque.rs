//! Opaque Harness Profile：默认拒绝、显式授权、观察事件与授权事件分离。

use else_api::{ApiError, Runtime};
use else_gateway::GatewayError;
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::EventPayload;
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::intent::OpaqueHarnessPolicy;

struct JsonParser;

impl LineParser for JsonParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent> {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return vec![];
        };
        match value["type"].as_str() {
            Some("tool") => vec![BridgeEvent::ToolObserved {
                tool: value["tool"].as_str().unwrap_or("unknown").into(),
                effect: serde_json::from_value(value["effect"].clone())
                    .unwrap_or(SideEffectKind::Unknown),
                summary: value["summary"].as_str().unwrap_or_default().into(),
                success: true,
            }],
            Some("completed") => vec![BridgeEvent::Completed {
                summary: value["summary"].as_str().unwrap_or_default().into(),
            }],
            _ => vec![],
        }
    }
}

fn opaque_harness(network: else_environment::NetworkPolicy) -> ProcessHarness<JsonParser> {
    ProcessHarness {
        name: "opaque-probe",
        capabilities: HarnessCapabilities {
            cancel: CancelSupport::KillOnly,
            checkpoint_resume: false,
            usage: UsageSupport::None,
            tool_effects: ToolEffectVisibility::Opaque,
            interactive_input: false,
        },
        network,
        egress_allowlist: &["api.openai.com"],
        timeout: None,
        prepare: Box::new(|_, _, _| {
            Ok(HarnessPrep {
                command: "touch plan.md; \
                          printf '%s\\n' '{\"type\":\"tool\",\"tool\":\"Write\",\"effect\":\"reversible\",\"summary\":\"write\"}'; \
                          printf '%s\\n' '{\"type\":\"completed\",\"summary\":\"done\"}'"
                    .into(),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: JsonParser,
    }
}

#[cfg(target_os = "macos")]
fn network_probe_harness() -> ProcessHarness<JsonParser> {
    let mut harness = opaque_harness(else_environment::NetworkPolicy::AllowOutbound);
    harness.name = "opaque-network-probe";
    harness.prepare = Box::new(|_, _, _| {
        Ok(HarnessPrep {
            command: "touch plan.md; \
                      hp=${HTTPS_PROXY#*//}; port=${hp##*:}; \
                      printf 'CONNECT example.com:443 HTTP/1.1\\r\\nHost: example.com\\r\\n\\r\\n' | nc 127.0.0.1 $port > proxy.txt 2>&1 || true; \
                      printf '%s\\n' '{\"type\":\"completed\",\"summary\":\"probe\"}'"
                .into(),
            env: vec![],
            read_paths: vec![],
            write_paths: vec![],
            cleanup_paths: vec![],
        })
    });
    harness
}

#[cfg(target_os = "macos")]
fn tool_gateway_harness() -> ProcessHarness<JsonParser> {
    let mut harness = opaque_harness(else_environment::NetworkPolicy::Deny);
    harness.name = "opaque-tool-gateway";
    harness.prepare = Box::new(|_, _, _| {
        Ok(HarnessPrep {
            command: r#"printf direct > direct.txt 2>/dev/null || true
url=${ELSE_TOOL_URL#*//}
body='{"tool":"fs.write","input":{"path":"plan.md","content":"gateway-write"},"idempotency_key":"opaque-plan"}'
length=${#body}
call_tool() {
  printf 'POST /tool HTTP/1.1\r\nHost: %s\r\nAuthorization: Bearer %s\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s' "$url" "$ELSE_TOOL_TOKEN" "$length" "$body" | nc "${url%%:*}" "${url##*:}" | grep -q '"ok":true'
}
call_tool || exit 12
call_tool || exit 13
printf '%s\n' '{"type":"completed","summary":"gateway done"}'"#
                .into(),
            env: vec![],
            read_paths: vec![],
            write_paths: vec![],
            cleanup_paths: vec![],
        })
    });
    harness
}

fn policy(network: bool, max_observed_effect: SideEffectKind) -> OpaqueHarnessPolicy {
    OpaqueHarnessPolicy {
        enabled: true,
        allow_native_provider_network: network,
        use_tool_gateway: false,
        max_observed_effect,
    }
}

#[test]
fn opaque_harness_is_denied_before_any_run_event_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("Opaque 默认拒绝", None).unwrap();
    let before = runtime.events(&spec.id).unwrap().len();

    let error = runtime
        .run_adapter(
            &spec.id,
            &opaque_harness(else_environment::NetworkPolicy::Deny),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ApiError::Gateway(GatewayError::OpaqueHarnessDenied(_))
    ));
    assert_eq!(runtime.events(&spec.id).unwrap().len(), before);
}

#[test]
fn opaque_observation_is_not_recorded_as_authorized_gateway_call() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("Opaque 观察记账", None).unwrap();
    runtime
        .configure_opaque_harness(&spec.id, policy(false, SideEffectKind::Reversible))
        .unwrap();

    runtime
        .run_adapter(
            &spec.id,
            &opaque_harness(else_environment::NetworkPolicy::Deny),
        )
        .unwrap();
    let events = runtime.events(&spec.id).unwrap();
    assert!(events.iter().any(|event| matches!(
        event.envelope.payload,
        EventPayload::OpaqueToolObserved { .. }
    )));
    assert!(!events
        .iter()
        .any(|event| matches!(event.envelope.payload, EventPayload::ToolCalled { .. })));
    let report = runtime.resources(&spec.id).unwrap();
    assert_eq!(report.usage.tool_calls, 1);
    assert_eq!(report.usage.opaque_tool_calls, 1);
}

#[test]
fn observed_effect_above_profile_aborts_the_harness() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("Opaque 副作用上限", None).unwrap();
    runtime
        .configure_opaque_harness(&spec.id, policy(false, SideEffectKind::ReadOnly))
        .unwrap();

    let error = runtime
        .run_adapter(
            &spec.id,
            &opaque_harness(else_environment::NetworkPolicy::Deny),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ApiError::Gateway(GatewayError::OpaqueEffectDenied(_))
    ));
}

#[test]
#[cfg(target_os = "macos")]
fn native_provider_network_requires_a_separate_explicit_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("Opaque 网络授权", None).unwrap();
    runtime
        .configure_opaque_harness(&spec.id, policy(false, SideEffectKind::Reversible))
        .unwrap();
    let error = runtime
        .run_adapter(
            &spec.id,
            &opaque_harness(else_environment::NetworkPolicy::AllowOutbound),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ApiError::Gateway(GatewayError::OpaqueHarnessDenied(_))
    ));

    runtime
        .configure_opaque_harness(&spec.id, policy(true, SideEffectKind::Reversible))
        .unwrap();
    runtime
        .run_adapter(
            &spec.id,
            &opaque_harness(else_environment::NetworkPolicy::AllowOutbound),
        )
        .unwrap();
}

#[test]
#[cfg(target_os = "macos")]
fn egress_proxy_denies_unlisted_host_and_records_decision() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_intent("Opaque 域名 Allowlist", None)
        .unwrap();
    runtime
        .configure_opaque_harness(&spec.id, policy(true, SideEffectKind::Reversible))
        .unwrap();

    runtime
        .run_adapter(&spec.id, &network_probe_harness())
        .unwrap();
    let proxy_response =
        std::fs::read_to_string(dir.path().join("workspace/proxy.txt")).unwrap_or_default();
    assert!(proxy_response.contains("403 Forbidden"));
    let events = runtime.events(&spec.id).unwrap();
    let decision = events
        .iter()
        .find_map(|event| match &event.envelope.payload {
            EventPayload::EgressIntercepted { decision } => Some(decision.clone()),
            _ => None,
        })
        .expect("egress decision");
    assert_eq!(decision.host, "example.com");
    assert!(!decision.allowed);
}

#[test]
#[cfg(target_os = "macos")]
fn tool_gateway_forces_workspace_writes_and_records_authorized_events() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_intent("Tool Gateway 强制写入", None)
        .unwrap();
    runtime
        .configure_opaque_harness(
            &spec.id,
            OpaqueHarnessPolicy {
                enabled: true,
                allow_native_provider_network: false,
                use_tool_gateway: true,
                max_observed_effect: SideEffectKind::Reversible,
            },
        )
        .unwrap();

    runtime
        .run_adapter(&spec.id, &tool_gateway_harness())
        .unwrap();
    let workspace = dir.path().join("workspace");
    assert!(!workspace.join("direct.txt").exists());
    assert_eq!(
        std::fs::read_to_string(workspace.join("plan.md")).unwrap(),
        "gateway-write"
    );
    let events = runtime.events(&spec.id).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.envelope.payload, EventPayload::ToolCalled { .. }))
            .count(),
        1
    );
    assert!(!events.iter().any(|event| matches!(
        event.envelope.payload,
        EventPayload::OpaqueToolObserved { .. }
    )));
}
