use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// 一条 stash。`ref` 是 `stash@{n}` 这种引用名：界面上的动作（apply/pop/drop）直接用它。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub reference: String,
    pub message: String,
    /// 入栈时的分支名
    pub branch: String,
    /// Unix 秒
    pub time: i64,
}

/// `stash list --format=%gd%x1f%gs%x1f%ci -z`。
///
/// `%gd` 给的是引用名而不是序号，所以界面不需要自己数"这是第几条"——
/// 中间有人跑过一次 `git stash`，序号全错，而引用名永远对得上。
pub fn list(repo: &Path) -> Result<Vec<StashEntry>, GitError> {
    let stdout = process::run(
        Some(repo),
        &["stash", "list", "--format=%gd%x1f%gs%x1f%ct", "-z"],
    )?
    .expect_success()?;

    let mut entries = Vec::new();
    for record in stdout.split('\0').filter(|record| !record.is_empty()) {
        let mut fields = record.split('\u{1f}');
        let (Some(reference), Some(message), Some(time)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(GitError::ParseFailure {
                snippet: process::snippet(record),
            });
        };
        // `%gs` 的两种形态（实测 git 2.54）：
        // 不给 `-m` 时是 `WIP on main: 1a2b3c4 提交标题`，给了 `-m` 时是 `On main: 改点东西`
        let branch = message
            .strip_prefix("WIP on ")
            .or_else(|| message.strip_prefix("On "))
            .and_then(|rest| rest.split_once(':').map(|(branch, _)| branch.trim()))
            .unwrap_or_default()
            .to_string();
        entries.push(StashEntry {
            reference: reference.to_string(),
            branch,
            message: message.to_string(),
            time: time.parse().unwrap_or(0),
        });
    }
    Ok(entries)
}

/// 存起来。`paths` 为 Some 表示只收这些文件（部分 stash），空列表视为不筛选。
///
/// pathspec 排在 `--` 之后：以 `-` 开头的文件名是真实存在的东西，
/// 不加 `--` 就会被 git 当成选项。
pub fn push(
    repo: &Path,
    paths: Option<&[String]>,
    include_untracked: bool,
    message: Option<&str>,
) -> Result<(), GitError> {
    let mut args = vec!["stash".to_string(), "push".to_string()];
    if include_untracked {
        // 不带 `-u` 时未跟踪文件会留在工作区：那是用户以为已经存起来了的东西
        args.push("-u".to_string());
    }
    if let Some(paths) = paths {
        if !paths.is_empty() {
            args.push("--".to_string());
            args.extend(paths.iter().cloned());
        }
    }

    if let Some(message) = message {
        // 正文直接当一个 argv 传：不经 shell（§5.5），多行与引号都不是问题
        args.push("-m".to_string());
        args.push(message.to_string());
    }
    process::run(Some(repo), &process::strs(&args))?.expect_success()?;
    Ok(())
}

/// 取出但不删。冲突时落进中断态，由 §7.3 的常驻提示条接住。
pub fn apply(repo: &Path, reference: &str) -> Result<(), GitError> {
    let reference = check_reference(reference)?;
    process::run(Some(repo), &["stash", "apply", &reference])?.expect_success()?;
    Ok(())
}

/// 取出并删。**冲突时不删那条 stash**：删了就再也回不去了（§7.9 的验收）。
pub fn pop(repo: &Path, reference: &str) -> Result<(), GitError> {
    let reference = check_reference(reference)?;
    let out = process::run(Some(repo), &["stash", "pop", &reference])?;
    if !out.success {
        return Err(conflict_or_failure(&out.stdout, &out.stderr));
    }
    Ok(())
}

pub fn drop(repo: &Path, reference: &str) -> Result<(), GitError> {
    let reference = check_reference(reference)?;
    process::run(Some(repo), &["stash", "drop", &reference])?.expect_success()?;
    Ok(())
}

/// 从一条 stash 开新分支并应用它（§7.9 的动作之一）。分支已存在时 git 会拒绝。
pub fn branch_from(repo: &Path, reference: &str, name: &str) -> Result<(), GitError> {
    let reference = check_reference(reference)?;
    super::branch::check_name(repo, name, "refs/heads/")?;
    process::run(Some(repo), &["stash", "branch", name, &reference])?.expect_success()?;
    Ok(())
}

/// stash 的失败要分两类给界面：冲突（会落中断态，界面要常驻提示）与普通失败。
///
/// 两个流都看：git 把 `CONFLICT (content): ...` 打在 stdout（实测），而
/// "The stash entry is kept in case you need it again." 之类的说明在 stderr。
fn conflict_or_failure(stdout: &str, stderr: &str) -> GitError {
    let both = format!("{stdout}\n{stderr}");
    let conflicted = both.contains("CONFLICT")
        || both.contains("conflict")
        || both.contains("could not restore untracked files")
        || both.contains("needs merge");
    if conflicted {
        return GitError::OperationInProgress {
            state: crate::git::refs::Interrupt::Merge,
        };
    }
    GitError::GitFailed {
        stderr: both.trim().to_string(),
    }
}

/// 引用名校验。只放行 `stash@{n}`：它会进 `git stash apply` 的参数位，
/// 而 stash 的输出是我们自己给的，不放心的形状就不该放进来。
fn check_reference(reference: &str) -> Result<String, GitError> {
    let trimmed = reference.trim();
    let shaped = trimmed.len() <= 24
        && trimmed.starts_with("stash@{")
        && trimmed.ends_with('}')
        && trimmed["stash@{".len()..trimmed.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit());
    if shaped {
        return Ok(trimmed.to_string());
    }
    Err(GitError::GitFailed {
        stderr: format!("不是合法的 stash 引用：{reference}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        out.stdout.trim_end().to_string()
    }

    fn must(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        out.stdout.trim_end().to_string()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        must(dir.path(), &["init", "-q", "-b", "main", "."]);
        must(dir.path(), &["config", "user.name", "测试者"]);
        must(dir.path(), &["config", "user.email", "t@example.com"]);
        must(dir.path(), &["config", "core.autocrlf", "false"]);
        fs::write(dir.path().join("a.txt"), "1\n").expect("write");
        must(dir.path(), &["add", "-A"]);
        must(dir.path(), &["commit", "-q", "-m", "feat: 基线"]);
        dir
    }

    #[test]
    fn push_list_apply_and_drop_round_trip() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("a.txt"), "改了\n").expect("write");

        push(path, None, false, Some("改点东西")).expect("push");
        assert_eq!(
            fs::read_to_string(path.join("a.txt")).expect("read"),
            "1\n",
            "push 之后工作区要回到干净状态"
        );

        let entries = list(path).expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].reference, "stash@{0}");
        assert_eq!(entries[0].branch, "main", "要能看出这条是在哪个分支上存的");
        assert!(entries[0].message.contains("改点东西"), "{:?}", entries[0]);

        apply(path, "stash@{0}").expect("apply");
        assert_eq!(
            fs::read_to_string(path.join("a.txt")).expect("read"),
            "改了\n"
        );
        assert_eq!(list(path).expect("list").len(), 1, "apply 不该把条目删掉");

        drop(path, "stash@{0}").expect("drop");
        assert!(list(path).expect("list").is_empty());
    }

    /// §7.9 的验收：带未跟踪文件的 stash apply 之后，文件集与操作前逐名一致
    #[test]
    fn untracked_files_come_back_with_the_pop() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("a.txt"), "改了\n").expect("write");
        fs::write(path.join("新文件.txt"), "新的\n").expect("write");

        push(path, None, true, None).expect("push 带 -u");
        assert!(!path.join("新文件.txt").exists(), "未跟踪文件也要被收走");

        pop(path, "stash@{0}").expect("pop");
        assert_eq!(
            fs::read_to_string(path.join("a.txt")).expect("read"),
            "改了\n"
        );
        assert_eq!(
            fs::read_to_string(path.join("新文件.txt")).expect("read"),
            "新的\n",
            "未跟踪的文件也得回来"
        );
        assert!(
            list(path).expect("list").is_empty(),
            "pop 成功后条目应该没了"
        );
    }

    /// pop 撞冲突时条目必须还在——删了就再也回不去了
    #[test]
    fn a_conflicting_pop_keeps_the_entry_and_reports_the_interrupt() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("a.txt"), "存起来的内容\n").expect("write");
        push(path, None, false, None).expect("push");

        // 制造一次冲突：同一行两边改成不同内容
        fs::write(path.join("a.txt"), "另一份内容\n").expect("write");
        must(path, &["commit", "-qam", "feat: 另一条线"]);

        let err = pop(path, "stash@{0}").expect_err("这次 pop 必然冲突");
        assert!(
            matches!(err, GitError::OperationInProgress { .. }),
            "冲突要落进中断态，界面据此常驻提示：{err:?}"
        );
        assert_eq!(
            list(path).expect("list").len(),
            1,
            "冲突时那条 stash 不能被 pop 掉"
        );
    }

    #[test]
    fn a_stash_can_become_a_branch() {
        let dir = repo();
        let path = dir.path();
        fs::write(path.join("a.txt"), "改点\n").expect("write");
        push(path, None, false, None).expect("push");

        branch_from(path, "stash@{0}", "rescue").expect("从 stash 建分支");
        assert_eq!(must(path, &["rev-parse", "--abbrev-ref", "HEAD"]), "rescue");
        assert_eq!(
            fs::read_to_string(path.join("a.txt")).expect("read"),
            "改点\n"
        );
        assert!(
            list(path).expect("list").is_empty(),
            "建分支会消费掉那条 stash"
        );
    }

    #[test]
    fn only_shapes_git_emitted_are_accepted() {
        for bad in [
            "",
            "  ",
            "stash@",
            "stash@{x}",
            "stash@{0} extra",
            "--all",
            "HEAD",
        ] {
            let err = check_reference(bad).expect_err("该拒绝：{bad}");
            assert!(format!("{err:?}").contains("stash 引用"), "{err:?}");
        }
        assert_eq!(
            check_reference(" stash@{12} ").ok(),
            Some("stash@{12}".to_string())
        );
    }

    #[test]
    fn an_empty_list_is_not_an_error() {
        let dir = repo();
        assert!(list(dir.path()).expect("list").is_empty());
        let _ = git_in(dir.path(), &["status"]);
    }
}
