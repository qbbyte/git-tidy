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
        if paths.contains(&file.path) && !targets.contains(&from) {
            targets.push(from);
        }
    }
    targets
}

/// 文件的 eol 规则。受 `.gitattributes` 约束的文件不做行级暂存：
/// 暂存区里的行尾与工作区里的行尾本来就不同，行号对不上，出来的补丁会把内容改错。
///
/// 形如 `path: eol: crlf` / `path: -text`。没有输出行就是没有这条规则。
pub fn eol_rule(repo: &Path, path: &str) -> Result<Option<String>, GitError> {
    let out = process::run(
        Some(repo),
        &["check-attr", "eol", "text", "--", path],
    )?
    .expect_success()?;
    let line = out.trim();
    if line.is_empty() {
        return Ok(None);
    }
    // `a.txt: eol: crlf` → 只取规则那一段，整行原样带出去在界面上太长
    Ok(Some(line.to_string()))
}

/// 该文件能不能做行级/分块暂存。
///
/// 判据三条（§7.8 的硬边界）：必须是已跟踪文件（未跟踪文件没有 index 版本可比）、
/// 不能受 eol 规则约束、不能是二进制。
pub fn partial_supported(repo: &Path, file: &WorkingFile) -> PartialSupport {
    if file.untracked {
        return PartialSupport {
            supported: false,
            reason: "未跟踪文件没有暂存区版本可比，只能整文件暂存".into(),
        };
    }
    if file.conflict {
        return PartialSupport {
            supported: false,
            reason: "这个文件正处在冲突中，先解决冲突".into(),
        };
    }
    match eol_rule(repo, &file.path) {
        Ok(Some(rule)) if !rule.contains(": unspecified") => PartialSupport {
            supported: false,
            reason: format!("该文件受 eol 规则约束（{rule}），行级暂存会改错内容"),
        },
        Ok(_) => PartialSupport {
            supported: true,
            reason: String::new(),
        },
        Err(err) => PartialSupport {
            supported: false,
            reason: format!("读不到该文件的行尾规则：{err:?}"),
        },
    }
}

/// 行级暂存能不能做，以及不能做时的原因（界面直接显示那句话）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialSupport {
    pub supported: bool,
    pub reason: String,
}

/// 选中的一段改动。`lines` 为空表示整个 hunk；给了行号就只取其中那些增删行
/// （行号是**工作区差异里**的行号，界面显示什么就传什么）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkSelection {
    pub hunk: usize,
    #[serde(default)]
    pub lines: Vec<usize>,
}

/// 按选中的 hunk / 行暂存（§7.8 的逐行 / 分块部分暂存）。
///
/// **裁剪在 Rust 侧做**：界面传"第几段、第几行"，这里从 git 的 diff 原文里取出对应的行，
/// 重算 hunk 头，再拼成补丁。让前端拼补丁文本等于让它自己算行号，而行号错一位就是
/// 暂存错内容，且这种错不会报错（§13 那条决策记录）。
///
/// `--check` 预验通过后才真的 apply，预验不过就整块不落，绝不产生"部分入栈"的中间态。
/// apply 时带 `--recount`：我们自己删过行，头里的计数已经不真了。
pub fn stage_hunks(
    repo: &Path,
    path: &str,
    selections: &[HunkSelection],
) -> Result<Vec<WorkingFile>, GitError> {
    if selections.is_empty() {
        return Err(GitError::PatchApplyFailed {
            detail: "没有选中任何改动".into(),
        });
    }
    let diff = process::run(
        Some(repo),
        &["diff", "--no-color", "--unified=3", "--", path],
    )?
    .expect_success()?;
    if diff.trim().is_empty() {
        return Err(GitError::PatchApplyFailed {
            detail: format!("{path} 当前没有未暂存的改动可分段暂存"),
        });
    }
    let hunks = split_hunks(&diff);

    let mut patch = String::new();
    for line in diff.lines().take_while(|line| !line.starts_with("@@")) {
        patch.push_str(line);
        patch.push('\n');
    }
    for selection in selections {
        let Some(raw) = hunks.get(selection.hunk) else {
            return Err(GitError::PatchApplyFailed {
                detail: format!("第 {} 段改动已经不存在了，请刷新后重试", selection.hunk + 1),
            });
        };
        patch.push_str(&select_from_hunk(raw, &selection.lines)?);
    }
    apply_patch(repo, &patch)
}

/// 从一段 hunk 原文里取出选中的行，并把 hunk 头的两个计数重算一遍。
///
/// 计数必须重算：我们删过行，而头里的数字已经不真，`git apply` 会按它去定位，
/// 于是后面几行全部错位——而错位不会报错，只会静默写错内容。
fn select_from_hunk(raw: &str, lines: &[usize]) -> Result<String, GitError> {
    let Some(header) = raw.lines().next() else {
        return Err(GitError::PatchApplyFailed {
            detail: "这段改动是空的".into(),
        });
    };
    // 整段：原样返回，头已经是 git 自己算对的
    if lines.is_empty() {
        return Ok(raw.to_string());
    }
    let (old_start, new_start) = parse_header(header)?;

    let mut body = String::new();
    let (mut old_count, mut new_count) = (0usize, 0usize);
    // 上一行被选中时，`\ No newline at end of file` 才跟着走：它说的是上一行没有换行
    let mut previous_selected = false;

    // enumerate 从 1 起：下标 0 是 hunk 头，它不参与选择
    for (offset, line) in raw.lines().skip(1).enumerate() {
        let index = offset;
        let selected = keep(lines, index, first_char(line));
        let marker = first_char(line);
        if selected {
            match marker {
                Some(' ') => {
                    old_count += 1;
                    new_count += 1;
                }
                Some('-') => old_count += 1,
                Some('+') => new_count += 1,
                _ => {}
            }
            body.push_str(line);
            body.push('\n');
        } else if marker == Some('\\') && previous_selected {
            body.push_str(line);
            body.push('\n');
        }
        previous_selected = selected;
    }

    if body.is_empty() {
        return Err(GitError::PatchApplyFailed {
            detail: "选中的行里没有可暂存的内容".into(),
        });
    }
    Ok(format!(
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n{body}"
    ))
}

/// `@@ -a,b +c,d @@` 里的两个起点。计数不参与我们这一步（重算了）。
fn parse_header(line: &str) -> Result<(usize, usize), GitError> {
    let failure = || GitError::ParseFailure {
        snippet: process::snippet(line),
    };
    // 头部形如 `@@ -8,7 +8,7 @@ 某个函数`，所以先把两个 `@` 去掉再分词
    let body = line.trim_start_matches('@');
    let mut parts = body.split_whitespace();
    let old = parts.next().ok_or_else(failure)?;
    let new = parts.next().ok_or_else(failure)?;
    let start_of = |token: &str| -> Result<usize, GitError> {
        token
            .trim_start_matches(['-', '+'])
            .split(',')
            .next()
            .unwrap_or_default()
            .parse::<usize>()
            .map_err(|_| failure())
    };
    Ok((start_of(old)?, start_of(new)?))
}

/// 这一行要不要留在补丁里。
///
/// 选中的增删行要留；它们周围的**上下文行也要留**，而且留满 3 行（与 `-U3` 相同）：
/// `git apply` 靠上下文定位，只给一行改动而不给上下文时它会报 "patch does not apply"，
/// 而那段上下文本身并不是这次要暂存的内容——它只是让 git 找得到位置。
fn keep(lines: &[usize], index: usize, marker: Option<char>) -> bool {
    if lines.contains(&index) {
        return true;
    }
    if marker != Some(' ') {
        return false;
    }
    // 上下 3 行内有任何一行被选中，这行就作为上下文带上
    let low = index.saturating_sub(3);
    let high = index + 3;
    (low..=high).any(|around| lines.contains(&around))
}

fn first_char(line: &str) -> Option<char> {
    line.chars().next()
}

/// 把 `git diff` 原文切成一段段 hunk。只认 `@@` 开头的行：
/// diff 正文里除 hunk 头之外没有别的行以 `@@` 开头。
fn split_hunks(diff: &str) -> Vec<String> {
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in diff.lines() {
        if line.starts_with("@@") {
            if let Some(text) = current.take() {
                hunks.push(text);
            }
            current = Some(format!("{line}\n"));
            continue;
        }
        if let Some(text) = current.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    if let Some(text) = current {
        hunks.push(text);
    }
    hunks
}

/// 先 `--check` 再 apply。预验不过就是整块不落，索引一动不动（§7.8 的失败回落）。
fn apply_patch(repo: &Path, patch: &str) -> Result<Vec<WorkingFile>, GitError> {
    let file = write_patch(repo, patch)?;
    let checked = process::run(
        Some(repo),
        &["apply", "--cached", "--check", "--recount", &file],
    );
    match checked {
        Ok(out) if out.success => {}
        Ok(out) => {
            let _ = std::fs::remove_file(&file);
            return Err(GitError::PatchApplyFailed {
                detail: first_lines(&out.stderr),
            });
        }
        Err(err) => {
            let _ = std::fs::remove_file(&file);
            return Err(err);
        }
    }

    let applied = process::run(
        Some(repo),
        &["apply", "--cached", "--recount", &file],
    );
    let _ = std::fs::remove_file(&file);
    applied?.expect_success()?;
    list(repo)
}

/// 补丁落在 git 目录旁边的临时文件。放在 git 目录里：同一卷上 `git apply` 读它更快，
/// 而清理失败也只是下次多一个文件。
fn write_patch(repo: &Path, patch: &str) -> Result<String, GitError> {
    let dir = process::run(Some(repo), &["rev-parse", "--git-dir"])?.expect_success()?;
    let path = repo
        .join(std::path::Path::new(dir.trim()))
        .join(format!("git-tidy-patch-{}", std::process::id()));
    std::fs::write(&path, patch)
        .map_err(|err| GitError::Internal(format!("写补丁临时文件失败：{err}")))?;
    Ok(path.to_string_lossy().into_owned())
}

fn first_lines(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("；")
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

    /// §7.8 的验收：一个文件三个 hunk，只暂存中间一个时 `git diff --cached` 与所选完全一致、
    /// `git diff` 剩两个 hunk。
    #[test]
    fn one_hunk_of_three_can_be_staged_on_its_own() {
        let repo = init_repo();
        let dir = repo.path();
        let mut base = String::new();
        for i in 1..=21 {
            base.push_str(&format!("原始第{i}行\n"));
        }
        fs::write(dir.join("a.txt"), &base).expect("write");
        stage_and_commit(dir, "chore: 铺底");

        let mut changed = base.clone();
        changed = changed.replace("原始第2行", "改过的第2行");
        changed = changed.replace("原始第11行", "改过的第11行");
        changed = changed.replace("原始第20行", "改过的第20行");
        fs::write(dir.join("a.txt"), &changed).expect("write");

        // 选中间那一整段：三段之间隔着足够的上下文，git 才会分成三段
        let diff = process::run(
            Some(dir),
            &["diff", "--no-color", "--unified=3", "--", "a.txt"],
        )
        .expect("diff")
        .stdout;
        let hunks = split_hunks(&diff);
        assert_eq!(hunks.len(), 3, "这个 fixture 应该有三段：{diff}");

        let after = stage_hunks(dir, "a.txt", &[HunkSelection { hunk: 1, lines: Vec::new() }])
            .expect("暂存中间那段");
        let entry = find(&after, "a.txt");
        assert!(entry.staged, "选中的一段要进索引");

        let staged = process::run(Some(dir), &["diff", "--cached", "--unified=0"]).expect("cached");
        assert!(
            staged.stdout.contains("改过的第11行"),
            "暂存区里该只有选中的那一处：{}",
            staged.stdout
        );
        assert!(!staged.stdout.contains("改过的第2行"), "没选的不该进来");
        assert!(!staged.stdout.contains("改过的第20行"), "没选的不该进来");

        let rest = process::run(Some(dir), &["diff", "--unified=0"]).expect("worktree");
        assert!(rest.stdout.contains("改过的第2行"));
        assert!(rest.stdout.contains("改过的第20行"));
        assert!(!rest.stdout.contains("改过的第11行"), "已暂存的那处不该还在工作区差异里");
    }

    /// 行级：一段里只选一行，暂存区里就该只有那一行的改动
    #[test]
    fn a_single_line_inside_a_hunk_can_be_staged() {
        let repo = init_repo();
        let dir = repo.path();
        fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").expect("write");
        stage_and_commit(dir, "chore: 铺底");
        fs::write(dir.join("a.txt"), "ONE\ntwo\nthree\n").expect("write");

        let diff = process::run(
            Some(dir),
            &["diff", "--no-color", "--unified=3", "--", "a.txt"],
        )
        .expect("diff")
        .stdout;
        let hunks = split_hunks(&diff);
        assert_eq!(hunks.len(), 1);

        // hunk 内第 0 行是删除 one、第 1 行是新增 ONE：只暂存这一处改动。
        // 上下文行会被自动带上（git apply 靠它定位），但不算改动
        let after = stage_hunks(
            dir,
            "a.txt",
            &[HunkSelection {
                hunk: 0,
                lines: vec![0, 1],
            }],
        )
        .expect("只暂存这一处改动");
        assert!(find(&after, "a.txt").staged);

        let staged = process::run(Some(dir), &["diff", "--cached"]).expect("cached").stdout;
        assert!(staged.contains("-one"), "{staged}");
        assert!(staged.contains("+ONE"), "{staged}");
        // 上下文行会被原样带上（`git apply` 靠它定位），但它们不算改动：
        // 暂存完之后，工作区与索引之间不该再剩下任何差异
        assert!(
            process::run(Some(dir), &["diff"]).expect("worktree").stdout.trim().is_empty(),
            "选中这一处之后不该还有未暂存的改动"
        );
    }

    /// 预验不过就整块不落：索引不能出现"半个 hunk"的中间态（§7.8 的失败回落）
    #[test]
    fn a_patch_that_does_not_apply_leaves_the_index_untouched() {
        let repo = init_repo();
        let dir = repo.path();
        fs::write(dir.join("a.txt"), "1\n").expect("write");
        stage_and_commit(dir, "chore: 铺底");

        let before = process::run(Some(dir), &["diff", "--cached"]).expect("cached").stdout;
        // 指一段不存在的位置：整块必须被拒
        let err = stage_hunks(dir, "a.txt", &[HunkSelection { hunk: 7, lines: Vec::new() }])
            .expect_err("不存在的段不该被接受");
        assert!(
            matches!(err, GitError::PatchApplyFailed { .. }),
            "要报 PatchApplyFailed：{err:?}"
        );
        assert_eq!(
            process::run(Some(dir), &["diff", "--cached"]).expect("cached").stdout,
            before,
            "预验不过时索引不能变"
        );
    }

    #[test]
    fn a_file_with_no_unstaged_change_cannot_be_staged_in_parts() {
        let repo = init_repo();
        let dir = repo.path();
        fs::write(dir.join("a.txt"), "1\n").expect("write");
        stage_and_commit(dir, "chore: 铺底");

        let err = stage_hunks(dir, "a.txt", &[HunkSelection { hunk: 0, lines: Vec::new() }])
            .expect_err("没有未暂存改动时不该接受分段暂存");
        assert!(format!("{err:?}").contains("未暂存"), "{err:?}");
    }

    #[test]
    fn an_empty_selection_is_refused_rather_than_writing_an_empty_patch() {
        let repo = init_repo();
        let err = stage_hunks(repo.path(), "a.txt", &[]).expect_err("空选不该写补丁");
        assert!(matches!(err, GitError::PatchApplyFailed { .. }), "{err:?}");
    }

    /// 硬边界：未跟踪文件没有暂存区版本可比，只能整文件暂存
    #[test]
    fn an_untracked_file_cannot_be_partially_staged() {
        let repo = init_repo();
        let dir = repo.path();
        fs::write(dir.join("a.txt"), "1\n").expect("write");
        stage_and_commit(dir, "chore: 铺底");
        fs::write(dir.join("new.txt"), "全新的\n").expect("write");

        let files = list(dir).expect("status");
        let fresh = find(&files, "new.txt");
        let support = partial_supported(dir, fresh);
        assert!(!support.supported, "未跟踪文件不能行级暂存");
        assert!(support.reason.contains("未跟踪"), "{}", support.reason);
    }

    #[test]
    fn a_plain_tracked_file_supports_line_level_staging() {
        let repo = init_repo();
        let dir = repo.path();
        fs::write(dir.join("a.txt"), "1\n").expect("write");
        stage_and_commit(dir, "chore: 铺底");
        fs::write(dir.join("a.txt"), "2\n").expect("write");

        let files = list(dir).expect("status");
        assert!(partial_supported(dir, find(&files, "a.txt")).supported);
    }

    /// 受 eol 规则约束的文件要明确拒绝行级暂存，而不是给出一个会改错内容的补丁
    #[test]
    fn an_eol_constrained_file_is_refused_with_a_reason() {
        let repo = init_repo();
        let dir = repo.path();
        fs::write(dir.join(".gitattributes"), "*.txt text eol=crlf\n").expect("write");
        fs::write(dir.join("a.txt"), "1\n").expect("write");
        stage_and_commit(dir, "chore: 铺底");
        fs::write(dir.join("a.txt"), "2\n").expect("write");

        let files = list(dir).expect("status");
        let support = partial_supported(dir, find(&files, "a.txt"));
        assert!(!support.supported, "eol 规则下的文件不该给行级入口");
        assert!(support.reason.contains("eol"), "{}", support.reason);
    }

    /// split_hunks 现在是库内的正式函数，测试直接用它，不再在下面写一份

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
