//! 交互式改写（§7.14）。
//!
//! **不用 `git rebase -i`**。`-i` 要求 git 通过 `GIT_SEQUENCE_EDITOR` 反向调用我们，
//! 在 Windows 上那意味着要做一个既是客户端又是编辑器、还得单实例互斥的进程；中断态续跑
//! 又依赖 git 自己的 todo 状态目录。崩溃面大、可测性差。
//!
//! 改为自驱：todo 由界面给出，在**临时分支**上按序执行，每一步是一次显式的
//! `cherry-pick` / `commit --amend` / `reset --soft`。这样每一步都是我们自己发起的
//! 子进程，任何一步失败都知道停在了哪里；tree 校验和还原 ref 兜底。
//!
//! 代价是冲突续跑要自己实现——那本来就是 §7.13 冲突解决器的活，两处共用。

use std::collections::HashMap;
use std::path::Path;

use crate::error::GitError;
use crate::git::process;

/// 临时分支前缀。留着的前缀让"上次改写停在哪"在 `git branch` 里一眼看得见。
const TEMP_PREFIX: &str = "git-tidy/rewrite";

/// todo 超过这个条数就在界面上提示分段。改写是每条一次进程，耗时随条数线性涨，
/// 与其让它跑一分钟没有反馈，不如先告诉用户会有多慢。
pub const LARGE_TODO: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// 原样保留
    Pick,
    /// 保留但换提交信息
    Reword,
    /// 并进前一条，提交信息追加
    Squash,
    /// 并进前一条，丢弃自己的提交信息
    Fixup,
    /// 丢弃
    Drop,
}

impl Action {
    /// Drop 是唯一会改变最终 tree 的动作——它真的扔掉了改动。
    /// 其余动作只动顺序与信息，tree 必须与改写前完全一致（§4 步骤 4）。
    pub fn changes_tree(self) -> bool {
        matches!(self, Action::Drop)
    }

    fn merges_into_previous(self) -> bool {
        matches!(self, Action::Squash | Action::Fixup)
    }

    fn label(self) -> &'static str {
        match self {
            Action::Pick => "pick",
            Action::Reword => "reword",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
        }
    }
}

/// todo 里的一条。`message` 只有 Reword 用得上。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub sha: String,
    pub action: Action,
    #[serde(default)]
    pub message: Option<String>,
}

impl TodoItem {
    /// 只给测试用：线上 todo 从界面反序列化而来，不走这个构造器。
    #[cfg(test)]
    pub fn pick(sha: &str) -> TodoItem {
        TodoItem { sha: sha.to_string(), action: Action::Pick, message: None }
    }
}

/// 界面初始化 todo 用的一行：`base..head` 区间里的每条提交与它的作者、原日期。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub sha: String,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    /// ISO 8601。压缩时要原样还回去（§7.14 要求保留作者与日期）
    pub authored_at: String,
}

/// `base..head` 区间里的提交，旧到新。界面上默认就按这个顺序摆。
pub fn plan(path: &Path, base: &str, head: &str) -> Result<Vec<PlanEntry>, GitError> {
    let raw = process::run(
        Some(path),
        &[
            "log",
            "--reverse",
            "--no-merges",
            "--format=%H\x1f%an\x1f%ae\x1f%aI\x1f%s",
            &format!("{base}..{head}"),
        ],
    )?
    .expect_success()?;

    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\u{1f}').collect();
            PlanEntry {
                sha: f.first().copied().unwrap_or_default().trim().to_string(),
                subject: f.get(4).copied().unwrap_or_default().to_string(),
                author_name: f.get(1).copied().unwrap_or_default().to_string(),
                author_email: f.get(2).copied().unwrap_or_default().to_string(),
                authored_at: f.get(3).copied().unwrap_or_default().to_string(),
            }
        })
        .collect())
}

/// 一条 todo 的执行结果。压缩与改信息之后提交号一定变，所以每个都要报新的。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReport {
    pub sha: String,
    pub action: Action,
    pub produced: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteReport {
    pub old_head: String,
    pub new_head: String,
    /// 成功时已删掉；失败时留在仓库里供诊断。
    pub temp_branch: String,
    pub steps: Vec<StepReport>,
    /// 停在哪个提交上（冲突这类半完成状态）。
    pub stopped_at: Option<String>,
}

/// 在临时分支上按序执行 todo。**不移动原分支**，那由 `promote` 做。
///
/// `on_step` 每推进一步回调一次，界面用它报进度：几十条提交的改写不该是个没有反馈的白屏。
///
/// 失败时**不做回滚**——回滚是 `write_guard` 的第 5 步，回滚会把整个仓库拉回改写前；
/// 这里只保证原分支的指针没被碰过，临时分支留着，用户能进去看停在了哪。
/// 两处各回滚一次就会出现"日志说成功、仓库已回滚"那种最难查的状态。
pub fn run(
    path: &Path,
    base: &str,
    old_head: &str,
    todo: &[TodoItem],
    on_step: &mut dyn FnMut(&str),
) -> Result<RewriteReport, GitError> {
    validate(todo)?;
    let groups = Group::split(todo)?;
    let subjects: HashMap<String, String> = plan(path, base, old_head)?
        .into_iter()
        .map(|entry| (entry.sha, entry.subject))
        .collect();

    let temp_branch = format!("{TEMP_PREFIX}-{}", short(old_head));
    // 上一次失败留下的同名临时分支要能直接覆盖，否则第二次改写起手就报分支已存在
    let _ = process::run(Some(path), &["branch", "-D", &temp_branch]);
    process::run(Some(path), &["checkout", "-q", "-B", &temp_branch, base])?.expect_success()?;

    let mut steps: Vec<StepReport> = Vec::new();
    for group in &groups {
        if let Err(err) = apply_group(path, group, on_step) {
            // 清掉冲突态再退：临时分支还指着停住的那一步，仓库本身已经能改写前的样子收场
            let _ = process::run(Some(path), &["cherry-pick", "--abort"]);
            let stopped_at = group.first().map(|item| item.sha.clone());
            return Err(annotate(err, stopped_at.as_deref(), &subjects, &temp_branch));
        }
        steps.push(StepReport {
            sha: group.first().expect("split 不产出空组").sha.clone(),
            action: group.first().expect("split 不产出空组").action,
            produced: rev_parse(path, "HEAD")?,
        });
    }

    Ok(RewriteReport {
        old_head: old_head.to_string(),
        new_head: rev_parse(path, "HEAD")?,
        temp_branch,
        steps,
        stopped_at: None,
    })
}

/// 把原分支指到 `new_head`、切回去，然后删掉临时分支。
///
/// 单独一个函数，因为它是**唯一**能移动原分支的地方：`run` 里的每一步都只动临时分支。
/// `update-ref` 带旧值做乐观锁，这期间要是有人动了原分支就失败，而不是无声覆盖。
pub fn promote(
    path: &Path,
    branch: &str,
    old_head: &str,
    new_head: &str,
    temp_branch: &str,
) -> Result<(), GitError> {
    process::run(
        Some(path),
        &["update-ref", &format!("refs/heads/{branch}"), new_head, old_head],
    )?
    .expect_success()?;
    // 先挪指针再切过去：顺序反了会 checkout 到一个尚未指向新提交的分支上
    process::run(Some(path), &["checkout", "--force", "-q", branch])?.expect_success()?;
    process::run(Some(path), &["branch", "-D", temp_branch])?.expect_success()?;
    Ok(())
}

/// 树上任何一步都可能非法，而这类失败**不能**靠回滚兜底，所以只能在开工前拦下来。
///
/// 唯一非法的形态是打头的 squash/fixup：它们要并进前一条，前面没有可并的对象。
/// reword 不在此列——它总是自己起一组（改这一条的信息），后面再跟 squash 就是
/// "先改信息再并进去"，与 git 的 todo 语义一致，也是最常用的那条命令。
pub fn validate(todo: &[TodoItem]) -> Result<(), GitError> {
    if todo.is_empty() {
        return Err(GitError::ParseFailure {
            snippet: "改写区间里一条提交都没有".into(),
        });
    }
    let mut has_leader = false;
    for item in todo {
        if item.action.merges_into_previous() && !has_leader {
            return Err(GitError::ParseFailure {
                snippet: format!(
                    "{} 是 {}，但前面没有可并进的提交",
                    short(&item.sha),
                    item.action.label()
                ),
            });
        }
        // drop 不给下一条留下可并的对象：它是被丢掉的那条，并进去等于既丢又留
        has_leader = item.action != Action::Drop && !item.action.merges_into_previous();
    }
    Ok(())
}

/// 一次输出的构成：一条 pick/reword 打头，后面跟若干 squash/fixup 并进它。
///
/// 与 git 的 todo 语义一致——squash 并进的是**紧邻的前一条**，所以连续两条 pick 是两个组，
/// 不能拿"连续非 drop"当一个组，否则两条普通提交会被误压成一条。
struct Group<'a> {
    items: Vec<&'a TodoItem>,
}

impl<'a> Group<'a> {
    fn first(&self) -> Option<&'a TodoItem> {
        self.items.first().copied()
    }

    fn split(todo: &'a [TodoItem]) -> Result<Vec<Group<'a>>, GitError> {
        let mut groups: Vec<Group<'a>> = Vec::new();
        for item in todo {
            if item.action == Action::Drop {
                // drop 同时结束上一组：它之后的 squash 没有并入对象，validate 已挡掉
                continue;
            }
            if item.action.merges_into_previous() {
                let Some(last) = groups.last_mut() else {
                    return Err(GitError::ParseFailure {
                        snippet: format!("{} 是 {}，但前面没有可并进的提交", short(&item.sha), item.action.label()),
                    });
                };
                last.items.push(item);
            } else {
                groups.push(Group { items: vec![item] });
            }
        }
        Ok(groups)
    }
}

fn apply_group(
    path: &Path,
    group: &Group<'_>,
    on_step: &mut dyn FnMut(&str),
) -> Result<(), GitError> {
    let items = &group.items;
    let leader = items[0];
    let squashing = items.len() > 1;

    for item in items {
        on_step(&format!("{} {}", item.action.label(), short(&item.sha)));
        // keep-redundant-commits：内容已在树里的提交照样落一条，否则改写后提交数量会变，
        // 用户看到的历史长度和界面里的 todo 对不上
        process::run(
            Some(path),
            &["cherry-pick", "--allow-empty", "--keep-redundant-commits", &item.sha],
        )?
        .expect_success()?;
    }

    if squashing {
        // 连着 pick 完再 soft reset 回本组第一条之前，把它们融成一条
        process::run(Some(path), &["reset", "--soft", "-q", &format!("{}^1", leader.sha)])?
            .expect_success()?;
        let message = compose_message(path, items)?;
        let author = format!(
            "{} <{}>",
            format_field(path, &leader.sha, "%an")?,
            format_field(path, &leader.sha, "%ae")?
        );
        let date = format_field(path, &leader.sha, "%aI")?;
        process::run(
            Some(path),
            &["commit", "-q", "--author", &author, "--date", &date, "-m", &message],
        )?
        .expect_success()?;
    } else if leader.action == Action::Reword {
        let message = leader.message.clone().unwrap_or_default();
        // amend 默认保留作者与作者日期，这里只换信息
        process::run(Some(path), &["commit", "--amend", "-q", "-m", &message])?.expect_success()?;
    }
    Ok(())
}

/// 合成一组的提交信息。语义与 git 自己的 squash/fixup 一致：
/// fixup 丢掉自己的信息，squash 追加（空一行分隔），reword 用界面给的新信息起头。
fn compose_message(path: &Path, items: &[&TodoItem]) -> Result<String, GitError> {
    let mut parts: Vec<String> = Vec::new();
    for item in items {
        let text = match item.action {
            Action::Fixup => continue,
            Action::Reword => item.message.clone().unwrap_or_default(),
            _ => format_field(path, &item.sha, "%B")?,
        };
        let text = text.trim_end().to_string();
        if !text.is_empty() {
            parts.push(text);
        }
    }
    Ok(parts.join("\n\n"))
}

fn format_field(path: &Path, sha: &str, format: &str) -> Result<String, GitError> {
    Ok(process::run(Some(path), &["log", "-1", &format!("--format={format}"), sha])?
        .expect_success()?
        .trim()
        .to_string())
}

fn rev_parse(path: &Path, rev: &str) -> Result<String, GitError> {
    Ok(process::run(Some(path), &["rev-parse", "--verify", rev])?
        .expect_success()?
        .trim()
        .to_string())
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// 失败信息要带上"停在哪、临时分支叫什么"，否则用户拿到一句 `cherry-pick failed` 无从下手，
/// 而原样再跑一遍正是这个功能最危险的地方。
fn annotate(
    err: GitError,
    stopped_at: Option<&str>,
    subjects: &HashMap<String, String>,
    temp_branch: &str,
) -> GitError {
    let Some(sha) = stopped_at else {
        return err;
    };
    let subject = subjects.get(sha).map(String::as_str).unwrap_or("");
    let at = if subject.is_empty() {
        short(sha).to_string()
    } else {
        format!("{} {}", short(sha), subject)
    };
    let tail = format!("改写停在 {at}；临时分支 {temp_branch} 已保留，可进去查看现场");
    match err {
        GitError::GitFailed { stderr } => GitError::GitFailed { stderr: format!("{tail}：{stderr}") },
        GitError::PatchApplyFailed { detail } => GitError::PatchApplyFailed { detail: format!("{tail}：{detail}") },
        other => other,
    }
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
        git_in(dir.path(), &["config", "core.autocrlf", "false"]);
        dir
    }

    /// 三个线性提交，每条只动自己的文件。返回 (临时目录, 最早一条提交的父提交)
    ///
    /// base 必须是区间之外的那一条，所以这里先铺一个初始提交再写三条：
    /// 否则 base 会落到第一条提交里，区间就少一条。
    fn three_commits() -> (tempfile::TempDir, String) {
        let dir = repo();
        std::fs::write(dir.path().join("seed.txt"), "0\n").expect("write");
        git_in(dir.path(), &["add", "-A"]);
        git_in(dir.path(), &["commit", "-q", "-m", "chore: 初始提交"]);
        for (name, text) in [("a.txt", "1\n"), ("b.txt", "2\n"), ("c.txt", "3\n")] {
            std::fs::write(dir.path().join(name), text).expect("write");
            git_in(dir.path(), &["add", "-A"]);
            git_in(dir.path(), &["commit", "-q", "-m", &format!("feat: {name}")]);
        }
        let base = rev_parse(dir.path(), "HEAD~3").expect("base");
        (dir, base)
    }

    fn head_of(dir: &Path) -> String {
        rev_parse(dir, "HEAD").expect("head")
    }

    fn tree_of(dir: &Path, rev: &str) -> String {
        rev_parse(dir, &format!("{rev}^{{tree}}")).expect("tree")
    }

    #[test]
    fn plan_lists_the_range_oldest_first_with_authorship() {
        let (dir, base) = three_commits();
        let entries = plan(dir.path(), &base, &head_of(dir.path())).expect("plan");

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].subject, "feat: a.txt");
        assert_eq!(entries[2].subject, "feat: c.txt");
        assert_eq!(entries[0].author_name, "t");
        assert_eq!(entries[0].author_email, "t@example.com");
        assert!(!entries[0].authored_at.is_empty());
    }

    #[test]
    fn squashing_two_commits_keeps_the_tree_and_the_original_author() {
        let (dir, base) = three_commits();
        let head = head_of(dir.path());
        let tree_before = tree_of(dir.path(), &head);
        let entries = plan(dir.path(), &base, &head).expect("plan");

        let todo = vec![
            TodoItem::pick(&entries[0].sha),
            TodoItem { sha: entries[1].sha.clone(), action: Action::Squash, message: None },
            TodoItem::pick(&entries[2].sha),
        ];
        let report = run(dir.path(), &base, &head, &todo, &mut |_| {}).expect("改写");

        assert_eq!(tree_before, tree_of(dir.path(), &report.new_head), "压缩不得改变最终 tree");

        let count = process::run(Some(dir.path()), &["rev-list", "--count", &format!("{base}..HEAD")])
            .expect("count")
            .stdout
            .trim()
            .to_string();
        assert_eq!(count, "2", "两条压成一条后区间里应该剩两条提交");

        // 压缩后的那条提交是 HEAD~1（后面还 pick 了第三条）
        let message = process::run(Some(dir.path()), &["log", "-1", "--format=%B", "HEAD~1"])
            .expect("message")
            .stdout;
        assert!(message.contains("feat: a.txt") && message.contains("feat: b.txt"), "{message}");

        let author = process::run(
            Some(dir.path()),
            &["log", "-1", "--format=%an <%ae>", "HEAD~1"],
        )
        .expect("author")
        .stdout;
        assert_eq!(author.trim(), "t <t@example.com>", "压缩要保留原作者");
    }

    #[test]
    fn consecutive_picks_are_not_accidentally_squashed() {
        let (dir, base) = three_commits();
        let head = head_of(dir.path());
        let entries = plan(dir.path(), &base, &head).expect("plan");

        let todo: Vec<TodoItem> = entries.iter().map(|e| TodoItem::pick(&e.sha)).collect();
        run(dir.path(), &base, &head, &todo, &mut |_| {}).expect("改写");

        let count = process::run(Some(dir.path()), &["rev-list", "--count", &format!("{base}..HEAD")])
            .expect("count")
            .stdout
            .trim()
            .to_string();
        assert_eq!(count, "3", "三条普通 pick 之后仍该是三条提交");
    }

    #[test]
    fn fixup_discards_its_own_message() {
        let (dir, base) = three_commits();
        let head = head_of(dir.path());
        let entries = plan(dir.path(), &base, &head).expect("plan");

        let todo = vec![
            TodoItem::pick(&entries[0].sha),
            TodoItem { sha: entries[1].sha.clone(), action: Action::Fixup, message: None },
        ];
        run(dir.path(), &base, &head, &todo, &mut |_| {}).expect("改写");

        let message = process::run(Some(dir.path()), &["log", "-1", "--format=%B", "HEAD"])
            .expect("message")
            .stdout;
        assert!(message.contains("feat: a.txt"), "{message}");
        assert!(!message.contains("feat: b.txt"), "fixup 的信息不该出现：{message}");
    }

    #[test]
    fn reword_replaces_the_message_and_keeps_the_author() {
        let (dir, base) = three_commits();
        let head = head_of(dir.path());
        let entries = plan(dir.path(), &base, &head).expect("plan");
        let tree_before = tree_of(dir.path(), &head);

        // todo 要把整段都写出来：只 reword 最后一条等于把前两条丢了，tree 必然变
        let mut todo: Vec<TodoItem> = entries.iter().map(|e| TodoItem::pick(&e.sha)).collect();
        let last = todo.len() - 1;
        todo[last] = TodoItem {
            sha: entries[last].sha.clone(),
            action: Action::Reword,
            message: Some("feat(spec): 改过的标题\n\n正文".into()),
        };
        let report = run(dir.path(), &base, &head, &todo, &mut |_| {}).expect("改写");

        let message = process::run(Some(dir.path()), &["log", "-1", "--format=%B", &report.new_head])
            .expect("message")
            .stdout;
        assert!(message.contains("feat(spec): 改过的标题"), "{message}");
        assert!(message.contains("正文"), "{message}");
        assert_eq!(tree_before, tree_of(dir.path(), &report.new_head));

        let author = process::run(
            Some(dir.path()),
            &["log", "-1", "--format=%an <%ae>", &report.new_head],
        )
        .expect("author")
        .stdout;
        assert_eq!(author.trim(), "t <t@example.com>", "amend 要保留作者");
    }

    #[test]
    fn reordering_reproduces_the_original_tree() {
        let (dir, base) = three_commits();
        let head = head_of(dir.path());
        let tree_before = tree_of(dir.path(), &head);
        let mut todo: Vec<TodoItem> = plan(dir.path(), &base, &head)
            .expect("plan")
            .iter()
            .map(|e| TodoItem::pick(&e.sha))
            .collect();
        todo.reverse();

        let report = run(dir.path(), &base, &head, &todo, &mut |_| {}).expect("改写");
        assert_eq!(tree_before, tree_of(dir.path(), &report.new_head));

        let subjects = process::run(Some(dir.path()), &["log", "--format=%s", "-3"])
            .expect("log")
            .stdout;
        assert!(
            subjects.starts_with("feat: a.txt"),
            "倒序重排之后最老的那条变成最新的，实际：{subjects}"
        );
    }

    #[test]
    fn promote_moves_the_branch_and_drops_the_temp_branch() {
        let (dir, base) = three_commits();
        let head = head_of(dir.path());
        let entries = plan(dir.path(), &base, &head).expect("plan");
        let report = run(dir.path(), &base, &head, &[TodoItem::pick(&entries[2].sha)], &mut |_| {})
            .expect("改写");

        promote(dir.path(), "main", &head, &report.new_head, &report.temp_branch).expect("切换");

        assert_eq!(head_of(dir.path()), report.new_head);
        assert_eq!(
            process::run(Some(dir.path()), &["symbolic-ref", "--short", "HEAD"])
                .expect("branch")
                .stdout
                .trim(),
            "main",
            "要切回原分支，不能留在临时分支上"
        );
        let branches = process::run(Some(dir.path()), &["branch", "--list"]).expect("branch").stdout;
        assert!(!branches.contains(TEMP_PREFIX), "成功之后临时分支要清掉：{branches}");
    }

    #[test]
    fn a_conflict_stops_the_run_and_keeps_the_temp_branch() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "base\n").expect("write");
        git_in(dir.path(), &["add", "-A"]);
        git_in(dir.path(), &["commit", "-q", "-m", "feat: base"]);
        let base = rev_parse(dir.path(), "HEAD").expect("base");

        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        std::fs::write(dir.path().join("a.txt"), "side\n").expect("write");
        git_in(dir.path(), &["commit", "-qam", "feat: side"]);
        let side = rev_parse(dir.path(), "HEAD").expect("side");

        git_in(dir.path(), &["checkout", "-q", "main"]);
        std::fs::write(dir.path().join("a.txt"), "main\n").expect("write");
        git_in(dir.path(), &["commit", "-qam", "feat: main"]);
        let head = rev_parse(dir.path(), "HEAD").expect("head");

        // side 与 main 改同一个文件，必冲突
        let err = run(
            dir.path(),
            &base,
            &head,
            &[TodoItem::pick(&side), TodoItem::pick(&head)],
            &mut |_| {},
        )
        .expect_err("该冲突");

        let text = format!("{err:?}");
        assert!(text.contains("改写停在"), "错误要说明停在哪个提交：{text}");
        assert!(text.contains(TEMP_PREFIX), "错误要给出临时分支名：{text}");
        let branches = process::run(Some(dir.path()), &["branch", "--list"]).expect("branch").stdout;
        assert!(branches.contains(TEMP_PREFIX), "临时分支要留下来供诊断：{branches}");
    }

    #[test]
    fn squash_without_a_previous_commit_is_refused() {
        let err = validate(&[TodoItem {
            sha: "a".repeat(40),
            action: Action::Squash,
            message: None,
        }])
        .expect_err("打头的 squash 没有可并对象");
        assert!(matches!(err, GitError::ParseFailure { .. }), "{err:?}");
    }

    #[test]
    fn reword_after_a_plain_pick_is_its_own_group() {
        let todo = vec![
            TodoItem::pick(&"a".repeat(40)),
            TodoItem { sha: "b".repeat(40), action: Action::Reword, message: Some("x".into()) },
        ];
        assert!(
            validate(&todo).is_ok(),
            "改写最后一条的信息是最常用的操作，不该被当成非法"
        );
    }

    #[test]
    fn reword_at_the_head_of_a_group_may_be_followed_by_squash() {
        let todo = vec![
            TodoItem { sha: "a".repeat(40), action: Action::Reword, message: Some("x".into()) },
            TodoItem { sha: "b".repeat(40), action: Action::Squash, message: None },
        ];
        assert!(validate(&todo).is_ok(), "reword 起头再 squash 是 git 自己的写法");
    }

    #[test]
    fn empty_todo_and_a_leading_squash_are_both_refused() {
        assert!(validate(&[]).is_err(), "空 todo 要挡住");
        assert!(
            validate(&[TodoItem { sha: "a".repeat(40), action: Action::Squash, message: None }]).is_err(),
            "打头的 squash 没有可并对象"
        );
    }

    #[test]
    fn only_drop_may_change_the_tree() {
        assert!(Action::Drop.changes_tree());
        for action in [Action::Pick, Action::Reword, Action::Squash, Action::Fixup] {
            assert!(!action.changes_tree(), "{action:?} 不该被当成改内容");
        }
    }
}