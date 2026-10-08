use std::sync::Arc;

use tauri::State;

use super::commit::MAX_PAGE_SIZE;
use crate::error::GitError;
use crate::git::{blame, log, tree};
use crate::store::db::{query, Db};
use crate::store::repos;

/// 逐行归属（§7.6）。
///
/// 只接受注册仓库的 id（§7.1）。browse（treeless）仓库**允许**读：blame 只要 commit
/// 与 blob 对象，按需向远程取也还是读。
#[tauri::command]
pub async fn file_blame(
    state: State<'_, Arc<Db>>,
    id: i64,
    rev: String,
    path: String,
    ignore_revs: Option<Vec<String>>,
) -> Result<blame::Blame, GitError> {
    let (repo_path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let rev = check_rev(&rev)?;
    let path = check_path(&path)?;
    // 忽略清单每一条都要进 git 的参数位，所以逐条按提交号判一次
    let ignore_revs = check_shas(ignore_revs.unwrap_or_default())?;

    tauri::async_runtime::spawn_blocking(move || blame::read(&repo_path, &rev, &path, &ignore_revs))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 一个文件的全部改动记录（§7.6）。走 `--follow`，历史跟着改名走。
#[tauri::command]
pub async fn file_history(
    state: State<'_, Arc<Db>>,
    id: i64,
    path: String,
    skip: usize,
    limit: usize,
) -> Result<log::CommitPage, GitError> {
    let (repo_path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let path = check_path(&path)?;
    let limit = limit.clamp(1, MAX_PAGE_SIZE);

    tauri::async_runtime::spawn_blocking(move || log::file_history(&repo_path, &path, skip, limit))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 某个修订的文件树（§7.6）。只读浏览：列的是提交里的内容，不碰工作区。
#[tauri::command]
pub async fn file_tree(
    state: State<'_, Arc<Db>>,
    id: i64,
    rev: String,
) -> Result<Vec<tree::Entry>, GitError> {
    let (repo_path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let rev = check_rev(&rev)?;

    tauri::async_runtime::spawn_blocking(move || tree::list(&repo_path, &rev))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 某个修订里一个文件的正文。子模块指针没有正文，Rust 侧会失败，界面也不给入口。
#[tauri::command]
pub async fn file_content(
    state: State<'_, Arc<Db>>,
    id: i64,
    rev: String,
    path: String,
) -> Result<tree::Content, GitError> {
    let (repo_path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let rev = check_rev(&rev)?;
    let path = check_path(&path)?;

    tauri::async_runtime::spawn_blocking(move || tree::content(&repo_path, &rev, &path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 修订号既可以是提交号，也可以是 `HEAD~2`、`main` 这类 rev。两种形态都要进 git 的
/// 参数位，所以这里只放行"看起来像一个 rev"的串：**绝不能以 `-` 开头**，
/// 否则 git 把它当选项解析（`--upload-pack=...` 这类就能被当命令执行）。
/// 提交号形态仍走 `commands::detail::check_sha` 的十六进制判据。
fn check_rev(rev: &str) -> Result<String, GitError> {
    let trimmed = rev.trim();
    let shaped = !trimmed.is_empty()
        && trimmed.len() <= 255
        && !trimmed.starts_with('-')
        && !trimmed.starts_with('^')
        && trimmed
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace());
    if shaped {
        return Ok(trimmed.to_string());
    }
    Err(GitError::GitFailed {
        stderr: format!("不是合法的修订号：{rev}"),
    })
}

/// 路径要进 `:(literal)<path>` 与 `<rev>:<path>` 两个位置，后者里的 `:` 是分隔符。
/// 判据只有三条：非空、不含控制字符、不含 `..` 与 `:`（否则能拼出 `<rev>:<另一个对象>`）。
/// 路径本身是从我们自己列出的文件树里点出来的，但这是它进 git 的入口。
fn check_path(path: &str) -> Result<String, GitError> {
    let trimmed = path.trim();
    let shaped = !trimmed.is_empty()
        && !trimmed.starts_with('-')
        && trimmed.chars().all(|c| !c.is_control() && c != ':')
        && !trimmed
            .split('/')
            .any(|segment| segment == ".." || segment == ".");
    if shaped {
        return Ok(trimmed.to_string());
    }
    Err(GitError::GitFailed {
        stderr: format!("不是合法的仓库内路径：{path}"),
    })
}

fn check_shas(shas: Vec<String>) -> Result<Vec<String>, GitError> {
    shas
        .iter()
        .map(|sha| super::detail::check_sha(sha))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rev_may_be_a_sha_or_a_walk_expression() {
        assert_eq!(check_rev("HEAD").ok(), Some("HEAD".to_string()));
        assert_eq!(check_rev("main").ok(), Some("main".to_string()));
        assert_eq!(
            check_rev("HEAD~2").ok(),
            Some("HEAD~2".to_string()),
            "带波浪号的表达式是常用形态"
        );
        assert_eq!(
            check_rev(&"a".repeat(40)).ok(),
            Some("a".repeat(40))
        );
        assert_eq!(check_rev("  v1.0  ").ok(), Some("v1.0".to_string()));
    }

    /// 挡的就是这些：带前导横线的 rev 能被 git 当成选项，一不当心就是一个命令执行
    #[test]
    fn a_rev_that_looks_like_an_option_is_refused() {
        for bad in [
            "",
            "   ",
            "--all",
            "--upload-pack=touch ./x",
            "-x",
            "^HEAD",
            "HEAD --not-a-flag",
            "HEAD\nrm -rf",
        ] {
            let err = check_rev(bad).expect_err("该拒绝：{bad}");
            assert!(format!("{err:?}").contains("合法的修订号"), "{err:?}");
        }
    }

    #[test]
    fn a_path_must_stay_inside_the_repository() {
        assert_eq!(check_path("src/main.rs").ok(), Some("src/main.rs".to_string()));
        assert_eq!(
            check_path("中文目录/文件名.md").ok(),
            Some("中文目录/文件名.md".to_string()),
            "中文路径原样放行"
        );
        assert_eq!(
            check_path("有 空格 的文件.txt").ok(),
            Some("有 空格 的文件.txt".to_string()),
            "git 的路径允许含空格，paths 分隔靠的是 `:(literal)` 前缀"
        );
    }

    #[test]
    fn a_path_that_could_escape_or_confuse_git_is_refused() {
        for bad in [
            "",
            "  ",
            "--all",
            "../../etc/passwd",
            "a/./b",
            "HEAD:refs/heads/main",
            "a\u{0}b",
            "a\nb",
        ] {
            let err = check_path(bad).expect_err("该拒绝：{bad}");
            assert!(format!("{err:?}").contains("合法的仓库内路径"), "{err:?}");
        }
    }

    #[test]
    fn ignored_revs_are_validated_one_by_one() {
        let ok = check_shas(vec!["a".repeat(40), "HEAD".to_string()])
            .expect_err("HEAD 不是十六进制，该被拒");
        assert!(format!("{ok:?}").contains("合法的提交号"));
        assert!(check_shas(Vec::new()).expect("空清单当然可以").is_empty());
    }
}