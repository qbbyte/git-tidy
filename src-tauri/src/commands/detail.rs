use std::sync::Arc;

use tauri::State;

use crate::error::GitError;
use crate::git::detail;
use crate::store::db::{query, Db};
use crate::store::repos;

/// 一条提交的改动清单 + 每个文件的行数差（§7.4）。
///
/// 只接受注册仓库的 id（§7.1）；browse（treeless）仓库允许读——`--raw` 只要 commit 和
/// tree 对象，工作区里有没有文件内容跟它无关。
#[tauri::command]
pub async fn commit_detail(
    state: State<'_, Arc<Db>>,
    id: i64,
    sha: String,
) -> Result<detail::Detail, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;
    let sha = check_sha(&sha)?;

    tauri::async_runtime::spawn_blocking(move || detail::read(&path, &sha))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 提交号要拼进 git 的参数位，所以先确认它只可能是十六进制。
/// 值本身是从我们自己的列表里来的，但这一层是它进 Rust 的唯一入口，带 `-` 的串会被
/// git 当成选项解析，这一步便宜到没有理由省。
///
/// diff 那一层（commands/diff.rs）共用这一个判据。
pub(crate) fn check_sha(sha: &str) -> Result<String, GitError> {
    let trimmed = sha.trim();
    // 40 位是 sha1，64 位留给 sha256 仓库（git 2.47 起可选）；缩写形态也认，git 自己解
    let shaped = !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed.chars().all(|c| c.is_ascii_hexdigit());
    if shaped {
        return Ok(trimmed.to_string());
    }
    Err(GitError::GitFailed {
        stderr: format!("不是合法的提交号：{sha}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_or_abbreviated_sha_is_accepted() {
        assert_eq!(check_sha(&"a".repeat(40)).ok(), Some("a".repeat(40)));
        assert_eq!(check_sha("  1F2e3  ").ok(), Some("1F2e3".to_string()));
    }

    /// 挡的就是这类值：它们会带着前导横线走到 git 那儿
    #[test]
    fn anything_that_is_not_hex_is_refused() {
        for bad in [
            "",
            "   ",
            "--all",
            "HEAD",
            "0123456789abcdef-",
            "a b",
            &"a".repeat(65),
        ] {
            let err = check_sha(bad).expect_err("该拒绝：{bad}");
            assert!(format!("{err:?}").contains("合法的提交号"), "{err:?}");
        }
    }
}
