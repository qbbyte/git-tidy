use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// 一个待提交文件。两个状态字符原样带出去，前端要区分"新增/修改/删除/重命名"就靠它们。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingFile {
    pub path: String,
    /// 重命名/复制的来源路径，其他状态为 None
    pub from_path: Option<String>,
    /// 索引列（相对 HEAD），单字符：'M' 'A' 'D' 'R' 'C' 'U' '?' ' '
    pub index_status: String,
    /// 工作区列（相对索引）
    pub worktree_status: String,
    /// 索引里有改动，即"已暂存"
    pub staged: bool,
    pub untracked: bool,
    pub conflict: bool,
}

/// 工作区里待提交的文件。
///
/// `-z` 是关键：路径不再被八进制转义，中文文件名直接可用（§5.1 §5.3）；
/// 代价是重命名条目会多出一个 NUL 分隔的来源路径字段，解析必须吃掉它，
/// 否则后面每一条都会错位。
pub fn list(repo: &Path) -> Result<Vec<WorkingFile>, GitError> {
    let out = process::run(
        Some(repo),
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?
    .expect_success()?;
    parse(&out)
}

fn parse(raw: &str) -> Result<Vec<WorkingFile>, GitError> {
    let mut files = Vec::new();
    let mut entries = raw.split('\0');

    while let Some(entry) = entries.next() {
        if entry.is_empty() {
            continue; // 结尾的 NUL 会切出一个空尾段
        }
        let bytes = entry.as_bytes();
        // 形态校验："XY 空格 路径"，至少 4 字节且第 3 字节是空格
        if bytes.len() < 4 || bytes[2] != b' ' {
            return Err(GitError::ParseFailure {
                snippet: truncate(entry),
            });
        }
        let index_status = &entry[0..1];
        let worktree_status = &entry[1..2];
        let path = entry[3..].to_string();

        let from_path = if is_rename_pair(index_status, worktree_status) {
            entries
                .next()
                .filter(|src| !src.is_empty())
                .map(str::to_string)
        } else {
            None
        };

        let pair = format!("{index_status}{worktree_status}");
        files.push(WorkingFile {
            untracked: pair == "??",
            // '?' 也占一个非空格位，直接判"非空格"会把未跟踪当成已暂存
            staged: !matches!(index_status, " " | "?"),
            conflict: matches!(
                pair.as_str(),
                "UU" | "AA" | "DD" | "AU" | "UA" | "UD" | "DU"
            ),
            path,
            from_path,
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
        });
    }

    Ok(files)
}

fn is_rename_pair(index_status: &str, worktree_status: &str) -> bool {
    matches!(
        (index_status, worktree_status),
        ("R", _) | ("C", _) | (_, "R") | (_, "C")
    )
}

/// 暂存这些路径。`-A` 是必须的：不带它，删除掉的文件传进 `git add` 不会记录移除。
pub fn stage(repo: &Path, paths: &[String]) -> Result<Vec<WorkingFile>, GitError> {
    if paths.is_empty() {
        return list(repo);
    }
    run_paths(repo, &["add", "-A", "--"], paths)?;
    list(repo)
}

/// 取消暂存。界面只给路径，索引怎么回退由 git 决定，我们不复算。
pub fn unstage(repo: &Path, paths: &[String]) -> Result<Vec<WorkingFile>, GitError> {
    if paths.is_empty() {
        return list(repo);
    }
    // 空仓库没有 HEAD 可回落，`restore --staged` 会直接 fatal，只能请 rm --cached
    let targets = with_rename_source(repo, paths);
    if has_head(repo) {
        run_paths(repo, &["restore", "--staged", "--"], &targets)?;
    } else {
        run_paths(repo, &["rm", "--cached", "-q", "--"], &targets)?;
    }
    list(repo)
}

/// 重命名在索引里是新旧两条，只解一半会把另一半留成一条来历不明的暂存改动。
fn with_rename_source(repo: &Path, paths: &[String]) -> Vec<String> {
    let mut targets = paths.to_vec();
    let current = list(repo).unwrap_or_default();
    for file in current {
        let Some(from) = file.from_path else {
            continue;
        };
        if paths.iter().any(|path| *path == file.path) && !targets.iter().any(|t| *t == from) {
            targets.push(from);
        }
    }
    targets
}

/// 路径一律排在 `--` 之后当 pathspec：以 `-` 开头的文件名不会被读成选项，
/// 越出仓库的路径由 git 自己拒（"is outside repository"），我们不复述它的规则。
fn run_paths(repo: &Path, head: &[&str], paths: &[String]) -> Result<(), GitError> {
    let mut args = head.to_vec();
    args.extend(paths.iter().map(String::as_str));
    process::run(Some(repo), &args)?.expect_success()?;
    Ok(())
}

fn has_head(repo: &Path) -> bool {
    process::run(Some(repo), &["rev-parse", "--verify", "-q", "HEAD"])
        .map(|out| out.success)
        .unwrap_or(false)
}

fn truncate(entry: &str) -> String {
    let mut snippet: String = entry.chars().take(80).collect();
    if snippet.chars().count() < entry.chars().count() {
        snippet.push('…');
    }
    snippet
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

    fn stage_and_commit(dir: &Path, message: &str) {
        git_in(dir, &["add", "-A"]);
        git_in(
            dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
    }

    fn find<'a>(files: &'a [WorkingFile], path: &str) -> &'a WorkingFile {
        files
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("列表里没有 {path}，实际有条目：{:?}", paths_of(files)))
    }

    fn paths_of(files: &[WorkingFile]) -> Vec<String> {
        files.iter().map(|file| file.path.clone()).collect()
    }

    #[test]
    fn clean_repo_reports_nothing() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");

        assert!(list(repo.path()).expect("status").is_empty());
    }

    #[test]
    fn untracked_and_staged_and_unstaged_are_told_apart() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");

        fs::write(repo.path().join("new.txt"), "n\n").expect("write"); // 没 add
        fs::write(repo.path().join("a.txt"), "one\ntwo\n").expect("write"); // 改了但没 add

        let files = list(repo.path()).expect("status");
        let new = find(&files, "new.txt");
        assert!(new.untracked && !new.staged, "未跟踪不该算已暂存");
        assert_eq!(new.index_status, "?");

        let modified = find(&files, "a.txt");
        assert!(
            !modified.staged && !modified.untracked,
            "只改工作区还没暂存"
        );
        assert_eq!(modified.worktree_status, "M");
    }

    #[test]
    fn a_file_can_be_staged_and_dirty_at_the_same_time() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");

        fs::write(repo.path().join("a.txt"), "two\n").expect("write");
        git_in(repo.path(), &["add", "a.txt"]);
        fs::write(repo.path().join("a.txt"), "three\n").expect("write");

        let files = list(repo.path()).expect("status");
        let entry = find(&files, "a.txt");
        assert_eq!(entry.index_status, "M");
        assert_eq!(entry.worktree_status, "M");
        assert!(entry.staged, "MM：暂存区和工作区各有改动");
    }

    #[test]
    fn deletion_is_reported_as_deleted_not_modified() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");
        fs::remove_file(repo.path().join("a.txt")).expect("rm");

        let files = list(repo.path()).expect("status");
        let entry = find(&files, "a.txt");
        assert_eq!(entry.worktree_status, "D");
        assert!(!entry.staged);
    }

    #[test]
    fn chinese_path_survives_without_octal_escape() {
        let repo = init_repo();
        fs::write(repo.path().join("中文文件.txt"), "x\n").expect("write");

        let files = list(repo.path()).expect("status");
        assert_eq!(paths_of(&files), vec!["中文文件.txt".to_string()]);
    }

    #[test]
    fn rename_consumes_its_source_field_and_keeps_the_next_entry_aligned() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");

        git_in(repo.path(), &["mv", "a.txt", "b.txt"]);
        fs::write(repo.path().join("c.txt"), "n\n").expect("write");

        let files = list(repo.path()).expect("status");
        let renamed = find(&files, "b.txt");
        assert_eq!(renamed.index_status, "R", "暂存重命名应是 R");
        assert_eq!(renamed.from_path.as_deref(), Some("a.txt"));

        // 来源字段没吃掉的话，c.txt 会被当成上一条的尾部吞掉，条数就错了
        assert!(find(&files, "c.txt").untracked);
        assert_eq!(
            files.len(),
            2,
            "不该因为错位多/少条目：{:?}",
            paths_of(&files)
        );
    }

    #[test]
    fn conflicted_file_is_flagged() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "base\n").expect("write");
        stage_and_commit(repo.path(), "feat: base");

        git_in(repo.path(), &["checkout", "-q", "-b", "side"]);
        fs::write(repo.path().join("a.txt"), "side\n").expect("write");
        stage_and_commit(repo.path(), "feat: side");

        // 用 `-` 回到上一个分支，不写死 master/main：那取决于用户的 init.defaultBranch
        git_in(repo.path(), &["checkout", "-q", "-"]);
        fs::write(repo.path().join("a.txt"), "main\n").expect("write");
        stage_and_commit(repo.path(), "feat: main");

        let merged = process::run(Some(repo.path()), &["merge", "side"]).expect("spawn merge");
        assert!(
            !merged.success,
            "这次 merge 必须冲突，stdout：{}",
            merged.stdout
        );

        let files = list(repo.path()).expect("status");
        let entry = find(&files, "a.txt");
        assert!(
            entry.conflict,
            "UU 必须标成冲突，实际：{}{}",
            entry.index_status, entry.worktree_status
        );
    }

    #[test]
    fn an_entry_without_the_separator_is_a_parse_failure() {
        // 形状是 "XY 空格 路径"。第 3 字节不是空格、或整条不足 4 字节，都说明分隔符被吃了
        for bad in ["M\tx.txt", "M", "XY", "M x"] {
            match parse(bad) {
                Err(GitError::ParseFailure { snippet }) => {
                    assert_eq!(snippet, bad, "片段要能看出是哪条坏了");
                }
                other => panic!("{bad:?} 应报 ParseFailure，实际：{other:?}"),
            }
        }
    }

    #[test]
    fn trailing_nul_and_empty_output_are_not_entries() {
        assert!(parse("").expect("empty").is_empty());
        assert_eq!(parse("M  a.txt\0").expect("trailing").len(), 1);
    }

    #[test]
    fn staging_a_dirty_file_clears_the_worktree_column() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");
        fs::write(repo.path().join("a.txt"), "two\n").expect("write");

        let files = stage(repo.path(), &["a.txt".to_string()]).expect("stage");
        let entry = find(&files, "a.txt");
        assert!(entry.staged, "add 之后应算已暂存");
        assert_eq!(entry.index_status, "M");
        assert_eq!(entry.worktree_status, " ", "工作区那一半该被清空");
    }

    #[test]
    fn a_deletion_has_to_be_staged_with_add_a() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");
        fs::remove_file(repo.path().join("a.txt")).expect("rm");

        let files = stage(repo.path(), &["a.txt".to_string()]).expect("stage 删除");
        let entry = find(&files, "a.txt");
        assert!(entry.staged, "不带 -A 的话这条移除根本进不了索引");
        assert_eq!(entry.index_status, "D");
    }

    #[test]
    fn unstaged_new_file_goes_back_to_untracked() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");
        fs::write(repo.path().join("new.txt"), "n\n").expect("write");

        stage(repo.path(), &["new.txt".to_string()]).expect("stage");
        let files = unstage(repo.path(), &["new.txt".to_string()]).expect("unstage");
        let entry = find(&files, "new.txt");
        assert!(!entry.staged && entry.untracked, "取消暂存要退回未跟踪");
    }

    /// 空仓库没有 HEAD，`restore --staged` 会 fatal；这条路径必须自己找到 `rm --cached`。
    #[test]
    fn unstaging_in_an_initial_repo_works_without_head() {
        let repo = init_repo();
        fs::write(repo.path().join("first.txt"), "one\n").expect("write");

        let staged = stage(repo.path(), &["first.txt".to_string()]).expect("stage");
        assert!(find(&staged, "first.txt").staged);

        let files = unstage(repo.path(), &["first.txt".to_string()]).expect("unstage");
        let entry = find(&files, "first.txt");
        assert!(
            !entry.staged && entry.untracked,
            "实际：{}{}",
            entry.index_status,
            entry.worktree_status
        );
    }

    #[test]
    fn unstaging_a_rename_takes_the_whole_pair_out() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");
        git_in(repo.path(), &["mv", "a.txt", "b.txt"]);

        let files = unstage(repo.path(), &["b.txt".to_string()]).expect("unstage");
        assert!(
            files.iter().all(|file| !file.staged),
            "解除重命名不该留下半场暂存：{:?}",
            files
                .iter()
                .map(|file| format!("{}{} {}", file.index_status, file.worktree_status, file.path))
                .collect::<Vec<_>>()
        );
        // 旧文件回到"已跟踪、工作区里没了"，新文件回到未跟踪
        assert_eq!(find(&files, "a.txt").worktree_status, "D");
        assert!(find(&files, "b.txt").untracked);
    }

    #[test]
    fn an_empty_path_list_is_a_no_op_that_still_returns_the_status() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        stage_and_commit(repo.path(), "feat: a");
        fs::write(repo.path().join("a.txt"), "two\n").expect("write");

        let files = stage(repo.path(), &[]).expect("空列表不该跑 git");
        assert_eq!(paths_of(&files), vec!["a.txt".to_string()]);
    }

    /// 路径参数排在 `--` 后面，所以名字长得像选项也不会被 git 当成开关。
    #[test]
    fn a_path_that_looks_like_an_option_is_still_just_a_path() {
        let repo = init_repo();
        fs::write(repo.path().join("-F"), "sneaky\n").expect("write");

        let files = stage(repo.path(), &["-F".to_string()]).expect("stage");
        let entry = find(&files, "-F");
        assert!(entry.staged, "`-F` 该被当成文件名暂存，实际：{:?}", paths_of(&files));
    }
}
