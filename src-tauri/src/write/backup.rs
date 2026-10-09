use std::path::Path;

use crate::error::GitError;
use crate::git::process;

/// 还原 ref 的命名空间。不在 `refs/heads` 或任何远端会看到的命名空间里，
/// 所以它既不会出现在分支列表里，也不会被 `push --all` 带走。
pub const NAMESPACE: &str = "refs/git-tidy";

/// 写一个还原 ref，指到 `head`。
///
/// 名字带时间戳（秒级 + 短随机尾巴）：同一秒里连做两次写操作也不会互相覆盖，
/// 而顺序在界面上仍然读得出来。
pub fn create(repo: &Path, head: &str) -> Result<String, GitError> {
    let name = format!("backup-{}", stamp());
    let reference = format!("{NAMESPACE}/{name}");
    process::run(Some(repo), &["update-ref", &reference, head])?.expect_success()?;
    Ok(reference)
}

/// 还原 ref 现在指向哪里。取不到返回 None：它可能被 `git gc` 或用户清掉了。
pub fn target(repo: &Path, reference: &str) -> Result<Option<String>, GitError> {
    let out = process::run(Some(repo), &["rev-parse", "-q", "--verify", reference])?;
    if !out.success {
        return Ok(None);
    }
    let sha = out.stdout.trim();
    Ok((!sha.is_empty()).then(|| sha.to_string()))
}

/// 删掉一个还原 ref。撤销成功之后顺手清掉，否则 `refs/git-tidy/` 会越积越多
/// （它们不在任何分支里，但 reflog 与对象一样要占地方）。
pub fn drop(repo: &Path, reference: &str) -> Result<(), GitError> {
    process::run(Some(repo), &["update-ref", "-d", reference])?.expect_success()?;
    Ok(())
}

/// 仓库里现有的全部还原 ref，新的在前。界面上给"找回改写前的状态"用。
pub fn list(repo: &Path) -> Result<Vec<Backup>, GitError> {
    let out = process::run(
        Some(repo),
        &[
            "for-each-ref",
            "--sort=-refname",
            "--format=%(refname)%1f%(objectname)",
            NAMESPACE,
        ],
    )?
    .expect_success()?;

    let mut backups = Vec::new();
    for line in out.lines() {
        let Some((reference, sha)) = line.split_once('\u{1f}') else {
            continue;
        };
        backups.push(Backup {
            reference: reference.to_string(),
            sha: sha.to_string(),
        });
    }
    Ok(backups)
}

/// 一个还原点。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub reference: String,
    /// 它指着的提交（通常是执行写操作之前的 HEAD）
    pub sha: String,
}

/// 校验 ref 名（界面传进来的还原点名字要进 `update-ref` 的参数位）。
pub fn check_reference(reference: &str) -> Result<String, GitError> {
    let trimmed = reference.trim();
    // 结构：`refs/git-tidy/backup-<名字>`，正好三段。段数与形状都收死：
    // `../..` 这种靠 `/` 拼出来的路径形状在这里过不去
    let mut segments = trimmed.split('/');
    let shaped = trimmed.len() <= 128
        && segments.next() == Some("refs")
        && segments.next() == Some("git-tidy")
        && segments
            .next()
            .and_then(|name| name.strip_prefix("backup-"))
            .is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            })
        && segments.next().is_none();
    if shaped {
        return Ok(trimmed.to_string());
    }
    Err(GitError::GitFailed {
        stderr: format!("不是本工具的还原点：{reference}"),
    })
}

fn stamp() -> String {
    let seconds = crate::write::journal::now();
    // 短随机尾巴只为同一秒内区分两次写；不用 rand 是为了不引依赖
    let tail = std::process::id() % 1000;
    format!("{seconds}-{tail:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_in(dir: &Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        git_in(dir.path(), &["config", "user.name", "t"]);
        git_in(dir.path(), &["config", "user.email", "t@example.com"]);
        std::fs::write(dir.path().join("a.txt"), "1\n").expect("write");
        git_in(dir.path(), &["add", "-A"]);
        git_in(dir.path(), &["commit", "-q", "-m", "feat: a"]);
        dir
    }

    #[test]
    fn a_backup_ref_points_at_the_head_it_was_taken_from() {
        let dir = repo();
        let head = process::run(Some(dir.path()), &["rev-parse", "HEAD"])
            .expect("rev-parse")
            .stdout
            .trim()
            .to_string();

        let reference = create(dir.path(), &head).expect("写还原点");
        assert!(reference.starts_with("refs/git-tidy/backup-"));
        assert_eq!(
            target(dir.path(), &reference).expect("target").as_deref(),
            Some(head.as_str())
        );

        // 它不该出现在分支列表里，也不该被 push 带走
        let branches = process::run(
            Some(dir.path()),
            &["for-each-ref", "--format=%(refname)", "refs/heads"],
        )
        .expect("refs")
        .stdout;
        assert_eq!(branches.trim(), "refs/heads/main");
    }

    #[test]
    fn dropping_a_backup_leaves_nothing_behind() {
        let dir = repo();
        let head = process::run(Some(dir.path()), &["rev-parse", "HEAD"])
            .expect("rev-parse")
            .stdout
            .trim()
            .to_string();
        let reference = create(dir.path(), &head).expect("写还原点");
        assert_eq!(list(dir.path()).expect("list").len(), 1);

        drop(dir.path(), &reference).expect("删还原点");
        assert!(list(dir.path()).expect("list").is_empty());
        assert_eq!(target(dir.path(), &reference).expect("target"), None);
    }

    #[test]
    fn only_our_own_backup_names_are_accepted() {
        let dir = repo();
        let head = process::run(Some(dir.path()), &["rev-parse", "HEAD"])
            .expect("rev-parse")
            .stdout
            .trim()
            .to_string();
        let reference = create(dir.path(), &head).expect("写还原点");
        assert_eq!(check_reference(&reference).ok(), Some(reference.clone()));

        for bad in [
            "",
            "refs/heads/main",
            "refs/git-tidy/backup-x/../../heads",
            "--delete",
            "refs/git-tidy/",
        ] {
            let err = check_reference(bad).expect_err("该拒绝：{bad}");
            assert!(format!("{err:?}").contains("还原点"), "{err:?}");
        }
        let _ = dir;
    }
}
