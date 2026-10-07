use std::path::Path;
use std::sync::Arc;

use tauri::State;

use crate::error::GitError;
use crate::git::repo::{self, RepoInfo};
use crate::git::status::{self, WorkingFile};
use crate::store::db::{query, Db};
use crate::store::repos::{self, derive_name, Repo, RepoKind};

/// 添加本地仓库：先探测（确认是仓库，并拿 git 认定的工作区顶层路径），再登记。
///
/// 这是唯一接受前端传路径的 git 命令——路径此刻还没进注册表，无从按 id 解析。
/// 它只做读，而且入库的是 `git rev-parse --show-toplevel` 的结果而不是用户输入，
/// 所以在子目录里添加也能落到仓库根。此后所有命令一律走 id（§7.1）。
#[tauri::command]
pub async fn repo_add(state: State<'_, Arc<Db>>, path: String) -> Result<Repo, GitError> {
    let info = tauri::async_runtime::spawn_blocking(move || repo::probe(Path::new(&path)))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))??;

    let canonical = info.work_tree.to_string_lossy().into_owned();
    query(state.inner().clone(), move |conn| {
        repos::add(conn, &canonical, RepoKind::Worktree, None)
    })
    .await
}

#[tauri::command]
pub async fn repo_list(state: State<'_, Arc<Db>>) -> Result<Vec<Repo>, GitError> {
    query(state.inner().clone(), repos::list).await
}

/// 按 id 重新探测，给仓库信息和工作区状态用。路径从注册表解析，前端传不了路径。
pub async fn info_of(db: &Arc<Db>, id: i64) -> Result<(RepoInfo, RepoKind), GitError> {
    let (path, kind) = query(db.clone(), move |conn| repos::locate(conn, id)).await?;
    let info = tauri::async_runtime::spawn_blocking(move || repo::probe(&path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))??;
    Ok((info, kind))
}

#[tauri::command]
pub async fn repo_refresh(state: State<'_, Arc<Db>>, id: i64) -> Result<RepoInfo, GitError> {
    info_of(state.inner(), id).await.map(|(info, _)| info)
}

#[tauri::command]
pub async fn repo_rename(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
) -> Result<Repo, GitError> {
    let name = name.trim().to_string();
    query(state.inner().clone(), move |conn| {
        // 清空名称等于放弃自定义，回落到目录名——比弹一个"名字不能为空"有用
        let final_name = if name.is_empty() {
            let (path, _) = repos::locate(conn, id)?;
            derive_name(&path.to_string_lossy())
        } else {
            name
        };
        repos::rename(conn, id, &final_name)
    })
    .await
}

/// 只移除注册记录，不删磁盘文件（§6.1）。
#[tauri::command]
pub async fn repo_remove(state: State<'_, Arc<Db>>, id: i64) -> Result<(), GitError> {
    query(state.inner().clone(), move |conn| repos::remove(conn, id)).await
}

/// 待提交文件。走 ensure_worktree：只读浏览仓库没有工作区，在 Rust 侧就被拒绝。
#[tauri::command]
pub async fn worktree_status(
    state: State<'_, Arc<Db>>,
    id: i64,
) -> Result<Vec<WorkingFile>, GitError> {
    let path = worktree_path(state.inner(), id).await?;

    tauri::async_runtime::spawn_blocking(move || status::list(&path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 暂存勾选的文件。返回整份最新状态而不是"成功"，界面就不用再补一次读取，
/// 也就不会在两次 IPC 之间显示一个过期的勾选态。
#[tauri::command]
pub async fn files_stage(
    state: State<'_, Arc<Db>>,
    id: i64,
    paths: Vec<String>,
) -> Result<Vec<WorkingFile>, GitError> {
    let path = worktree_path(state.inner(), id).await?;

    tauri::async_runtime::spawn_blocking(move || status::stage(&path, &paths))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

#[tauri::command]
pub async fn files_unstage(
    state: State<'_, Arc<Db>>,
    id: i64,
    paths: Vec<String>,
) -> Result<Vec<WorkingFile>, GitError> {
    let path = worktree_path(state.inner(), id).await?;

    tauri::async_runtime::spawn_blocking(move || status::unstage(&path, &paths))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 写路径的共用闸门：解析 id → 必须是本地工作区仓库（§7.5）。
async fn worktree_path(db: &Arc<Db>, id: i64) -> Result<std::path::PathBuf, GitError> {
    let db = db.clone();
    query(db, move |conn| repos::ensure_worktree(conn, id)).await
}
