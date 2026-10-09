use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;

use crate::error::GitError;
use crate::git::log::{self, Filter};
use crate::git::message;
use crate::git::process;

/// 活跃度统计（报告页的第二个视角）。
///
/// 与符合率报告同源的纪律：**所有数字都由这里出**，界面不重算、不猜。
/// 两者共用 `log::Filter` 的条件（同一组 `--since/--author/--rev`），
/// 所以"报告里的条数"和"历史里筛出来的条数"不会各说各话。
///
/// 三条口径上的自觉（都是"算错比不算更糟"的坑）：
///
/// 1. **作者走 `.mailmap`**（`%aN/%aE` 而不是 `%an/%ae`）。同一个人换邮箱、换机器
///    就该是同一个人，否则"每人提交次数"会把一个人拆成三个，表格直接失去意义。
/// 2. **merge 与 revert 不计入**。理由与符合率报告一样：它们的信息是 git 生成的，
///    记在谁头上都不对；`excluded` 把排除了多少说出来。
/// 3. **行数是"git 记录的增删行"，不是净产出**。rebase / amend 会把同一次改动数两遍，
///    大重构也会刷屏。字段命名与界面文案都按"活动度"而不是"绩效"来写。
///
/// 上限与截断：`SCAN_LIMIT` 与符合率报告同量级，撞了上限必须 `truncated` 明说——
/// 长区间下静默截断比不给数字更糟（需求 5.10：10 万级仓库不做全量统计）。

/// 硬上限。撞上了就是 `truncated`，界面要显示"只统计了最近 N 条"
const SCAN_LIMIT: usize = 5000;

/// 按天趋势最多保留的天数（一天的量级足够看出节奏，再多只是把表格拉长）
const DAY_LIMIT: usize = 400;

const FIELD_SEP: char = '\u{1f}';
/// 提交头与 numstat 之间用 RS（`%x1e`）分段：`--numstat` 会把行数接在提交头下面，
/// 而 `-z` 会让 numstat 自己用 NUL 做字段分隔，跟提交的 NUL 分隔符撞在一起，
/// 没法稳定切开。所以这里不用 `-z`，改用格式串里的 RS 自己分段。
const RECORD_SEP: char = '\u{1e}';

/// 提交头的字段。`%aN/%aE` 是 `.mailmap` 归并后的作者，`%an/%ae` 是原样。
/// `%ad` 在 `--date=format-local:%Y-%m-%d` 下就是本地日历日，用来分天。
const FIELDS: &[&str] = &["%H", "%aN", "%aE", "%at", "%ad", "%P", "%s"];

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthorStat {
    /// `.mailmap` 归并后的名字；同名不同邮箱按邮箱分开（那确实是两个人）
    pub name: String,
    pub email: String,
    pub commits: usize,
    pub insertions: usize,
    pub deletions: usize,
    /// 动过的文件数（重命名算一个文件，靠 `-M` 让纯改名不虚增）
    pub files: usize,
    /// 有提交的自然天数。它比"提交次数"更能看出节奏（连着五天每天一条 vs 一天五条）
    pub active_days: usize,
    pub last_time: i64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DayStat {
    /// 本地日历日（git 用 `format-local` 算，时区归它管）
    pub day: String,
    pub commits: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub authors: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub commits: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub files: usize,
    /// 二进制文件数：numstat 对它们给 `-\t-`，行数无从谈起，
    /// 所以单独报个数，而不是让它们悄悄算成 0 行
    pub binary_files: usize,
    pub author_count: usize,
    pub active_days: usize,
    pub authors: Vec<AuthorStat>,
    /// 按本地日历日升序（趋势图从左往右就是时间方向）
    pub by_day: Vec<DayStat>,
    /// 被排除的 merge / revert 条数
    pub excluded: usize,
    /// 扫描撞了上限：下面的数字只覆盖最近 `SCAN_LIMIT` 条
    pub truncated: bool,
    /// 区间里一共多少条（`rev-list --count`，与历史列表页同一把尺）
    pub total_in_range: usize,
    pub rev: String,
    pub since: Option<String>,
    pub until: Option<String>,
}

/// 统计区间。作者筛选留在 `Filter.authors` 里，但按作者聚合时以聚合结果为准，
/// 所以命令行不传 `--author`——否则表格只剩一个人，"按作者"就白给了。
pub fn build(repo: &Path, filter: &Filter) -> Result<Activity, GitError> {
    let rev = filter.rev.clone().unwrap_or_else(|| "HEAD".to_string());

    // 空仓库没有 HEAD，`git log HEAD` 是硬失败。这是需要说清楚的正常状态：
    // 给一个空统计（而不是报错），界面自己显示"这个区间还没有提交"
    let head = process::run(Some(repo), &["rev-parse", "--verify", "-q", "HEAD"])?;
    if !head.success {
        return Ok(empty(&rev, filter));
    }

    let raw = scan(repo, filter, &rev)?;
    let parsed = parse(&raw)?;
    let truncated = parsed.len() >= SCAN_LIMIT;
    let total_in_range = if truncated {
        log::count(repo, filter)?
    } else {
        parsed.len()
    };

    let mut authors: BTreeMap<(String, String), Author> = BTreeMap::new();
    let mut days: BTreeMap<String, Day> = BTreeMap::new();
    let mut excluded = 0usize;
    let mut commits = 0usize;
    let mut insertions = 0usize;
    let mut deletions = 0usize;
    let mut files = 0usize;
    let mut binary_files = 0usize;

    for commit in &parsed {
        // 合并与 revert 的提交信息是 git 自己生成的，记在谁头上都不对
        if commit.merge || commit.revert {
            excluded += 1;
            continue;
        }

        commits += 1;
        insertions += commit.insertions;
        deletions += commit.deletions;
        files += commit.changed_files;
        binary_files += commit.binary_files;

        let key = (commit.author_name.clone(), commit.author_email.clone());
        let slot = authors.entry(key.clone()).or_insert_with(|| Author {
            name: commit.author_name.clone(),
            email: commit.author_email.clone(),
            commits: 0,
            insertions: 0,
            deletions: 0,
            files: 0,
            active_days: BTreeSet::new(),
            last_time: commit.time,
        });
        slot.commits += 1;
        slot.insertions += commit.insertions;
        slot.deletions += commit.deletions;
        slot.files += commit.changed_files;
        slot.active_days.insert(commit.day.clone());
        slot.last_time = slot.last_time.max(commit.time);

        let day = days.entry(commit.day.clone()).or_insert_with(|| Day {
            commits: 0,
            insertions: 0,
            deletions: 0,
            authors: BTreeSet::new(),
        });
        day.commits += 1;
        day.insertions += commit.insertions;
        day.deletions += commit.deletions;
        day.authors.insert(key);
    }

    let mut author_list: Vec<AuthorStat> = authors
        .into_values()
        .map(|author| AuthorStat {
            active_days: author.active_days.len(),
            name: author.name,
            email: author.email,
            commits: author.commits,
            insertions: author.insertions,
            deletions: author.deletions,
            files: author.files,
            last_time: author.last_time,
        })
        .collect();
    // 提交多的排前面；同数按名字排，保证同一个仓库每次刷新顺序都一样（截图/对比才有用）
    author_list.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.email.cmp(&b.email))
    });

    // 按天保留最近的一段：更早的日期在趋势图上只是一条平线，却能把图压扁
    let mut day_list: Vec<DayStat> = days
        .iter()
        .map(|(day, stat)| DayStat {
            day: day.clone(),
            commits: stat.commits,
            insertions: stat.insertions,
            deletions: stat.deletions,
            authors: stat.authors.len(),
        })
        .collect();
    if day_list.len() > DAY_LIMIT {
        day_list.drain(..day_list.len() - DAY_LIMIT);
    }

    Ok(Activity {
        commits,
        insertions,
        deletions,
        files,
        binary_files,
        author_count: author_list.len(),
        active_days: days.len(),
        authors: author_list,
        by_day: day_list,
        excluded,
        truncated,
        total_in_range,
        rev,
        since: filter.since.clone(),
        until: filter.until.clone(),
    })
}

struct Author {
    name: String,
    email: String,
    commits: usize,
    insertions: usize,
    deletions: usize,
    files: usize,
    active_days: BTreeSet<String>,
    last_time: i64,
}

struct Day {
    commits: usize,
    insertions: usize,
    deletions: usize,
    authors: BTreeSet<(String, String)>,
}

fn empty(rev: &str, filter: &Filter) -> Activity {
    Activity {
        commits: 0,
        insertions: 0,
        deletions: 0,
        files: 0,
        binary_files: 0,
        author_count: 0,
        active_days: 0,
        authors: Vec::new(),
        by_day: Vec::new(),
        excluded: 0,
        truncated: false,
        total_in_range: 0,
        rev: rev.to_string(),
        since: filter.since.clone(),
        until: filter.until.clone(),
    }
}

/// 一次 `log --numstat`。`-M` 显式打开改名检测：纯改名不该算成"删掉整个文件再加一遍"，
/// 而它依赖用户全局的 `diff.renames`，不显式给就不可复现。
fn scan(repo: &Path, filter: &Filter, rev: &str) -> Result<String, GitError> {
    // 提交头用 %x1f 分字段、首字段前用 %x1e 分段
    let format = format!("--format=%x1e{}%x1f", FIELDS.join("%x1f"));
    let limit = SCAN_LIMIT.to_string();
    let mut args = vec![
        "log",
        "--topo-order",
        "-M",
        "--numstat",
        // 天数按本地日历切，时区换算交给 git——自己算就得引时区库，
        // 而 `--since today` 的口径也是 git 的，两边必须同一个裁判
        "--date=format-local:%Y-%m-%d",
    ];
    args.push(&format);
    args.extend(log::condition_args(filter));
    args.push("-n");
    args.push(&limit);
    args.push(rev);
    if let Some(path) = filter.path.as_deref() {
        args.push("--");
        args.push(path);
    }
    let out = process::run(Some(repo), &args)?.expect_success()?;
    Ok(out)
}

#[derive(Debug)]
struct Parsed {
    author_name: String,
    author_email: String,
    time: i64,
    day: String,
    merge: bool,
    revert: bool,
    insertions: usize,
    deletions: usize,
    changed_files: usize,
    binary_files: usize,
}

fn parse(raw: &str) -> Result<Vec<Parsed>, GitError> {
    let mut commits = Vec::new();
    for record in raw.split(RECORD_SEP) {
        if record.trim().is_empty() {
            continue;
        }
        let mut lines = record.lines();
        let Some(head) = lines.next() else {
            continue;
        };
        let parts: Vec<&str> = head.split(FIELD_SEP).collect();
        if parts.len() < FIELDS.len() {
            return Err(GitError::ParseFailure {
                snippet: process::snippet(record),
            });
        }
        let Ok(time) = parts[3].trim().parse::<i64>() else {
            return Err(GitError::ParseFailure {
                snippet: process::snippet(record),
            });
        };
        let subject = parts[6].to_string();
        let summary = message::summarize(&subject, "");

        let mut parsed = Parsed {
            author_name: parts[1].to_string(),
            author_email: parts[2].to_string(),
            time,
            day: parts[4].trim().to_string(),
            // 合并：多个父。空父（根提交）算 0，不受影响
            merge: parts[5].split_whitespace().count() > 1,
            revert: message::is_revert(&subject, summary.commit_type.as_deref()),
            insertions: 0,
            deletions: 0,
            changed_files: 0,
            binary_files: 0,
        };
        // 名字可能为空（git 允许 `--author` 配空名），退回邮箱：表格里不能出现空行
        if parsed.author_name.trim().is_empty() {
            parsed.author_name = parsed.author_email.clone();
        }

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // numstat 一行是「新增 \t 删除 \t 路径」：要切够三段，
            // 只切两段的话第二段会变成「0\tsrc/a.rs」而解析失败，
            // 于是每个文本文件都被当成二进制，行数全丢
            let mut fields = line.splitn(3, '\t');
            let (Some(added), Some(removed)) = (fields.next(), fields.next()) else {
                continue;
            };
            parsed.changed_files += 1;
            // 二进制文件 numstat 给 `-\t-`：算进文件数，但行数不参与加减
            match (added.parse::<usize>(), removed.parse::<usize>()) {
                (Ok(added), Ok(removed)) => {
                    parsed.insertions += added;
                    parsed.deletions += removed;
                }
                _ => parsed.binary_files += 1,
            }
        }

        commits.push(parsed);
    }
    Ok(commits)
}





#[cfg(test)]
mod tests {
    use super::*;

    fn git_in(dir: &Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    /// 建一条提交。`name` / `email` / `when` 可以按需改，用来测 mailmap 与时间分组。
    ///
    /// 作者时间与提交时间都显式给：`--since/--until` 认的是**提交时间**，
    /// 只改作者时间的话区间测试会被真实时钟搅乱。
    fn commit_at(
        dir: &Path,
        name: &str,
        email: &str,
        when: &str,
        message_text: &str,
        file: &str,
        content: &str,
    ) {
        let content = format!("{content}\n");
        std::fs::write(dir.join(file), &content).expect("write");
        git_in(dir, &["add", file]);
        let stamp = when.to_string();
        let out = process::run_with_env(
            Some(dir),
            &[
                "-c",
                &format!("user.name={name}"),
                "-c",
                &format!("user.email={email}"),
                "commit",
                "-q",
                "-m",
                message_text,
            ],
            &[("GIT_AUTHOR_DATE", &stamp), ("GIT_COMMITTER_DATE", &stamp)],
        )
        .expect("spawn git");
        assert!(out.success, "git commit 失败：{}", out.stderr);
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        dir
    }

    fn stat_of(activity: &Activity, name: &str) -> AuthorStat {
        activity
            .authors
            .iter()
            .find(|item| item.name == name)
            .unwrap_or_else(|| panic!("找不到作者 {name}：{:?}", activity.authors))
            .clone()
    }

    #[test]
    fn counts_commits_and_lines_per_author() {
        let dir = repo();
        commit_at(
            dir.path(),
            "张三",
            "z@example.com",
            "2026-01-02T10:00:00",
            "feat: 甲",
            "a.txt",
            "one",
        );
        commit_at(
            dir.path(),
            "张三",
            "z@example.com",
            "2026-01-02T11:00:00",
            "fix: 乙",
            "a.txt",
            "one\ntwo",
        );
        commit_at(
            dir.path(),
            "李四",
            "l@example.com",
            "2026-01-03T10:00:00",
            "chore: 丙",
            "b.txt",
            "x",
        );

        let activity = build(dir.path(), &Filter::default()).expect("build");
        assert_eq!(activity.commits, 3);
        // 甲的两条各加一行（第二行、b.txt 那一行），李四加一行
        assert_eq!(activity.insertions, 3);
        assert_eq!(activity.deletions, 0);
        assert_eq!(activity.author_count, 2);

        let zhang = stat_of(&activity, "张三");
        assert_eq!(zhang.commits, 2);
        // 两条都在同一天：活跃天数是 1，不是 2
        assert_eq!(zhang.active_days, 1);
        assert_eq!(zhang.files, 2);

        // 提交多的排前面
        assert_eq!(activity.authors[0].name, "张三");
    }

    #[test]
    fn mailmap_merges_two_emails_into_one_person() {
        let dir = repo();
        // 两条旧身份都归到同一个正经身份。mailmap 只在**旧**身份那一侧匹配，
        // 所以测试里不能用正经邮箱去提交（那样反而不会被改名）
        std::fs::write(
            dir.path().join(".mailmap"),
            "Zhang <proper@example.com> <old@example.com>\n\
             Zhang <proper@example.com> <other@example.com>\n",
        )
        .expect("write mailmap");
        commit_at(
            dir.path(),
            "zhang",
            "old@example.com",
            "2026-01-02T10:00:00",
            "feat: 甲",
            "a.txt",
            "one",
        );
        commit_at(
            dir.path(),
            "wang",
            "other@example.com",
            "2026-01-03T10:00:00",
            "feat: 乙",
            "b.txt",
            "two",
        );

        let activity = build(dir.path(), &Filter::default()).expect("build");
        // 换了邮箱、连名字都不同，还是一个人：两个人会让"每人提交次数"完全失去意义
        assert_eq!(activity.author_count, 1, "{:?}", activity.authors);
        assert_eq!(stat_of(&activity, "Zhang").commits, 2);
    }

    #[test]
    fn merge_and_revert_are_excluded_but_still_reported() {
        let dir = repo();
        commit_at(
            dir.path(),
            "甲",
            "a@example.com",
            "2026-01-02T10:00:00",
            "feat: 基线",
            "a.txt",
            "one",
        );
        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        commit_at(
            dir.path(),
            "乙",
            "b@example.com",
            "2026-01-02T11:00:00",
            "feat: 支线",
            "b.txt",
            "two",
        );
        git_in(dir.path(), &["checkout", "-q", "main"]);
        git_in(dir.path(), &["merge", "--no-ff", "-q", "-m", "chore: 合并支线", "side"]);
        git_in(dir.path(), &["revert", "--no-edit", "HEAD~1"]);

        let activity = build(dir.path(), &Filter::default()).expect("build");
        assert_eq!(activity.commits, 2, "只剩两条真人提交");
        assert_eq!(activity.excluded, 2, "合并与 revert 都要说出来");
        assert_eq!(activity.insertions, 2, "合并不能把支线那一条再数一遍");
    }

    #[test]
    fn a_since_filter_limits_the_range() {
        let dir = repo();
        commit_at(
            dir.path(),
            "甲",
            "a@example.com",
            "2026-01-02T10:00:00",
            "feat: 一",
            "a.txt",
            "one",
        );
        commit_at(
            dir.path(),
            "甲",
            "a@example.com",
            "2026-03-05T10:00:00",
            "feat: 二",
            "b.txt",
            "two",
        );

        let filter = Filter {
            since: Some("2026-02-01".into()),
            ..Filter::default()
        };
        let activity = build(dir.path(), &filter).expect("build");
        assert_eq!(activity.commits, 1);
        assert_eq!(activity.by_day[0].day, "2026-03-05");
        assert_eq!(activity.since.as_deref(), Some("2026-02-01"));
    }

    #[test]
    fn binary_files_are_counted_without_polluting_line_totals() {
        let dir = repo();
        let mut data = vec![0u8; 32];
        data[0] = 0x00;
        std::fs::write(dir.path().join("blob.bin"), data).expect("write");
        git_in(dir.path(), &["add", "blob.bin"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=甲",
                "-c",
                "user.email=a@example.com",
                "commit",
                "-q",
                "-m",
                "chore: 放个二进制",
            ],
        );

        let activity = build(dir.path(), &Filter::default()).expect("build");
        assert_eq!(activity.commits, 1);
        assert_eq!(activity.insertions, 0, "二进制的行数无从谈起，不能算成 0 之外的东西");
        assert_eq!(activity.files, 1);
        assert!(activity.binary_files >= 1, "二进制要单独报出来");
    }

    #[test]
    fn an_empty_repository_reports_zero_rather_than_failing() {
        let dir = repo();
        let activity = build(dir.path(), &Filter::default()).expect("空仓库也要有数");
        assert_eq!(activity.commits, 0);
        assert!(activity.authors.is_empty());
        assert!(activity.by_day.is_empty());
    }

    #[test]
    fn day_grouping_uses_local_calendar_days() {
        // 贴近 UTC 零点的那条提交：按 UTC 分会把它算到隔壁日子，
        // 而界面上的「今日」是本地零点，两边必须同一个裁判
        let dir = repo();
        commit_at(
            dir.path(),
            "甲",
            "a@example.com",
            "2026-01-02T00:30:00",
            "feat: 甲",
            "a.txt",
            "one",
        );
        commit_at(
            dir.path(),
            "乙",
            "b@example.com",
            "2026-01-02T09:00:00",
            "feat: 乙",
            "b.txt",
            "two",
        );

        let activity = build(dir.path(), &Filter::default()).expect("build");
        assert_eq!(activity.by_day.len(), 1);
        assert_eq!(activity.by_day[0].day, "2026-01-02");
        assert_eq!(activity.by_day[0].commits, 2);
        assert_eq!(activity.by_day[0].authors, 2, "一天里两笔就是两位作者");
    }
}