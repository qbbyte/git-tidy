use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;

use super::process;
use crate::error::GitError;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub work_tree: PathBuf,
    pub git_dir: PathBuf,
    pub git_version: String,
    /// None 表示游离 HEAD（detached）
    pub branch: Option<String>,
    /// None 表示空仓库，还没有任何提交
    pub head_commit: Option<String>,
    pub dirty: bool,
}

/// 探测一个目录是否为 Git 仓库，并读取仓库页需要的基础信息。
/// 这是"添加仓库"流程的入口，只做读取，不写任何东西。
pub fn probe(path: &str) -> Result<RepoInfo, GitError> {
    let root = Path::new(path);
    if !root.is_dir() {
        return Err(GitError::NotARepo);
    }

    let git_version = process::run(None, &["--version"])?
        .expect_success()?
        .trim()
        .strip_prefix("git version ")
        .unwrap_or_default()
        .to_string();

    let git_dir = field(root, &["rev-parse", "--absolute-git-dir"])?.ok_or(GitError::NotARepo)?;
    let work_tree = field(root, &["rev-parse", "--show-toplevel"])?.ok_or(GitError::NotARepo)?;

    let branch = field(root, &["symbolic-ref", "--short", "-q", "HEAD"])?.filter(|b| !b.is_empty());
    let head_commit = field(root, &["rev-parse", "HEAD"])?;
    let status = process::run(Some(root), &["status", "--porcelain"])?.expect_success()?;

    Ok(RepoInfo {
        work_tree: PathBuf::from(work_tree),
        git_dir: PathBuf::from(git_dir),
        git_version,
        branch,
        head_commit,
        dirty: !status.trim().is_empty(),
    })
}

/// 执行一条只输出一行的查询命令。空仓库、游离 HEAD 这类"命令按预期失败"
/// 在这里归一成 None，而不是错误——它们是需要区分的正常状态。
fn field(repo: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    let out = process::run(Some(repo), args)?;
    Ok(out.success.then(|| out.stdout.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_in(dir: &Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} failed: {}", out.stderr);
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "."]);
        dir
    }

    fn probe_dir(dir: &Path) -> RepoInfo {
        probe(dir.to_str().expect("utf-8 temp path")).expect("probe ok")
    }

    #[test]
    fn plain_directory_is_not_a_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            probe(dir.path().to_str().unwrap()),
            Err(GitError::NotARepo)
        ));
    }

    #[test]
    fn file_path_is_not_a_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.txt");
        fs::write(&file, b"x").expect("write");
        assert!(matches!(
            probe(file.to_str().unwrap()),
            Err(GitError::NotARepo)
        ));
    }

    #[test]
    fn empty_repo_has_branch_but_no_head_commit() {
        let repo = init_repo();
        let info = probe_dir(repo.path());
        assert!(
            info.branch.is_some(),
            "刚 init 的仓库 HEAD 指向未诞生的分支，仍应有分支名"
        );
        assert!(info.head_commit.is_none(), "空仓库不该有 HEAD 提交");
        assert!(!info.dirty);
    }

    #[test]
    fn repo_with_commit_reports_head_and_toggles_dirty() {
        let repo = init_repo();
        let work = repo.path();
        fs::write(work.join("a.txt"), "one\n").expect("write");
        git_in(work, &["add", "a.txt"]);
        git_in(
            work,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "feat: first",
            ],
        );

        let clean = probe_dir(work);
        let head = clean.head_commit.expect("应有 HEAD 提交");
        assert_eq!(head.len(), 40, "HEAD 应是完整 sha");
        assert!(!clean.dirty);

        fs::write(work.join("a.txt"), "one\ntwo\n").expect("write");
        assert!(probe_dir(work).dirty, "改动已跟踪文件后应报告 dirty");
    }

    #[test]
    fn detached_head_reports_no_branch() {
        let repo = init_repo();
        let work = repo.path();
        fs::write(work.join("a.txt"), "one\n").expect("write");
        git_in(work, &["add", "a.txt"]);
        git_in(
            work,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "feat: first",
            ],
        );
        git_in(work, &["checkout", "-q", "--detach"]);

        let info = probe_dir(work);
        assert!(info.branch.is_none(), "游离 HEAD 不该报告分支名");
        assert!(info.head_commit.is_some());
    }
}
