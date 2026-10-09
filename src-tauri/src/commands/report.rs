use std::sync::Arc;

use tauri::State;

use crate::config::spec;
use crate::error::GitError;
use crate::git::compliance::{self, Range, Report};
use crate::git::process;
use crate::store::db::{query, Db};
use crate::store::repos::{self, RepoKind};

/// 符合率报告（需求 7.21）。
///
/// 与表单、hook 同源：判定内核只有 `config::check::evaluate` 一处，本命令不重写规则。
/// 区间与历史列表页共用 `git log` 的口径（同一组筛选、同一套字段），
/// 所以"报告里的条数"与"历史里筛出来的条数"不会各说各话。
#[tauri::command]
pub async fn compliance_report(
    state: State<'_, Arc<Db>>,
    id: i64,
    rev: Option<String>,
    author: Option<String>,
) -> Result<Report, GitError> {
    let (path, kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let range = Range { rev, author };
    // 只读浏览仓库也要能看报告：它有完整历史，只是没有工作区
    let loaded = tauri::async_runtime::spawn_blocking(move || {
        let loaded = match kind {
            RepoKind::Worktree => spec::load(&path),
            RepoKind::Browse => spec::load_at_head(&path),
        };
        compliance::build(&path, &loaded, &range)
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))??;
    Ok(loaded)
}

/// 可选区间：HEAD、tag、`a..b`。界面把它当输入提示给用户，而不是让人手填 rev 语法。
#[tauri::command]
pub async fn compliance_revisions(
    state: State<'_, Arc<Db>>,
    id: i64,
) -> Result<Vec<String>, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    tauri::async_runtime::spawn_blocking(move || revisions(&path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

fn revisions(repo: &std::path::Path) -> Result<Vec<String>, GitError> {
    let out = process::run(
        Some(repo),
        &["tag", "--sort=-creatordate", "--format=%(refname:short)"],
    )?;
    // 没有 tag 不是错误，只是一个候选都没有
    let tags: Vec<String> = if out.success {
        out.stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(30)
            .map(|tag| format!("{tag}..HEAD"))
            .collect()
    } else {
        Vec::new()
    };
    let mut list = vec!["HEAD".to_string(), "HEAD~100".to_string()];
    list.extend(tags);
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    #[test]
    fn revision_candidates_start_from_head_and_list_recent_tags() {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        std::fs::write(dir.path().join("a.txt"), "x\n").expect("write");
        git_in(dir.path(), &["add", "a.txt"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "feat: 一",
            ],
        );
        git_in(dir.path(), &["tag", "v0.1.0"]);

        let list = revisions(dir.path()).expect("revisions");
        assert_eq!(list[0], "HEAD");
        assert!(
            list.iter().any(|item| item == "v0.1.0..HEAD"),
            "tag 要能直接选：{list:?}"
        );
    }
}
