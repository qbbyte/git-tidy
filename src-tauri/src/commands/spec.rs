use std::sync::Arc;

use tauri::State;

use crate::config::check::Outcome;
use crate::config::spec::{self, Spec};
use crate::error::GitError;
use crate::git::commit::{self, Draft};
use crate::git::hook::{self, HookInstall, HookStatus};
use crate::git::log;
use crate::store::db::{query, Db};
use crate::store::repos::{self, RepoKind};

/// scope 补全最多扫这么多条历史：再多的话一次 IPC 的耗时就开始影响输入框手感了。
const SCOPE_SCAN_LIMIT: usize = 800;
const SCOPE_SUGGESTION_MAX: usize = 60;

/// 读这个仓库当前生效的规范。表单要显示可选项、报告要按同一份尺子计数，
/// 所以两边都从这里取，不各自解释配置文件。
#[tauri::command]
pub async fn spec_for(state: State<'_, Arc<Db>>, id: i64) -> Result<Spec, GitError> {
    resolve_spec(state.inner(), id).await
}

/// 同一个读取逻辑的库内版本：hook 那几条命令没有 `State`（要连着跑好几步），
/// 走这个而不是把 `State` 到处传。
async fn resolve_spec(db: &Arc<Db>, id: i64) -> Result<Spec, GitError> {
    let (path, kind) = query(db.clone(), move |conn| repos::locate(conn, id)).await?;
    tauri::async_runtime::spawn_blocking(move || match kind {
        RepoKind::Worktree => spec::load(&path),
        // 只读浏览仓库没有工作区，配置文件只能从 HEAD 里读
        RepoKind::Browse => spec::load_at_head(&path),
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))
}

/// 校验一条草稿信息但不提交。这是"实时提示"的入口，也是 hook 与报告共用的那颗内核
/// （需求 6.2/6.6）：三处必须同生同源，否则表单放过、hook 拦下，用户就得到一个解释不了的报错。
/// 入参就是 `commit_create` 的那个 Draft，预览与落库走的是同一次判定。
#[tauri::command]
pub async fn message_check(
    state: State<'_, Arc<Db>>,
    id: i64,
    draft: Draft,
) -> Result<Outcome, GitError> {
    let spec = resolve_spec(state.inner(), id).await?;
    Ok(commit::preview(&spec, &draft))
}

/// 历史里用过的 scope，供表单补全（需求 6.2）。按出现次数排，最常见的在最前面。
#[tauri::command]
pub async fn commit_scopes(state: State<'_, Arc<Db>>, id: i64) -> Result<Vec<String>, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    tauri::async_runtime::spawn_blocking(move || scopes_from_history(&path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// commit-msg hook 的当前状态（需求 7.20）。纯读，进页面就能调。
#[tauri::command]
pub async fn hook_status(state: State<'_, Arc<Db>>, id: i64) -> Result<HookStatus, GitError> {
    let spec = resolve_spec(state.inner(), id).await?;
    let path = worktree_path(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || hook::status(&path, &spec))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 一键安装 / 强制覆盖（`force` 仅对默认 .git/hooks 下别人写的 commit-msg 有意义，
/// 会先备份）。被 `core.hooksPath` 占着时一定不写文件，只把共存方案带回去。
#[tauri::command]
pub async fn hook_install(
    state: State<'_, Arc<Db>>,
    id: i64,
    force: Option<bool>,
) -> Result<HookInstall, GitError> {
    let spec = resolve_spec(state.inner(), id).await?;
    let path = worktree_path(state.inner(), id).await?;
    let force = force.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || hook::install(&path, &spec, force))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 卸载。只删本工具生成的那一份，别人的 hook 不动。
#[tauri::command]
pub async fn hook_uninstall(state: State<'_, Arc<Db>>, id: i64) -> Result<HookInstall, GitError> {
    let spec = resolve_spec(state.inner(), id).await?;
    let path = worktree_path(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || hook::uninstall(&path, &spec))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 将要写入磁盘的那份脚本原文（需求 7.20：规则以字面量快照进脚本，且不依赖本工具）。
/// 界面用它做「查看规则快照」与共存时的复制粘贴内容。
#[tauri::command]
pub async fn hook_script(state: State<'_, Arc<Db>>, id: i64) -> Result<String, GitError> {
    let spec = resolve_spec(state.inner(), id).await?;
    Ok(hook::render(&spec))
}

/// hook 写入的是 .git 下的文件，不是工作区文件，所以不走写操作日志；
/// 但只读浏览仓库（treeless）同样没有 .git/hooks，那边在 Rust 侧就拒。
async fn worktree_path(db: &Arc<Db>, id: i64) -> Result<std::path::PathBuf, GitError> {
    let db = db.clone();
    query(db, move |conn| repos::ensure_worktree(conn, id)).await
}

fn scopes_from_history(repo: &std::path::Path) -> Result<Vec<String>, GitError> {
    let page = log::list(
        repo,
        0,
        SCOPE_SCAN_LIMIT,
        &log::Filter::default(),
        &Spec::default(),
    )?;
    let mut counts: Vec<(String, usize)> = Vec::new();
    for commit in page.commits {
        let Some(scope) = commit.summary.scope else {
            continue;
        };
        match counts.iter_mut().find(|(name, _)| *name == scope) {
            Some(entry) => entry.1 += 1,
            None => counts.push((scope, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts.truncate(SCOPE_SUGGESTION_MAX);
    Ok(counts.into_iter().map(|(scope, _)| scope).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::process;

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    fn repo_with(commits: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        for (i, subject) in commits.iter().enumerate() {
            std::fs::write(dir.path().join("f.txt"), format!("{i}\n")).expect("write");
            git_in(dir.path(), &["add", "f.txt"]);
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
                    subject,
                ],
            );
        }
        dir
    }

    /// scope 补全要的是"这个项目里大家实际怎么写的"，所以按频次排、去重、大小写不合并。
    #[test]
    fn scopes_come_back_most_frequent_first() {
        let dir = repo_with(&[
            "feat(cli): 一",
            "fix(cli): 二",
            "feat(ui): 三",
            "feat: 四",
            "docs(readme): 五",
            "fix(cli): 六",
        ]);
        assert_eq!(
            scopes_from_history(dir.path()).expect("scopes"),
            vec!["cli".to_string(), "readme".to_string(), "ui".to_string()]
        );
    }

    #[test]
    fn an_empty_repo_suggests_nothing_instead_of_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        assert!(scopes_from_history(dir.path())
            .expect("空仓库不该报错")
            .is_empty());
    }
}
