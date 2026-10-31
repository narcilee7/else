//! Workspace / Artifact Store：任务产物的通用快照。
//!
//! 分支的不是代码，而是任务状态。快照 = 内容寻址的 manifest +
//! 文件 blob；文件树、文档统一建模，不绑定 Git（Git 可作为另一种后端）。
//!
//! 检查点三要素中的"工件快照"由这里提供；事件水位与在途副作用
//! 结论由 Ledger 与 Gateway 提供。

use std::path::{Path, PathBuf};

use else_protocol::artifact::{Artifact, ArtifactKind};
use else_protocol::ids::{ArtifactId, RunId};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("codec: {0}")]
    Codec(#[from] serde_json::Error),
    #[error("missing blob: {0}")]
    MissingBlob(String),
}

pub type Result<T> = std::result::Result<T, WorkspaceError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// 相对工作区的路径。
    pub path: String,
    pub hash: String,
    pub size: u64,
}

/// 快照清单：路径 → 内容哈希。内容相同的工作区得到相同的 manifest。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub files: Vec<ManifestEntry>,
}

/// 内容寻址存储：`store/blobs/<hash>` + `store/manifests/<hash>.json`。
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("blobs"))?;
        std::fs::create_dir_all(root.join("manifests"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 对工作区拍快照：写入全部 blob 与 manifest，返回 FileTree 工件。
    pub fn snapshot(&self, workspace: &Path, produced_by: &RunId) -> Result<Artifact> {
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(workspace) {
            let entry = entry.map_err(|e| std::io::Error::other(e.to_string()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(workspace)
                .map_err(|e| std::io::Error::other(e.to_string()))?
                .to_string_lossy()
                .to_string();
            if is_private_runtime_path(Path::new(&rel)) {
                continue;
            }
            let content = std::fs::read(path)?;
            let hash = self.write_blob(&content)?;
            files.push(ManifestEntry {
                path: rel,
                hash,
                size: content.len() as u64,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Manifest { files };
        let manifest_json = serde_json::to_string(&manifest)?;
        let manifest_hash = blake3::hash(manifest_json.as_bytes()).to_hex().to_string();
        std::fs::write(
            self.root
                .join("manifests")
                .join(format!("{manifest_hash}.json")),
            &manifest_json,
        )?;
        Ok(Artifact {
            id: ArtifactId::new(),
            kind: ArtifactKind::FileTree,
            locator: manifest_hash,
            manifest: Some(manifest_json),
            produced_by: produced_by.clone(),
            note: "工作区快照".into(),
        })
    }

    /// 把快照恢复到目标目录（分支工作区，COW 隔离）。
    pub fn restore(&self, manifest_json: &str, target: &Path) -> Result<()> {
        let manifest: Manifest = serde_json::from_str(manifest_json)?;
        for file in manifest.files {
            if is_private_runtime_path(Path::new(&file.path)) {
                continue;
            }
            let blob_path = self.root.join("blobs").join(&file.hash);
            if !blob_path.exists() {
                return Err(WorkspaceError::MissingBlob(file.hash));
            }
            let dest = target.join(&file.path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, std::fs::read(&blob_path)?)?;
        }
        Ok(())
    }

    /// 读取快照中的非私有文件，供用户数据导出使用。
    pub fn snapshot_files(&self, manifest_json: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let manifest: Manifest = serde_json::from_str(manifest_json)?;
        let mut files = Vec::with_capacity(manifest.files.len());
        for file in manifest.files {
            if is_private_runtime_path(Path::new(&file.path)) {
                continue;
            }
            let blob_path = self.root.join("blobs").join(&file.hash);
            if !blob_path.exists() {
                return Err(WorkspaceError::MissingBlob(file.hash));
            }
            files.push((file.path, std::fs::read(blob_path)?));
        }
        Ok(files)
    }

    /// 单文件内容哈希（Compare 用）。
    pub fn hash_file(path: &Path) -> Result<String> {
        Ok(blake3::hash(&std::fs::read(path)?).to_hex().to_string())
    }

    fn write_blob(&self, content: &[u8]) -> Result<String> {
        let hash = blake3::hash(content).to_hex().to_string();
        let path = self.root.join("blobs").join(&hash);
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
        Ok(hash)
    }
}

/// 凭据与 Harness 私有状态永远不能成为任务产物，即使旧版本曾把它们
/// 暂存在 Workspace 中。排除是 Snapshot 层的硬边界，不依赖适配器清理。
fn is_private_runtime_path(path: &Path) -> bool {
    path.components().any(|component| {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        matches!(name.to_str(), Some(".else-harness" | ".codex" | ".claude"))
    })
}
