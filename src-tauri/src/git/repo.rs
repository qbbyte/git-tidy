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
}

/// 探测一个目录是否为 Git 仓库，并读取仓库页需要的基础信息。只做读取，不写任何东西。
///
/// 这里**不**报告工作区是否干净：`git status` 属于待提交文件那一层（git/status.rs），
/// 而且 treeless 浏览仓库根本没有工作区，在探测里塞一个 status 只会造出假"脏"报告。
pub fn probe(path: &Path) -> Result<RepoInfo, GitError> {
    if !path.is_dir() {
        return Err(GitError::NotARepo);
    }

    let git_version = process::run(None, &["--version"])?
        .expect_success()?
        .trim()
        .strip_prefix("git version ")
        .unwrap_or_default()
        .to_string();

    let git_dir = field(path, &["rev-parse", "--absolute-git-dir"])?.ok_or(GitError::NotARepo)?;
    let work_tree = field(path, &["rev-parse", "--show-toplevel"])?.ok_or(GitError::NotARepo)?;
    let branch = field(path, &["symbolic-ref", "--short", "-q", "HEAD"])?.filter(|b| !b.is_empty());
    let head_commit = field(path, &["rev-parse", "HEAD"])?;

    Ok(RepoInfo {
        work_tree: PathBuf::from(work_tree),
        git_dir: PathBuf::from(git_dir),
        git_version,
        branch,
        head_commit,
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

    fn commit_a_file(work: &Path) {
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
    }

    #[test]
    fn plain_directory_is_not_a_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(probe(dir.path()), Err(GitError::NotARepo)));
    }

    #[test]
    fn file_path_is_not_a_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.txt");
        fs::write(&file, b"x").expect("write");
        assert!(matches!(probe(&file), Err(GitError::NotARepo)));
    }

    #[test]
    fn empty_repo_has_branch_but_no_head_commit() {
        let repo = init_repo();
        let info = probe(repo.path()).expect("probe ok");
        assert!(
            info.branch.is_some(),
            "刚 init 的仓库 HEAD 指向未诞生的分支，仍应有分支名"
        );
        assert!(info.head_commit.is_none(), "空仓库不该有 HEAD 提交");
    }

    #[test]
    fn probing_a_subdirectory_resolves_the_repo_root() {
        let repo = init_repo();
        commit_a_file(repo.path());
        let sub = repo.path().join("src").join("deep");
        fs::create_dir_all(&sub).expect("mkdir");

        // 两边都比 git 自己的输出，不引 fs::canonicalize：它在 Windows 上会加 \\?\ 前缀，
        // 而 git 返回正斜杠形式，比出来的是路径写法差异而不是真 bug
        let from_root = probe(repo.path()).expect("probe root");
        let from_sub = probe(&sub).expect("probe sub dir");
        assert_eq!(
            from_sub.work_tree, from_root.work_tree,
            "在子目录探测必须落到工作区顶层"
        );
        assert!(!from_sub.work_tree.ends_with("deep"));
    }

    #[test]
    fn head_commit_is_a_full_sha() {
        let repo = init_repo();
        commit_a_file(repo.path());
        let info = probe(repo.path()).expect("probe ok");
        let head = info.head_commit.expect("应有 HEAD 提交");
        assert_eq!(head.len(), 40, "HEAD 应是完整 sha");
    }

    #[test]
    fn detached_head_reports_no_branch() {
        let repo = init_repo();
        commit_a_file(repo.path());
        git_in(repo.path(), &["checkout", "-q", "--detach"]);

        let info = probe(repo.path()).expect("probe ok");
        assert!(info.branch.is_none(), "游离 HEAD 不该报告分支名");
        assert!(info.head_commit.is_some());
    }

    #[test]
    fn a_treeless_clone_is_still_probeable() {
        // --no-checkout + --filter=tree:0 的仓库里 git status 会列出成千上万条
        // "D"（实测 git/git：4857 行），所以探测绝不能碰 status
        let remote = init_repo();
        commit_a_file(remote.path());

        let dir = tempfile::tempdir().expect("tempdir");
        let clone = dir.path().join("browse");
        git_in(
            remote.path(),
            &[
                "clone",
                "-q",
                "--filter=tree:0",
                "--no-checkout",
                remote.path().to_str().expect("utf-8 path"),
                clone.to_str().expect("utf-8 path"),
            ],
        );

        let info = probe(&clone).expect("treeless 仓库要能探测");
        assert!(info.head_commit.is_some(), "commit 对象齐全，HEAD 读得到");
    }
}
