use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// reset 的三档。名字与 `git reset` 的参数一字不差：界面上摆的必须是用户会背的那三个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResetMode {
    /// 提交留在历史里，改动回到暂存区
    Soft,
    /// 提交留在历史里，改动回到工作区
    Mixed,
    /// 提交与改动都扔掉
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }
}

/// 摘取一批提交。`-x` 记来源：日后 `git log` 里能看出这条是从哪摘来的。
///
/// 支持区间与多个 sha，所以参数是一个列表。冲突时 git 非零退出，
/// 上层把它映射成中断态——**不做任何回滚**，那会抹掉用户已经解好的冲突。
pub fn cherry_pick(repo: &Path, shas: &[String], record_source: bool) -> Result<(), GitError> {
    if shas.is_empty() {
        return Err(GitError::NothingStaged);
    }
    let mut args = vec!["cherry-pick".to_string()];
    if record_source {
        args.push("-x".to_string());
    }
    args.extend(shas.iter().cloned());
    let out = process::run(Some(repo), &process::strs(&args))?;
    if out.success {
        return Ok(());
    }
    Err(sequence_failure(&out.stderr))
}

/// 回滚一条提交。合并提交**必须**显式给主线号（`-m 1|2`）：选错主线是真实事故，
/// 所以这里不接受"没给就用 1"这种默认值，缺参数直接拒。
pub fn revert(repo: &Path, sha: &str, mainline: Option<usize>) -> Result<(), GitError> {
    if sha.is_empty() {
        return Err(GitError::GitFailed {
            stderr: "没有指定要回滚的提交".into(),
        });
    }
    let mut args = vec!["revert".to_string()];
    match mainline {
        Some(parent) if (1..=2).contains(&parent) => {
            args.push("-m".to_string());
            args.push(parent.to_string());
        }
        Some(_) => {
            return Err(GitError::GitFailed {
                stderr: "主线号只能是 1 或 2".into(),
            })
        }
        None => {}
    }
    args.push("--no-edit".to_string());
    args.push(sha.to_string());
    let out = process::run(Some(repo), &process::strs(&args))?;
    if out.success {
        return Ok(());
    }
    Err(sequence_failure(&out.stderr))
}

/// 移动 HEAD。`target` 默认为 HEAD，即"撤销最近若干次提交"。
pub fn reset(repo: &Path, mode: ResetMode, target: &str) -> Result<(), GitError> {
    let out = process::run(Some(repo), &["reset", mode.flag(), target])?;
    if out.success {
        return Ok(());
    }
    Err(GitError::GitFailed { stderr: out.stderr })
}

/// 放弃一次冲突中的序列。cherry-pick / revert / rebase 各自有对应的 `--abort`，
/// 统一走标记文件判断当前是哪一种（M2 只有一键退回，逐块解决在 M3）。
pub fn abort(repo: &Path) -> Result<AbortOutcome, GitError> {
    let info = crate::git::refs::interrupt(repo)?;
    use crate::git::refs::Interrupt::*;
    let (command, kind) = match info.kind {
        CherryPick => ("cherry-pick", "cherry-pick"),
        Revert => ("revert", "revert"),
        Rebase => ("rebase", "rebase"),
        Merge => ("merge", "merge"),
        None => {
            return Err(GitError::GitFailed {
                stderr: "这个仓库没有进行到一半的操作".into(),
            })
        }
    };
    let out = process::run(Some(repo), &[command, "--abort"])?;
    if !out.success {
        return Err(GitError::GitFailed { stderr: out.stderr });
    }
    Ok(AbortOutcome {
        aborted: kind.to_string(),
        branch: info.branch,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbortOutcome {
    pub aborted: String,
    pub branch: Option<String>,
}

/// 序列类命令的失败分两类：**冲突**（落中断态，界面常驻提示）与普通失败。
/// 只有前者能映射成 `OperationInProgress`——那决定了界面是给"退回"还是只报错。
fn sequence_failure(stderr: &str) -> GitError {
    let conflicted = stderr.contains("CONFLICT")
        || stderr.contains("conflict")
        || stderr.contains("could not apply")
        || stderr.contains("needs merge");
    if conflicted {
        // 摘取/回滚留下的中断态：标记文件是 CHERRY_PICK_HEAD / REVERT_HEAD，
        // 由 refs::interrupt 判具体是哪一种；这里只报"有冲突"，具体种类让界面重读一次状态
        return GitError::OperationInProgress {
            state: crate::git::refs::Interrupt::CherryPick,
        };
    }
    GitError::GitFailed {
        stderr: stderr.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn must(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        out.stdout.trim_end().to_string()
    }

    fn commit(dir: &Path, file: &str, content: &str, message: &str) -> String {
        fs::write(dir.join(file), content).expect("write");
        must(dir, &["add", "-A"]);
        must(dir, &["commit", "-q", "-m", message]);
        must(dir, &["rev-parse", "HEAD"])
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        must(dir.path(), &["init", "-q", "-b", "main", "."]);
        must(dir.path(), &["config", "user.name", "测试者"]);
        must(dir.path(), &["config", "user.email", "t@example.com"]);
        must(dir.path(), &["config", "core.autocrlf", "false"]);
        commit(dir.path(), "a.txt", "1\n", "feat: 基线");
        dir
    }

    /// §7.11 的验收：cherry-pick 一个干净提交后，新提交的 diff 与源提交等价
    #[test]
    fn a_clean_cherry_pick_produces_an_equivalent_change() {
        let dir = repo();
        let path = dir.path();
        must(path, &["checkout", "-q", "-b", "side"]);
        let source = commit(path, "b.txt", "带过来的内容\n", "feat: 支线上的提交");
        must(path, &["checkout", "-q", "main"]);

        cherry_pick(path, std::slice::from_ref(&source), true).expect("摘取");
        let message = must(path, &["log", "-1", "--format=%B"]);
        assert!(
            message.contains(&format!("cherry picked from commit {source}")),
            "-x 要把来源记进正文：{message}"
        );
        assert_eq!(
            fs::read_to_string(path.join("b.txt")).expect("read"),
            "带过来的内容\n"
        );
    }

    /// 冲突时落进中断态，而且标记文件要真的留在那里（界面靠它给"退回"按钮）
    #[test]
    fn a_conflicting_cherry_pick_lands_in_the_interrupt_state() {
        let dir = repo();
        let path = dir.path();
        let source = commit(path, "a.txt", "支线的改法\n", "feat: 支线");
        must(path, &["checkout", "-q", "-b", "target"]);
        commit(path, "a.txt", "另一条线的改法\n", "feat: 另一条线");

        let err = cherry_pick(path, &[source], false).expect_err("必然冲突");
        assert!(
            matches!(err, GitError::OperationInProgress { .. }),
            "冲突要映射成中断态：{err:?}"
        );
        let interrupt = crate::git::refs::interrupt(path).expect("interrupt");
        assert_eq!(interrupt.kind, crate::git::refs::Interrupt::CherryPick);

        abort(path).expect("退回");
        assert_eq!(
            fs::read_to_string(path.join("a.txt")).expect("read"),
            "另一条线的改法\n",
            "退回之后工作区要回到冲突前"
        );
        assert_eq!(
            crate::git::refs::interrupt(path).expect("interrupt").kind,
            crate::git::refs::Interrupt::None
        );
    }

    #[test]
    fn revert_undoes_a_commit() {
        let dir = repo();
        let path = dir.path();
        let sha = commit(path, "a.txt", "2\n", "fix: 改一行");

        revert(path, &sha, None).expect("回滚");
        assert_eq!(fs::read_to_string(path.join("a.txt")).expect("read"), "1\n");
        assert!(
            must(path, &["log", "-1", "--format=%s"]).starts_with("Revert"),
            "回滚要留下 Revert 提交"
        );
    }

    /// 合并提交必须显式选主线，不接受默认的 `-m 1`
    #[test]
    fn reverting_a_merge_demands_an_explicit_mainline() {
        let dir = repo();
        let path = dir.path();
        must(path, &["checkout", "-q", "-b", "side"]);
        commit(path, "b.txt", "支线\n", "feat: 支线");
        must(path, &["checkout", "-q", "main"]);
        must(
            path,
            &["merge", "--no-ff", "-q", "-m", "chore: 合并", "side"],
        );
        let merge = must(path, &["rev-parse", "HEAD"]);

        // 不给主线号：git 自己会拒绝（fatal: commit ... is a merge but no -m option was given）
        let err = revert(path, &merge, None).expect_err("合并回滚不该默认主线");
        assert!(matches!(err, GitError::GitFailed { .. }), "{err:?}");

        let err = revert(path, &merge, Some(5)).expect_err("主线号只能是 1 或 2");
        assert!(format!("{err:?}").contains("1 或 2"), "{err:?}");

        revert(path, &merge, Some(1)).expect("显式主线后就能回滚");
        assert!(!path.join("b.txt").exists(), "-m 1 是回掉第一父带来的那侧");
    }

    #[test]
    fn reset_three_modes_move_the_worktree_differently() {
        let dir = repo();
        let path = dir.path();
        let base = must(path, &["rev-parse", "HEAD"]);
        commit(path, "a.txt", "2\n", "fix: 第二次");
        let tip = must(path, &["rev-parse", "HEAD"]);

        reset(path, ResetMode::Soft, &base).expect("soft");
        assert_eq!(
            must(path, &["rev-parse", "HEAD"]),
            base,
            "soft 保留提交之外的指针移动"
        );
        assert_eq!(
            must(path, &["status", "--porcelain"]),
            "M  a.txt",
            "soft 把改动留在暂存区"
        );

        let dir2 = repo();
        let path2 = dir2.path();
        let base2 = must(path2, &["rev-parse", "HEAD"]);
        commit(path2, "a.txt", "3\n", "fix: 第三次");
        reset(path2, ResetMode::Hard, &base2).expect("hard");
        assert_eq!(
            fs::read_to_string(path2.join("a.txt")).expect("read"),
            "1\n"
        );
        assert!(
            must(path2, &["status", "--porcelain"]).is_empty(),
            "hard 把改动也扔掉"
        );
        let _ = tip;
    }

    #[test]
    fn aborting_without_an_operation_is_a_plain_answer() {
        let dir = repo();
        let err = abort(dir.path()).expect_err("没有可退回的操作");
        assert!(format!("{err:?}").contains("没有进行到一半"), "{err:?}");
    }
}
