use std::sync::Arc;

use tauri::State;

use crate::config::spec;
use crate::error::GitError;
use crate::git::commit::{self, Draft};
use crate::git::log::{self, Commit, CommitPage};
use crate::store::db::{query, Db};
use crate::store::repos;

/// 一页最多这么多条。前端要更多也只给这么多，防止一次 IPC 塞进整个历史。
const MAX_PAGE_SIZE: usize = 500;

/// 分页读提交列表。只接受注册仓库的 id，路径由 Rust 侧解析（§7.1）。
/// browse（treeless 只读浏览）仓库允许读列表——这正是它存在的唯一理由。
#[tauri::command]
pub async fn commit_list(
    state: State<'_, Arc<Db>>,
    id: i64,
    skip: usize,
    limit: usize,
) -> Result<CommitPage, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let limit = limit.clamp(1, MAX_PAGE_SIZE);

    tauri::async_runtime::spawn_blocking(move || log::list(&path, skip, limit))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 用表单内容提交。
///
/// 只接受注册仓库的 id（§7.1），且 browse 仓库在解析路径这一步就被拒（§7.5）。
/// 规范判定在读到配置之后、执行 git 之前完成，前端禁用挡不住的东西这里再挡一次。
#[tauri::command]
pub async fn commit_create(
    state: State<'_, Arc<Db>>,
    id: i64,
    draft: Draft,
) -> Result<Commit, GitError> {
    let path = query(state.inner().clone(), move |conn| {
        repos::ensure_worktree(conn, id)
    })
    .await?;

    tauri::async_runtime::spawn_blocking(move || {
        let spec = spec::load(&path);
        commit::create(&path, &spec, &draft)
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}
