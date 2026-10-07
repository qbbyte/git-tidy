use crate::error::GitError;
use crate::git::repo::{self, RepoInfo};

/// 探测目录是否为 Git 仓库。
/// git 调用是阻塞子进程，必须放 spawn_blocking，否则占满异步运行时会让界面假死。
#[tauri::command]
pub async fn repo_probe(path: String) -> Result<RepoInfo, GitError> {
    tauri::async_runtime::spawn_blocking(move || repo::probe(&path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}
