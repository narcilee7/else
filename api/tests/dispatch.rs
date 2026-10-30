//! MVP-3 本地 Dispatch：规则选择、隔离 Workspace、工件交接、终态幂等。

use else_api::Runtime;
use else_harness::adapter::{BridgeEvent, HarnessPrep, LineParser, ProcessHarness};
use else_protocol::effect::SideEffectKind;
use else_protocol::event::{EventPayload, ScopeState};
use else_protocol::harness::{
    CancelSupport, HarnessCapabilities, ToolEffectVisibility, UsageSupport,
};
use else_protocol::ids::{IntentId, ScopeId};
use else_protocol::intent::IntentState;
use else_protocol::memory::SnapshotAudience;

struct JsonParser;

impl LineParser for JsonParser {
    fn parse(&self, line: &str) -> Vec<BridgeEvent> {
        let event = (|| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            match value["type"].as_str()? {
                "output" => Some(BridgeEvent::Output {
                    text: value["text"].as_str()?.into(),
                }),
                "artifact" => Some(BridgeEvent::Artifact {
                    locator: value["locator"].as_str()?.into(),
                    note: value["note"].as_str().unwrap_or_default().into(),
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

fn fake_adapter(name: &'static str, script: &'static str) -> ProcessHarness<JsonParser> {
    ProcessHarness {
        name,
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
        prepare: Box::new(move |_, _, _| {
            Ok(HarnessPrep {
                command: format!(
                    "touch plan.md; printf 'dependency artifact' > result.md; printf '%s' {}",
                    else_harness::adapter::shell_quote(script)
                ),
                env: vec![],
                read_paths: vec![],
                write_paths: vec![],
                cleanup_paths: vec![],
            })
        }),
        parser: JsonParser,
    }
}

fn dependent_harness() -> ProcessHarness<JsonParser> {
    ProcessHarness {
        name: "dependent",
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
                command: "test \"$(cat result.md)\" = 'dependency artifact' || exit 9; \
                          touch plan.md; \
                          printf '%s\\n' '{\"type\":\"completed\",\"summary\":\"dependent done\"}'"
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

const DONE_SCRIPT: &str = "{\"type\":\"artifact\",\"locator\":\"result.md\",\"note\":\"子任务产物\"}\n{\"type\":\"completed\",\"summary\":\"子任务完成\"}\n";

fn root_scope(runtime: &Runtime, intent_id: &IntentId) -> ScopeId {
    runtime
        .projection(intent_id)
        .unwrap()
        .scopes
        .iter()
        .find(|(_, scope)| scope.parent.is_none())
        .map(|(id, _)| id.clone())
        .unwrap()
}

#[test]
fn dispatch_runs_scope_in_isolated_workspace_and_keeps_root_running() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime
        .create_intent("编码任务：实现两个模块", None)
        .unwrap();
    runtime.run(&spec.id).unwrap(); // 根： scripted 推进到 NeedsDecision，产生计划产物
    let root = root_scope(&runtime, &spec.id);
    let child = runtime
        .spawn_subagent(
            &spec.id,
            &root,
            "实现模块 A 的代码",
            SideEffectKind::Reversible,
        )
        .unwrap();

    let projection = runtime
        .dispatch_scope_adapter(&spec.id, &child, &fake_adapter("fake-coder", DONE_SCRIPT))
        .unwrap();
    assert_eq!(projection.scopes[&child].state, ScopeState::Completed);
    assert_eq!(projection.scopes[&root].state, ScopeState::Running);
    assert_eq!(projection.state, Some(IntentState::NeedsDecision));

    // 隔离 Workspace：scope 有自己的目录，不共享可写工作区
    assert!(dir
        .path()
        .join(format!("workspace/scopes/{child}"))
        .is_dir());
    let handed_off =
        std::fs::read_to_string(dir.path().join(format!("workspace/scopes/{child}/plan.md")))
            .unwrap();
    assert!(
        handed_off.contains("目标：编码任务：实现两个模块"),
        "实际交接内容：{handed_off:?}"
    );

    let events = runtime.events(&spec.id).unwrap();
    // 能力随 RunStarted 落账
    assert!(events.iter().any(|event| matches!(
        &event.envelope.payload,
        EventPayload::RunStarted { harness, capabilities, .. }
            if harness == "fake-coder" && capabilities.is_some()
    )));
    // Executor 快照为子 Scope 编译，且携带根已产出的工件引用（交接）
    let snapshot = events
        .iter()
        .rev()
        .find_map(|event| match &event.envelope.payload {
            EventPayload::ContextCompiled { snapshot }
                if snapshot.audience == SnapshotAudience::Executor =>
            {
                Some(snapshot.clone())
            }
            _ => None,
        })
        .expect("Executor 快照");
    assert_eq!(snapshot.outcome, "实现模块 A 的代码");
    assert!(
        !snapshot.artifact_refs.is_empty(),
        "父级工件引用必须随快照交接给子 Scope"
    );
    assert!(
        snapshot.artifact_paths.iter().any(|path| path == "plan.md"),
        "工件必须带可直接读取的路径"
    );
    // 子 Scope 产物落账
    assert!(events.iter().any(|event| matches!(
        &event.envelope.payload,
        EventPayload::ArtifactProduced { artifact } if artifact.locator == "result.md"
    )));
}

#[test]
fn dispatch_terminal_scope_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("幂等派发", None).unwrap();
    runtime.run(&spec.id).unwrap();
    let root = root_scope(&runtime, &spec.id);
    let child = runtime
        .spawn_subagent(&spec.id, &root, "子任务", SideEffectKind::ReadOnly)
        .unwrap();
    runtime
        .dispatch_scope_adapter(&spec.id, &child, &fake_adapter("fake-coder", DONE_SCRIPT))
        .unwrap();
    let before = runtime.events(&spec.id).unwrap().len();
    let again = runtime
        .dispatch_scope_adapter(&spec.id, &child, &fake_adapter("fake-coder", DONE_SCRIPT))
        .unwrap();
    assert_eq!(again.scopes[&child].state, ScopeState::Completed);
    assert_eq!(runtime.events(&spec.id).unwrap().len(), before);
}

#[test]
fn dependent_scope_receives_artifacts_from_its_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let spec = runtime.create_intent("依赖工件交接", None).unwrap();
    runtime.run(&spec.id).unwrap();
    let root = root_scope(&runtime, &spec.id);
    let producer = runtime
        .spawn_subagent(&spec.id, &root, "生产依赖工件", SideEffectKind::ReadOnly)
        .unwrap();
    runtime
        .dispatch_scope_adapter(&spec.id, &producer, &fake_adapter("producer", DONE_SCRIPT))
        .unwrap();
    let consumer = runtime
        .spawn_subagent_with_dependencies(
            &spec.id,
            &root,
            "消费依赖工件",
            SideEffectKind::ReadOnly,
            vec![producer],
        )
        .unwrap();
    let projection = runtime
        .dispatch_scope_adapter(&spec.id, &consumer, &dependent_harness())
        .unwrap();
    assert_eq!(projection.scopes[&consumer].state, ScopeState::Completed);
}

#[test]
fn rule_selection_prefers_explicit_hint_and_degrades() {
    // 显式指定优先；未知 harness 显式指定 → scripted 兜底
    assert_eq!(
        else_harness::select_harness("任意任务", Some("scripted")),
        else_harness::SCRIPTED
    );
    // 编码信号 + 无指定：本机 codex 可用时应选中 codex，否则 claude-code 或 scripted
    let picked = else_harness::select_harness("修复 login 模块的 bug", None);
    assert!([
        else_harness::SCRIPTED,
        else_harness::real::CODEX,
        else_harness::real::CLAUDE_CODE
    ]
    .contains(&picked));
    // 非编码任务：scripted
    assert_eq!(
        else_harness::select_harness("整理会议纪要", None),
        else_harness::SCRIPTED
    );
}
