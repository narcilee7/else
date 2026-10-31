//! 跨语言契约 fixture 测试。
//!
//! fixture 由 Apple 客户端测试套件共用（`surfaces/apple/Tests/ElseClientCoreTests/Fixtures/`）。
//! Rust 与 Swift 读同一份 JSON：Rust 侧改动契约形状时，这里会先失败，
//! 而不是等到某个 surface 在运行时才发现解不出来。

use std::path::PathBuf;

use else_protocol::api::IntentSnapshot;

/// fixture 相对 `core/else-protocol` 的位置。
const FIXTURE: &str =
    "../../surfaces/apple/Tests/ElseClientCoreTests/Fixtures/intent_snapshot.json";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE)
}

#[test]
fn intent_snapshot_fixture_round_trips_without_shape_drift() {
    let raw = std::fs::read_to_string(fixture_path()).expect("contract fixture is readable");
    let expected: serde_json::Value = serde_json::from_str(&raw).expect("fixture is valid JSON");

    let snapshot: IntentSnapshot =
        serde_json::from_value(expected.clone()).expect("fixture decodes into IntentSnapshot");
    let actual = serde_json::to_value(&snapshot).expect("IntentSnapshot serializes");

    assert_is_subset(
        &expected,
        &actual,
        "IntentSnapshot 的线上形状与共享 fixture 不一致；\
         契约只允许加字段，不允许改名或改语义",
    );
}

/// 断言 `expected` 是 `actual` 的子集：fixture 里的每个字段都必须在线上形状中
/// 存在且取值一致。
///
/// 线上形状比 fixture 多出来的字段是允许的 —— 那正是契约**只加不减**的表现
/// （`#[serde(default)]` 字段在序列化时被物化，新字段被加入）。
/// 反过来，fixture 里的字段消失或改名，就是破坏性变更，必须失败。
fn assert_is_subset(expected: &serde_json::Value, actual: &serde_json::Value, context: &str) {
    match (expected, actual) {
        (serde_json::Value::Object(expected), serde_json::Value::Object(actual)) => {
            for (key, value) in expected {
                let actual = actual.get(key).unwrap_or_else(|| {
                    panic!("{context}: 线上形状缺少字段 `{key}`（契约只允许加字段）")
                });
                assert_is_subset(value, actual, &format!("{context}.{key}"));
            }
        }
        (serde_json::Value::Array(expected), serde_json::Value::Array(actual)) => {
            assert_eq!(
                expected.len(),
                actual.len(),
                "{context}: 数组长度不一致（契约只允许加字段，不允许改数组语义）"
            );
            for (index, value) in expected.iter().enumerate() {
                assert_is_subset(value, &actual[index], &format!("{context}[{index}]"));
            }
        }
        (expected, actual) => assert_eq!(
            expected, actual,
            "{context}: 取值不一致，契约不允许改字段语义"
        ),
    }
}

#[test]
fn intent_snapshot_fixture_keeps_its_meaning() {
    let raw = std::fs::read_to_string(fixture_path()).expect("contract fixture is readable");
    let snapshot: IntentSnapshot =
        serde_json::from_str(&raw).expect("fixture decodes into IntentSnapshot");

    assert_eq!(snapshot.spec.revision, 2);
    assert_eq!(snapshot.spec.outcome, "完成客户端基础设施");
    assert_eq!(snapshot.spec.done_when.len(), 1);
    assert_eq!(snapshot.spec.done_when[0].description, "预算内完成");
    assert_eq!(snapshot.spec.project.as_deref(), Some("else"));
    assert_eq!(snapshot.spec.authority.forbidden, vec!["不对外发送"]);
    assert_eq!(snapshot.spec.attention_policy.batch_interval_minutes, 30);
    assert!(!snapshot.spec.opaque_harness_policy.enabled);

    assert_eq!(snapshot.decisions.len(), 1);
    assert_eq!(snapshot.decisions[0].summary, "允许执行？");
    assert_eq!(
        snapshot.decisions[0].default_option.as_deref(),
        Some("approve")
    );
    assert_eq!(
        snapshot.decisions[0].related_call_key.as_deref(),
        Some("shell:42")
    );

    assert_eq!(snapshot.resource_budget.max_model_tokens, 1_000_000);
    assert_eq!(snapshot.resource_usage.tool_calls, 4);
    assert_eq!(snapshot.cursor, 17);
    assert!(snapshot.operation_id.is_none());
}
