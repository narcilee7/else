//! 真实后端验收：这些测试不是检查字符串，而是实际启动隔离进程。

use else_environment::{Sandbox, SandboxProfile};

fn native_sandbox(workspace: &std::path::Path) -> Option<Sandbox> {
    match Sandbox::new(SandboxProfile::new(workspace)) {
        Ok(sandbox) => Some(sandbox),
        Err(else_environment::SandboxError::BackendUnavailable { .. }) => None,
        Err(error) => panic!("failed to initialize native sandbox: {error}"),
    }
}

#[test]
fn workspace_write_succeeds_but_outside_file_read_is_denied() {
    let dir = tempfile::tempdir().unwrap();
    let Some(sandbox) = native_sandbox(dir.path()) else {
        return;
    };
    let output = sandbox
        .run_shell(
            "set -e; \
             printf isolated > artifact.txt; \
             test \"$(cat artifact.txt)\" = isolated; \
             if cat /etc/passwd >/dev/null 2>&1; then \
                 echo outside-file-readable; exit 41; \
             fi; \
             echo filesystem-isolated",
        )
        .unwrap();
    assert!(
        output.status.success(),
        "status={:?}, stdout={}, stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("filesystem-isolated"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("artifact.txt")).unwrap(),
        "isolated"
    );
}

#[test]
fn outbound_network_is_denied() {
    if !std::path::Path::new("/usr/bin/curl").is_file()
        && !std::path::Path::new("/bin/curl").is_file()
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let Some(sandbox) = native_sandbox(dir.path()) else {
        return;
    };
    let output = sandbox
        .run_shell(
            "if curl -I --max-time 2 https://example.com >/dev/null 2>&1; then \
                 echo network-open; exit 42; \
             fi; \
             echo network-isolated",
        )
        .unwrap();
    assert!(
        output.status.success(),
        "status={:?}, stdout={}, stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("network-isolated"));
}
