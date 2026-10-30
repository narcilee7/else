//! CAS 快照的核心承诺：恢复后的工作区与快照时逐字节一致。

use else_workspace::{Manifest, Store};

#[test]
fn snapshot_and_restore_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("sub")).unwrap();
    std::fs::write(workspace.join("plan.md"), "原计划").unwrap();
    std::fs::write(workspace.join("sub/data.txt"), "12345").unwrap();

    let store = Store::new(dir.path().join("store")).unwrap();
    let artifact = store
        .snapshot(&workspace, &else_protocol::ids::RunId::new())
        .unwrap();
    let manifest_json = artifact.manifest.unwrap();
    let manifest: Manifest = serde_json::from_str(&manifest_json).unwrap();
    assert_eq!(manifest.files.len(), 2);

    // 快照后破坏工作区，再恢复到新目录：必须与快照时一致
    std::fs::write(workspace.join("plan.md"), "被改坏").unwrap();
    let branch = dir.path().join("branch");
    store.restore(&manifest_json, &branch).unwrap();
    assert_eq!(
        std::fs::read_to_string(branch.join("plan.md")).unwrap(),
        "原计划"
    );
    assert_eq!(
        std::fs::read_to_string(branch.join("sub/data.txt")).unwrap(),
        "12345"
    );
}

#[test]
fn snapshots_exclude_harness_credentials_and_private_state() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let private = workspace.join(".else-harness/codex-home");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(private.join("auth.json"), "secret").unwrap();
    std::fs::write(workspace.join("result.md"), "public artifact").unwrap();

    let store = Store::new(dir.path().join("store")).unwrap();
    let artifact = store
        .snapshot(&workspace, &else_protocol::ids::RunId::new())
        .unwrap();
    let manifest = artifact.manifest.unwrap();
    assert!(manifest.contains("result.md"));
    assert!(!manifest.contains("auth.json"));
    assert!(!manifest.contains(".else-harness"));
}

#[test]
fn identical_content_shares_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("a.txt"), "same").unwrap();
    std::fs::write(workspace.join("b.txt"), "same").unwrap();

    let store = Store::new(dir.path().join("store")).unwrap();
    let artifact = store
        .snapshot(&workspace, &else_protocol::ids::RunId::new())
        .unwrap();
    let manifest: Manifest = serde_json::from_str(&artifact.manifest.unwrap()).unwrap();
    assert_eq!(manifest.files[0].hash, manifest.files[1].hash);

    let blobs = std::fs::read_dir(dir.path().join("store/blobs")).unwrap();
    assert_eq!(blobs.count(), 1, "内容寻址：相同内容只存一份");
}
