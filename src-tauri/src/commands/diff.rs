use std::sync::Arc;

use tauri::State;

use super::detail::check_sha;
use crate::error::GitError;
use crate::git::diff;
use crate::store::db::{query, Db};
use crate::store::repos;

/// 工作区里某个文件的未暂存改动（索引 → 工作区）。逐行暂存的界面靠它（§7.8）。
///
/// 只读浏览仓库没有工作区，这条命令对它天然不可用。
#[tauri::command]
pub async fn worktree_file_diff(
    state: State<'_, Arc<Db>>,
    id: i64,
    path: String,
    ignore_white_space: bool,
) -> Result<diff::Diff, GitError> {
    let repo_path = query(state.inner().clone(), move |conn| {
        repos::ensure_worktree(conn, id)
    })
    .await?;

    tauri::async_runtime::spawn_blocking(move || {
        diff::read_worktree(&repo_path, &path, ignore_white_space)
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 提交表单的「AI 生成」要用的一整片暂存区 diff（§AI）。
///
/// 走 `repos::ensure_worktree` 而不是 `locate`：只读浏览仓库没有工作区，本就不该
/// 出现"提交"入口，让它在这里也自然不可用，与 `worktree_file_diff` 同口径。
///
/// 返回的是 git 原始文本（每个文件自带 `diff --git` 段头），交给前端喂给 LLM。
/// 不做任何长度截断：截断策略留给前端——它知道上下文窗口，也能给用户"已截断"提示。
#[tauri::command]
pub async fn commit_diff(state: State<'_, Arc<Db>>, id: i64) -> Result<String, GitError> {
    let repo_path = query(state.inner().clone(), move |conn| {
        repos::ensure_worktree(conn, id)
    })
    .await?;

    tauri::async_runtime::spawn_blocking(move || diff::staged_text(&repo_path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 单个文件的差异（§7.5）。一次点一个文件：整条提交所有文件的 diff 一起回，
/// 大提交第一次点开就要传几十 MB，而人一次只看一个文件。
///
/// 只接受注册仓库的 id（§7.1）。browse（treeless）仓库**允许**读：这一条会按需向远程取
/// blob，那是读不是写，也正是它存在的意义。
#[tauri::command]
pub async fn commit_file_diff(
    state: State<'_, Arc<Db>>,
    id: i64,
    sha: String,
    path: String,
    old_path: Option<String>,
    ignore_white_space: bool,
) -> Result<diff::Diff, GitError> {
    let (repo_path, _kind) =
        query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let sha = check_sha(&sha)?;

    tauri::async_runtime::spawn_blocking(move || {
        diff::read(
            &repo_path,
            &sha,
            &path,
            old_path.as_deref(),
            ignore_white_space,
        )
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}
