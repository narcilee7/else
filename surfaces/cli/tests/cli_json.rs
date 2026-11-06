use std::path::Path;
use std::process::{Command, Output};

fn run(runtime: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_else-bot"))
        .arg("--dir")
        .arg(runtime)
        .args(arguments)
        .output()
        .expect("run else-bot")
}

fn stdout_json(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be one JSON document")
}

#[test]
fn write_read_and_brief_commands_emit_json() {
    let runtime = tempfile::tempdir().expect("temporary runtime");
    let created = run(
        runtime.path(),
        &[
            "thread",
            "Verify JSON automation",
            "--harness",
            "scripted",
            "--json",
        ],
    );
    let snapshot = stdout_json(&created);
    assert_eq!(snapshot["spec"]["outcome"], "Verify JSON automation");
    let thread_id = snapshot["spec"]["id"].as_str().expect("thread id");

    let decisions = run(runtime.path(), &["decisions", thread_id, "--json"]);
    let decisions = stdout_json(&decisions);
    assert!(decisions["windows"].is_array());
    assert!(decisions["requests"].is_array());

    let brief = run(runtime.path(), &["--json"]);
    assert!(stdout_json(&brief)["generated_at"].is_string());
}

#[test]
fn runtime_and_argument_errors_are_json_on_stderr() {
    let runtime = tempfile::tempdir().expect("temporary runtime");
    let failure = run(runtime.path(), &["plan", "missing-thread", "--json"]);
    assert!(!failure.status.success());
    assert!(failure.stdout.is_empty());
    let error: serde_json::Value =
        serde_json::from_slice(&failure.stderr).expect("runtime error must be JSON");
    assert_eq!(error["error"]["kind"], "client");
    let message = error["error"]["message"].as_str().expect("error message");
    assert!(message.contains("404"), "{message}");
    assert!(message.contains("missing-thread"), "{message}");

    let failure = run(runtime.path(), &["decisions", "--json"]);
    assert_eq!(failure.status.code(), Some(2));
    assert!(failure.stdout.is_empty());
    let error: serde_json::Value =
        serde_json::from_slice(&failure.stderr).expect("argument error must be JSON");
    assert_eq!(error["error"]["kind"], "usage");
}

#[test]
fn memory_and_empty_collection_commands_return_json_values() {
    let runtime = tempfile::tempdir().expect("temporary runtime");
    let memory = run(runtime.path(), &["memory", "list", "--json"]);
    let memory = stdout_json(&memory);
    assert!(memory["promoted"].is_array());
    assert!(memory["candidates"].is_array());

    for arguments in [
        &["signal-feed", "--json"][..],
        &["candidate-feed", "--json"][..],
        &["connector-adapters", "--json"][..],
    ] {
        let output = run(runtime.path(), arguments);
        assert!(stdout_json(&output).is_array());
    }
}
