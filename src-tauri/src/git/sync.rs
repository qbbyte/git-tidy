use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// 怎么拉。默认快进——不是快进就报"需要先决定合并还是变基"，
/// 那是用户的决定，工具不替他做（§7.12）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PullStrategy {
    FfOnly,
    Rebase,
}

/// 一次远程操作的结果。远程写（push / 删除远程分支）与本地写分开标出来：
/// 界面要能说清"哪些改动已经离开这台机器了"。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub action: String,
    /// git 的进度行（最后一两行足够看出发生了什么），不给界面编造进度百分比
    pub summary: Vec<String>,
    /// push 之后远程引用落到了哪里（`refs/heads/main` 那一类）
    pub updated: Vec<String>,
}

/// 进度回调：每次拿到 git 的一行 stderr 就回调一次。签名与克隆那一套一致，
/// 所以界面上是同一个进度条。
pub fn fetch<F>(repo: &Path, remote: Option<&str>, mut on_line: F) -> Result<SyncReport, GitError>
where
    F: FnMut(&str),
{
    let mut args = vec![
        "fetch".to_string(),
        "--prune".to_string(),
        "--tags".to_string(),
    ];
    if let Some(remote) = remote {
        args.push(remote.to_string());
    }
    run_streaming(repo, &args, "fetch", &mut on_line)
}

/// 拉取并合入当前分支。`--ff-only` 是默认：合不进去就停，不自动制造合并提交。
pub fn pull<F>(
    repo: &Path,
    remote: Option<&str>,
    strategy: PullStrategy,
    mut on_line: F,
) -> Result<SyncReport, GitError>
where
    F: FnMut(&str),
{
    let mut args = vec!["pull".to_string()];
    match strategy {
        PullStrategy::FfOnly => args.push("--ff-only".to_string()),
        PullStrategy::Rebase => args.push("--rebase".to_string()),
    }
    if let Some(remote) = remote {
        args.push(remote.to_string());
    }
    let outcome = run_streaming(repo, &args, "pull", &mut on_line);
    // 非快进且策略是 ff-only：这不是失败，是"要用户先做决定"，所以给一条能执行的话
    if let Err(GitError::GitFailed { stderr }) = &outcome {
        if stderr.contains("Not possible to fast-forward")
            || stderr.contains("need to specify how to reconcile divergent branches")
            || stderr.contains("cannot pull with rebase")
        {
            return Err(GitError::Diverged {
                detail: "本地与远程已经分叉，默认的快进拉取停在这里。请选择合并或变基后再拉一次"
                    .into(),
            });
        }
    }
    outcome
}

/// 推送。当前分支为空则推当前分支；`set_upstream` 为真时用 `-u`（新分支首次推送）。
///
/// **只用 `--force-with-lease`，绝不用裸 `--force`**（§7.12）：对方抢先推过东西时，
/// 裸 force 会把别人的提交抹掉，lease 会失败并保留现场。
pub fn push(
    repo: &Path,
    remote: &str,
    branch: &str,
    set_upstream: bool,
    force_with_lease: bool,
) -> Result<SyncReport, GitError> {
    let mut args = vec!["push".to_string()];
    if set_upstream {
        args.push("-u".to_string());
    }
    if force_with_lease {
        args.push("--force-with-lease".to_string());
    }
    args.push(remote.to_string());
    args.push(branch.to_string());
    let report = run_streaming(repo, &args, "push", &mut |_| {})?;
    Ok(SyncReport {
        updated: updated_refs(&report.summary, branch),
        ..report
    })
}

/// 删除远程分支。`push <remote> --delete <br>`：这是远程写，确认强度按 §7.12 走，
/// 界面上要求用户手输分支名。
pub fn delete_remote_branch(
    repo: &Path,
    remote: &str,
    branch: &str,
) -> Result<SyncReport, GitError> {
    let args = vec![
        "push".to_string(),
        remote.to_string(),
        "--delete".to_string(),
        branch.to_string(),
    ];
    run_streaming(repo, &args, "delete_remote_branch", &mut |_| {})
}

/// lease 被拒时给一条能执行的话（§7.12）：对方已经推了新东西。
fn map_failure(_action: &str, stderr: &str, stdout: &str) -> GitError {
    let both = format!("{stdout}\n{stderr}");
    if both.contains("Authentication failed")
        || both.contains("could not read Username")
        || both.contains("Permission denied")
        || both.contains("terminal prompts disabled")
        || both.contains("Authentication required")
    {
        // 凭据交给系统 git / GCM / ssh-agent，我们不经手（§6.14）
        return GitError::AuthRequired {
            detail: redact(&both),
        };
    }
    if both.contains("stale info")
        || both.contains("fetch first")
        || both.contains("--force-with-lease alone")
    {
        return GitError::Diverged {
            detail: "对方已经推了新提交，改写推送被 lease 挡下了。先 fetch 再试".into(),
        };
    }
    if both.contains("index.lock") {
        return GitError::RepoBusy;
    }
    GitError::GitFailed {
        stderr: redact(&both),
    }
}

/// 远程命令的 stderr 里可能带 URL 上的凭据。写进错误详情前先抹掉：
/// 日志与错误都会落到磁盘上，而凭据不该进我们的库（§6.14）。
fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        // `https://user:token@host/...` 这一段整体换成 `https://***@host/...`
        if let Some((head, tail)) = line.split_once("://") {
            if let Some((_credentials, rest)) = tail.split_once('@') {
                out.push_str(head);
                out.push_str("://***@");
                out.push_str(rest);
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// 从 git 的输出里摘出"哪些引用更新了"。只取 `refs/` 开头的那些行，
/// 其余（进度、统计）归到 summary。
fn updated_refs(summary: &[String], branch: &str) -> Vec<String> {
    let _ = branch;
    summary
        .iter()
        .filter(|line| line.contains("refs/"))
        .cloned()
        .collect()
}

/// 跑一条远程命令，进度逐行回调。`process::run_streaming` 已经按 `\r` 与 `\n` 双双断行，
/// 所以下载期间的进度也能一条条发出去。
fn run_streaming(
    repo: &Path,
    args: &[String],
    action: &str,
    on_line: &mut dyn FnMut(&str),
) -> Result<SyncReport, GitError> {
    let out = process::run_streaming(Some(repo), &process::strs(args), |line| on_line(line))?;
    if !out.success {
        return Err(map_failure(action, &out.stderr, &out.stdout));
    }

    let lines: Vec<String> = out
        .stdout
        .lines()
        .chain(out.stderr.lines())
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    Ok(SyncReport {
        action: action.to_string(),
        summary: lines,
        updated: Vec::new(),
    })
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

    /// 一个本地裸库当远程：验收全部在本地跑，不依赖网络
    fn pair() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        // 裸库直接建在自己的目录里，不靠改名：改名在某些平台上会踩到句柄还开着的问题
        let bare = tmp.path().join("origin.git");
        std::fs::create_dir_all(&bare).expect("mkdir");
        must(&bare, &["init", "-q", "--bare", "-b", "main", "."]);

        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).expect("mkdir");
        must(&work, &["init", "-q", "-b", "main", "."]);
        must(&work, &["config", "user.name", "测试者"]);
        must(&work, &["config", "user.email", "t@example.com"]);
        must(&work, &["config", "core.autocrlf", "false"]);
        must(
            &work,
            &["remote", "add", "origin", bare.to_string_lossy().as_ref()],
        );
        fs::write(work.join("a.txt"), "1\n").expect("write");
        must(&work, &["add", "-A"]);
        must(&work, &["commit", "-q", "-m", "feat: 基线"]);
        must(&work, &["push", "-q", "-u", "origin", "main"]);
        (tmp, bare, work)
    }

    fn commit(work: &Path, content: &str, message: &str) {
        fs::write(work.join("a.txt"), content).expect("write");
        must(work, &["commit", "-qam", message]);
    }

    #[test]
    fn fetch_pulls_a_new_remote_commit() {
        let (_tmp, bare, work) = pair();
        // 另一个克隆推一条
        let other = bare.parent().unwrap().join("other");
        std::fs::create_dir_all(&other).expect("mkdir");
        must(&other, &["init", "-q", "-b", "main", "."]);
        must(&other, &["config", "user.name", "别人"]);
        must(&other, &["config", "user.email", "other@example.com"]);
        must(
            &other,
            &["remote", "add", "origin", bare.to_string_lossy().as_ref()],
        );
        must(&other, &["fetch", "-q", "origin"]);
        must(&other, &["checkout", "-q", "-B", "main", "origin/main"]);
        commit(&other, "别人改的\n", "fix: 别人推了一条");
        must(&other, &["push", "-q", "origin", "main"]);

        let mut lines: Vec<String> = Vec::new();
        let report =
            fetch(&work, Some("origin"), |line| lines.push(line.to_string())).expect("fetch");
        assert_eq!(report.action, "fetch");
        assert!(
            must(&work, &["rev-parse", "origin/main"]) != must(&work, &["rev-parse", "HEAD"]),
            "fetch 之后远程引用要指到别人的提交上"
        );
    }

    #[test]
    fn a_new_branch_pushes_and_sets_upstream() {
        let (_tmp, _bare, work) = pair();
        must(&work, &["checkout", "-q", "-b", "feat/new"]);

        let report = push(&work, "origin", "feat/new", true, false).expect("push");
        assert!(!report.summary.is_empty());
        let listed = must(&work, &["ls-remote", "--heads", "origin"]);
        assert!(listed.contains("refs/heads/feat/new"), "{listed}");
        assert_eq!(
            must(&work, &["rev-parse", "--abbrev-ref", "feat/new@{upstream}"]),
            "origin/feat/new",
            "-u 要把上游配上"
        );
    }

    /// §7.12 的验收：新分支推送后 `ls-remote` 可见；远端被抢先推一条时
    /// `--force-with-lease` 必须失败而不是覆盖
    #[test]
    fn force_with_lease_refuses_to_overwrite_someone_elses_push() {
        let (_tmp, bare, work) = pair();
        must(&work, &["checkout", "-q", "-b", "shared"]);
        must(&work, &["push", "-q", "-u", "origin", "shared"]);

        // 另一个人先推了一条
        let other = bare.parent().unwrap().join("other2");
        std::fs::create_dir_all(&other).expect("mkdir");
        must(&other, &["init", "-q", "-b", "main", "."]);
        must(&other, &["config", "user.name", "别人"]);
        must(&other, &["config", "user.email", "other@example.com"]);
        must(
            &other,
            &["remote", "add", "origin", bare.to_string_lossy().as_ref()],
        );
        must(&other, &["fetch", "-q", "origin"]);
        must(&other, &["checkout", "-q", "-B", "shared", "origin/shared"]);
        commit(&other, "别人写的\n", "fix: 别人先推");
        must(&other, &["push", "-q", "origin", "shared"]);
        let theirs = must(&other, &["rev-parse", "HEAD"]);

        // 我这边基于旧历史改写自己的
        commit(&work, "我写的\n", "feat: 我这边改了");
        commit(&work, "再改一次\n", "feat: 再改一次");
        must(&work, &["reset", "-q", "--hard", "HEAD~1"]);
        commit(&work, "我改写的\n", "feat: 改写后的版本");

        let err = push(&work, "origin", "shared", false, true).expect_err("lease 该挡住");
        assert!(matches!(err, GitError::Diverged { .. }), "{err:?}");

        // 远端那一条必须还在。要查**远端**而不是本地跟踪引用：
        // 被拒的推送不会更新 origin/shared，拿它比只会证明跟踪引用没动
        let remote_now = must(&work, &["ls-remote", "origin", "refs/heads/shared"]);
        assert!(
            remote_now.contains(&theirs),
            "别人的提交不能被覆盖掉，远端现在是 {remote_now}，他们的是 {theirs}"
        );
    }

    #[test]
    fn pull_stops_when_the_branches_have_diverged() {
        let (_tmp, bare, work) = pair();
        let other = bare.parent().unwrap().join("other3");
        std::fs::create_dir_all(&other).expect("mkdir");
        must(&other, &["init", "-q", "-b", "main", "."]);
        must(&other, &["config", "user.name", "别人"]);
        must(&other, &["config", "user.email", "other@example.com"]);
        must(
            &other,
            &["remote", "add", "origin", bare.to_string_lossy().as_ref()],
        );
        must(&other, &["fetch", "-q", "origin"]);
        must(&other, &["checkout", "-q", "-B", "main", "origin/main"]);
        commit(&other, "别人的\n", "fix: 远程一条");
        must(&other, &["push", "-q", "origin", "main"]);

        commit(&work, "我自己的\n", "feat: 本地一条");
        // 本地没记住远程的新提交，所以要显式快进到远程头，造出真正的分叉
        let remote_tip = must(&other, &["rev-parse", "HEAD"]);
        must(&work, &["fetch", "-q", "origin"]);
        must(&work, &["reset", "-q", "--hard", "HEAD"]);
        commit(&work, "本地第二条\n", "feat: 本地第二条");
        must(
            &work,
            &["update-ref", "refs/remotes/origin/main", &remote_tip],
        );

        let err = pull(&work, Some("origin"), PullStrategy::FfOnly, |_| {})
            .expect_err("分叉时快进拉取必须停");
        assert!(matches!(err, GitError::Diverged { .. }), "{err:?}");
        let message = match err {
            GitError::Diverged { detail } => detail,
            _ => unreachable!(),
        };
        assert!(
            message.contains("合并") || message.contains("变基"),
            "{message}"
        );
    }

    #[test]
    fn a_fast_forwardable_pull_goes_through() {
        let (_tmp, bare, work) = pair();
        let other = bare.parent().unwrap().join("other4");
        std::fs::create_dir_all(&other).expect("mkdir");
        must(&other, &["init", "-q", "-b", "main", "."]);
        must(&other, &["config", "user.name", "别人"]);
        must(&other, &["config", "user.email", "other@example.com"]);
        must(
            &other,
            &["remote", "add", "origin", bare.to_string_lossy().as_ref()],
        );
        must(&other, &["fetch", "-q", "origin"]);
        must(&other, &["checkout", "-q", "-B", "main", "origin/main"]);
        commit(&other, "别人的\n", "fix: 远程一条");
        must(&other, &["push", "-q", "origin", "main"]);

        pull(&work, Some("origin"), PullStrategy::FfOnly, |_| {}).expect("能快进就该拉下来");
        assert_eq!(
            must(&work, &["rev-parse", "HEAD"]),
            must(&work, &["rev-parse", "origin/main"])
        );
    }

    #[test]
    fn deleting_a_remote_branch_takes_an_explicit_command() {
        let (_tmp, _bare, work) = pair();
        must(&work, &["branch", "doomed"]);
        must(&work, &["push", "-q", "origin", "doomed"]);

        let report = delete_remote_branch(&work, "origin", "doomed").expect("删远程分支");
        assert_eq!(report.action, "delete_remote_branch");
        let listed = must(&work, &["ls-remote", "--heads", "origin"]);
        assert!(!listed.contains("refs/heads/doomed"), "{listed}");
    }

    /// 凭据不进错误详情：URL 里带的 token 会被抹掉
    #[test]
    fn credentials_never_reach_the_error_payload() {
        let text = "fatal: could not read from https://user:s3cr3t@github.com/a/b.git";
        let redacted = redact(text);
        assert!(!redacted.contains("s3cr3t"), "{redacted}");
        assert!(redacted.contains("https://***@github.com"), "{redacted}");
    }

    #[test]
    fn an_authentication_failure_is_mapped_to_its_own_error() {
        let err = map_failure(
            "push",
            "fatal: Authentication failed for 'https://github.com/a/b.git/'",
            "",
        );
        assert!(matches!(err, GitError::AuthRequired { .. }), "{err:?}");
    }
}
