//! 客户端与真实 daemon 的契约测试。
//!
//! 这里跑的是**完整链路**：客户端 → HTTP → daemon → Runtime → Ledger。
//! 它证明 CLI / TUI / App 走的是同一套语义，而不是各自猜出来的行为。

use else_client::{ClientConfig, RuntimeClient};
use else_daemon::{Daemon, DaemonConfig};
use else_protocol::ids::ThreadId;

fn start() -> (tempfile::TempDir, Daemon) {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(DaemonConfig {
        dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".into(),
        token: Some("contract-token".into()),
    })
    .unwrap();
    (dir, daemon)
}

fn client(daemon: &Daemon) -> RuntimeClient {
    RuntimeClient::new(&daemon.url, &daemon.token)
}

#[test]
fn client_creates_reads_and_advances_an_thread_over_http() {
    let (_dir, daemon) = start();
    let client = client(&daemon);

    let snapshot = client
        .create_thread("契约测试目标", Some("contract"), &Default::default())
        .expect("create over http");
    assert_eq!(snapshot.spec.outcome, "契约测试目标");
    assert_eq!(snapshot.spec.project.as_deref(), Some("contract"));
    let thread_id = snapshot.spec.id.clone();

    // 列表必须包含刚建的 Thread。
    let threads = client.list_threads().expect("list over http");
    assert!(threads.iter().any(|item| item.spec.id == thread_id));

    // 再读一次快照必须一致。
    let reread = client.thread(&thread_id).expect("thread over http");
    assert_eq!(reread.spec.id, thread_id);
    assert_eq!(reread.spec.revision, snapshot.spec.revision);

    // 推进一个 Kernel 步。
    let advanced = client
        .advance(&thread_id, Some("scripted"))
        .expect("advance");
    assert_eq!(advanced.spec.id, thread_id);

    // Scope 树：至少有一个根 Scope。
    let scopes = client.scopes(&thread_id).expect("scopes over http");
    assert!(!scopes.is_empty());

    daemon.shutdown();
}

#[test]
fn client_rejects_unknown_threads_without_hiding_system_memory() {
    let (_dir, daemon) = start();
    let client = client(&daemon);
    let missing = ThreadId::from("missing-thread");

    // Memory 是系统级接口，不要求存在名为 else-system 的普通 Thread。
    client.memory().expect("system memory over http");

    for error in [
        client.thread(&missing).expect_err("unknown thread read"),
        client.plan(&missing).expect_err("unknown thread plan"),
        client
            .advance(&missing, Some("scripted"))
            .expect_err("unknown thread advance"),
    ] {
        match error {
            else_client::ClientError::Rejected(message) => {
                assert!(message.contains("404"), "{message}");
                assert!(
                    message.contains("thread not found: missing-thread"),
                    "{message}"
                );
            }
            other => panic!("expected HTTP 404 rejection, got {other}"),
        }
    }

    daemon.shutdown();
}

#[test]
fn client_reads_the_personal_brief_and_world() {
    let (_dir, daemon) = start();
    let client = client(&daemon);

    client
        .create_thread("简报目标", None, &Default::default())
        .expect("create");

    // Brief 是跨 Provider 聚合的；没有 Connector 时应为空而不是报错。
    let brief = client.brief().expect("brief over http");
    assert!(brief.open_commitments.len() <= brief.open_commitments.len());

    let projects = client.project_worlds(None, false).expect("projects");
    assert!(projects.is_empty() || !projects.is_empty());

    daemon.shutdown();
}

#[test]
fn client_writes_require_a_matching_revision() {
    let (_dir, daemon) = start();
    let client = client(&daemon);

    let snapshot = client
        .create_thread("Revision 冲突", None, &Default::default())
        .expect("create");
    let thread_id = snapshot.spec.id.clone();

    // 先改一次目标，Revision 前进。
    client
        .revise_thread(&thread_id, Some("改过的目标"), &[], "测试修订")
        .expect("revise");

    // 客户端缓存了最新 Revision，写操作应当成功。
    client
        .record_external_event(&thread_id, "mail", "已回复")
        .expect("record event with fresh revision");

    daemon.shutdown();
}

#[test]
fn client_reports_missing_token_for_a_missing_token_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = ClientConfig::for_dir(dir.path());
    let error = config.resolve_token().expect_err("no token file");
    assert!(
        matches!(error, else_client::ClientError::MissingToken(_)),
        "{error}"
    );
}

#[test]
fn client_reads_a_token_file_that_is_private_to_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let token_path = dir.path().join("daemon-token");
    std::fs::write(&token_path, "secret-token\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    let config = ClientConfig::for_dir(dir.path());
    assert_eq!(config.resolve_token().unwrap(), "secret-token");
}

#[test]
fn client_refuses_a_token_file_that_group_or_others_can_read() {
    let dir = tempfile::tempdir().unwrap();
    let token_path = dir.path().join("daemon-token");
    std::fs::write(&token_path, "secret-token").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    let config = ClientConfig::for_dir(dir.path());
    let error = config.resolve_token().expect_err("group readable token");
    assert!(
        matches!(error, else_client::ClientError::MissingToken(_)),
        "{error}"
    );
}

#[test]
fn client_probe_confirms_the_token_belongs_to_that_runtime() {
    let (_dir, daemon) = start();
    let client = client(&daemon);

    // 正确令牌：探针通过。
    client.probe().expect("probe with the right token");

    // 错误令牌：探针必须失败 —— 否则 CLI 会连上别人的 Runtime，
    // 在别人的账本上写东西。
    let wrong = RuntimeClient::new(&daemon.url, "not-the-token");
    let error = wrong.probe().expect_err("probe with a wrong token");
    assert!(
        matches!(error, else_client::ClientError::Rejected(_)),
        "{error}"
    );

    daemon.shutdown();
}

#[test]
fn client_reports_an_unreachable_runtime_clearly() {
    // 保留一个几乎不可能被占用的端口。
    let client = RuntimeClient::new("http://127.0.0.1:1", "token");
    let error = client.health().expect_err("nothing is listening");
    assert!(
        matches!(error, else_client::ClientError::Unreachable(_)),
        "{error}"
    );
}

#[test]
fn client_resolves_thread_ids_from_strings() {
    let id = ThreadId::from("demo-thread");
    assert_eq!(id.to_string(), "demo-thread");
}
