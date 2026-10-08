use std::sync::Arc;

use tauri::State;

use crate::config::spec;
use crate::error::GitError;
use crate::git::commit::{self, Draft};
use crate::git::log::{self, Commit, CommitPage};
use crate::store::db::{query, Db};
use crate::store::repos::{self, RepoKind};

/// 一页最多这么多条。前端要更多也只给这么多，防止一次 IPC 塞进整个历史。
/// 图那一页（commands/graph.rs）用的是同一个数：两边必须同页同宽，行才对得上。
pub(crate) const MAX_PAGE_SIZE: usize = 500;

/// 分页读提交列表。只接受注册仓库的 id，路径由 Rust 侧解析（§7.1）。
/// browse（treeless 只读浏览）仓库允许读列表——这正是它存在的唯一理由。
///
/// `filter` 为空时与筛选前完全同一条路径。带筛选时：git 认识的条件交给 `log`，
/// type 与合规在解析层做；总数带同一组条件跑（需求 7.7）。
#[tauri::command]
pub async fn commit_list(
    state: State<'_, Arc<Db>>,
    id: i64,
    skip: usize,
    limit: usize,
    filter: Option<log::Filter>,
) -> Result<CommitPage, GitError> {
    let (path, kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let limit = limit.clamp(1, MAX_PAGE_SIZE);
    let filter = filter.unwrap_or_default();

    tauri::async_runtime::spawn_blocking(move || {
        // 合规判定要用仓库的规范。browse 仓库没有工作区，配置文件只能从 HEAD 读
        let spec = match kind {
            RepoKind::Worktree => spec::load(&path),
            RepoKind::Browse => spec::load_at_head(&path),
        };
        log::list(&path, skip, limit, &filter, &spec)
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 读一条提交本身。从文件历史、blame 这些地方跳到一条不在当前列表页里的提交时用它。
#[tauri::command]
pub async fn commit_show(
    state: State<'_, Arc<Db>>,
    id: i64,
    sha: String,
) -> Result<Commit, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let sha = super::detail::check_sha(&sha)?;

    tauri::async_runtime::spawn_blocking(move || log::show(&path, &sha))
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
