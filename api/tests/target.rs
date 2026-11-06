//! Thread 的执行位置：Scratch 隔离目录 vs 用户指定的真实目录。
//!
//! 两个不变量：
//! 1. 两个 Thread 不共享可写目录（隔离）；
//! 2. `Directory` 模式真的在用户目录里干活，且 else 不会删它。

use else_api::Runtime;
use else_protocol::thread::ThreadMode;

fn directory(path: &std::path::Path) -> ThreadMode {
    ThreadMode::Directory {
        path: path.to_string_lossy().into_owned(),
    }
}

#[test]
fn scratch_threads_get_their_own_directory() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();
    let first = runtime
        .create_thread("第一件事", None, ThreadMode::Scratch)
        .unwrap();
    let second = runtime
        .create_thread("第二件事", None, ThreadMode::Scratch)
        .unwrap();

    let first_dir = runtime.thread_workspace(&first).unwrap();
    let second_dir = runtime.thread_workspace(&second).unwrap();
    assert!(first_dir.is_dir());
    assert!(second_dir.is_dir());
    assert_ne!(first_dir, second_dir);
    // 形状稳定：<runtime>/workspace/threads/<thread>
    assert!(first_dir.starts_with(dir.path().join("workspace/threads")));
    assert_eq!(
        first_dir.file_name().unwrap().to_string_lossy(),
        first.id.to_string()
    );
}

#[test]
fn directory_thread_works_in_that_directory() {
    let runtime_dir = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(runtime_dir.path()).unwrap();
    let spec = runtime
        .create_thread("写一份纪要", None, directory(target.path()))
        .unwrap();

    assert_eq!(
        runtime.thread_workspace(&spec).unwrap(),
        target.path().canonicalize().unwrap()
    );

    // 跑一次：产出落在用户目录里，而不是 Runtime 里。
    runtime.run(&spec.id).unwrap();
    assert!(target.path().join("plan.md").is_file());
    assert!(!runtime_dir
        .path()
        .join("workspace/threads")
        .join(spec.id.to_string())
        .join("plan.md")
        .exists());

    // 删除 Thread 不会删用户的目录。
    runtime.delete_thread(&spec.id).unwrap();
    assert!(target.path().join("plan.md").is_file());
}

#[test]
fn unsupported_targets_are_rejected_at_creation() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path()).unwrap();

    let relative = runtime.create_thread(
        "相对路径没有意义",
        None,
        ThreadMode::Directory {
            path: "relative/place".into(),
        },
    );
    assert!(relative.is_err(), "相对路径必须被拒绝");

    let missing = runtime.create_thread(
        "不存在的目录",
        None,
        ThreadMode::Directory {
            path: dir.path().join("nope").to_string_lossy().into_owned(),
        },
    );
    assert!(missing.is_err(), "不存在的目录必须被拒绝");

    // 校验失败不落任何事件。
    assert!(runtime.list_threads().unwrap().is_empty());
}

#[test]
fn checkpoints_require_a_scratch_thread() {
    let runtime_dir = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(runtime_dir.path()).unwrap();
    let spec = runtime
        .create_thread("目录里的工作", None, directory(target.path()))
        .unwrap();

    let error = runtime.checkpoint(&spec.id).unwrap_err().to_string();
    assert!(
        error.contains("scratch"),
        "应该说明 checkpoint 只对 scratch 成立：{error}"
    );
    // 拒绝不产生任何副作用：没有检查点事件。
    assert!(!runtime
        .events(&spec.id)
        .unwrap()
        .iter()
        .any(|event| matches!(
            event.envelope.payload,
            else_protocol::event::EventPayload::CheckpointCreated { .. }
        )));
}
