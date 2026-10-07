use std::path::Path;

use serde::Serialize;

use super::refs::{self, Badge};
use super::{message, process};
use crate::error::GitError;

/// git log 的字段顺序，同时是解析时的下标顺序。
///
/// 分隔符契约（本机 git 2.54 实测）：`-z` 让每条记录以 NUL 结尾，字段之间用 `%x1f`。
/// git 不允许提交信息里出现 NUL，所以记录边界是绝对的；0x1f 理论上能出现在正文里，
/// 但 `%b` 排在最后一位，解析用 splitn 截断，多余的分隔符只会留在正文内、不会切错字段。
/// `%s` 可以从 `%b` 推出来，但那是 git 自己的主题切分逻辑，不自己重写一遍。
///
/// `%D`（引用装饰）在这里取，不在父子遍历（git/graph.rs）那边取：图那一层的分配结果
/// 是按 HEAD 的 sha 缓存的，而"打个标签""把分支挪一挪"都不动 HEAD 的 sha，放过去就会
/// 一直端着过期的徽标。列表每页本来就要起一次 log 进程，顺带就拿到了。
const FIELDS: &[&str] = &[
    "%H",
    "%an",
    "%ae",
    "%at",
    "%P",
    "%D",
    "%s",
    "%b",
];
const FIELD_HEX: &str = "%x1f";
const FIELD_SEP: char = '\u{1f}';
const RECORD_SEP: char = '\0';

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub id: String,
    pub author_name: String,
    pub author_email: String,
    /// 作者时间，Unix 秒；渲染成什么时区归前端管
    pub time: i64,
    pub subject: String,
    pub body: String,
    pub merge: bool,
    pub revert: bool,
    /// 这一条是不是 HEAD 当前所在的提交，含游离 HEAD（游离时没有任何具名引用指向它，
    /// 靠 refs 里找不出"当前"，所以这个标记不能省）
    pub head: bool,
    /// 指向这一条的分支 / 远程跟踪分支 / 标签
    pub refs: Vec<Badge>,
    /// Conventional Commits 的语法拆解结果，列表按 type 染色就靠它
    #[serde(flatten)]
    pub summary: message::Summary,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitPage {
    pub commits: Vec<Commit>,
    /// HEAD 可达的提交总数，供分页与"共 N 条"使用
    pub total: usize,
}

/// 按 skip/limit 读一页提交。需求五第 9 条：10 万级仓库禁止全量拉取。
pub fn list(repo: &Path, skip: usize, limit: usize) -> Result<CommitPage, GitError> {
    // 空仓库没有 HEAD。这是需要区分的正常状态，不是错误——返回空页。
    let head = process::run(Some(repo), &["rev-parse", "--verify", "-q", "HEAD"])?;
    if !head.success {
        return Ok(CommitPage {
            commits: Vec::new(),
            total: 0,
        });
    }

    let count = process::run(Some(repo), &["rev-list", "--count", "HEAD"])?.expect_success()?;
    let total = parse_count(count.trim())?;

    let format = format!("--format={}", FIELDS.join(FIELD_HEX));
    let (skip_arg, limit_arg) = (skip.to_string(), limit.to_string());
    let stdout = process::run(
        Some(repo),
        &[
            // --topo-order：子一定在父之前，且一条支线不被日期切散。
            // 图列（git/graph.rs）按同一个顺序算泳道，两边顺序必须一致，
            // 否则第 N 行的连线会接到隔壁那行身上。
            "log",
            "-z",
            "--topo-order",
            // 短形式下本地分支 feat/x 和远程 feat/x 长得一模一样（实测），
            // 徽标要分得开类别就只有限定完整 refname 这一条路。
            "--decorate=full",
            &format,
            "--skip",
            &skip_arg,
            "-n",
            &limit_arg,
            "HEAD",
        ],
    )?
    .expect_success()?;

    Ok(CommitPage {
        commits: parse(&stdout)?,
        total,
    })
}

fn parse(stdout: &str) -> Result<Vec<Commit>, GitError> {
    stdout
        .split(RECORD_SEP)
        .filter(|record| !record.trim().is_empty())
        .map(parse_commit)
        .collect()
}

fn parse_commit(record: &str) -> Result<Commit, GitError> {
    let failure = || GitError::ParseFailure {
        snippet: process::snippet(record),
    };
    let parts: Vec<&str> = record.splitn(FIELDS.len(), FIELD_SEP).collect();
    if parts.len() != FIELDS.len() {
        return Err(failure());
    }

    // 时间戳必须是数字：字段一旦因为异常分隔符而错位，这里最先暴露
    let Ok(time) = parts[3].parse::<i64>() else {
        return Err(failure());
    };

    let subject = parts[6].to_string();
    let body = parts[7].trim_end_matches(['\n', '\r']).to_string();
    let summary = message::summarize(&subject, &body);
    let revert = message::is_revert(&subject, summary.commit_type.as_deref());
    let (head, badges) = parse_decoration(parts[5]);

    Ok(Commit {
        id: parts[0].to_string(),
        author_name: parts[1].to_string(),
        author_email: parts[2].to_string(),
        time,
        merge: parts[4].split_whitespace().count() > 1,
        revert,
        head,
        refs: badges,
        subject,
        body,
        summary,
    })
}

/// `%D` 的形态（本机 git 2.54 实测，配合 `--decorate=full`）：
/// `HEAD -> refs/heads/main`、游离 HEAD 时单独一项 `HEAD`、`tag: refs/tags/v1`
/// （附注和轻量标签都带 `tag: ` 前缀，实测分不开，所以标签归类看 refs/tags/）、
/// `refs/remotes/origin/main`。没有引用指向的提交这一段是空串，分隔符照样输出，
/// 所以字段数量不受影响。
///
/// 项之间用 `, ` 分隔。refname 里允许逗号、不允许空格（git-check-ref-format），
/// 所以按 `, ` 切不会把一条名字切成两段——实测分支 `feat,with-comma` 完整落在一项里。
fn parse_decoration(raw: &str) -> (bool, Vec<Badge>) {
    let mut head = false;
    let mut badges = Vec::new();

    for item in raw.split(", ") {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if let Some(target) = item.strip_prefix("HEAD -> ") {
            // 箭头是"当前"的判据，不是徽标内容：它指向的那个引用照常出一个徽标
            head = true;
            push_badge(&mut badges, target, true);
        } else if item == "HEAD" {
            // 游离 HEAD：这一条就是当前提交，但它不落在任何具名引用上
            head = true;
        } else {
            push_badge(&mut badges, item, false);
        }
    }

    (head, badges)
}

fn push_badge(badges: &mut Vec<Badge>, item: &str, head: bool) {
    // `tag: ` 只是说明它来自 refs/tags/，归类看前缀就够，这个标记不必留下
    let full = item.strip_prefix("tag: ").unwrap_or(item);
    let Some((kind, name)) = refs::classify(full) else {
        return; // `grafted:` 这类项不是一个引用名字，画不出徽标
    };

    badges.push(Badge {
        name: name.to_string(),
        kind,
        head,
    });
}

fn parse_count(raw: &str) -> Result<usize, GitError> {
    raw.parse::<usize>().map_err(|_| GitError::ParseFailure {
        snippet: format!("rev-list --count 输出：{raw}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::refs::RefKind;
    use std::fs;
    use std::path::Path;

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} failed: {}", out.stderr);
        out.stdout.trim().to_string()
    }

    /// 每个测试独立建临时仓库，避免共享 fixture 带来的顺序依赖
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "."]);
        git_in(dir.path(), &["config", "user.name", "测试者"]);
        git_in(dir.path(), &["config", "user.email", "t@example.com"]);
        dir
    }

    fn commit(dir: &Path, file: &str, msg: &str) {
        fs::write(dir.join(file), format!("{file}\n")).expect("write");
        git_in(dir, &["add", file]);
        git_in(dir, &["commit", "-q", "-m", msg]);
    }

    fn subject_of(dir: &Path, skip: usize, limit: usize) -> Vec<String> {
        list(dir, skip, limit)
            .expect("list ok")
            .commits
            .into_iter()
            .map(|c| c.subject)
            .collect()
    }

    #[test]
    fn empty_repo_is_an_empty_page_not_an_error() {
        let dir = repo();
        let page = list(dir.path(), 0, 200).expect("空仓库不该报错");
        assert_eq!(page.total, 0);
        assert!(page.commits.is_empty());
    }

    #[test]
    fn chinese_subject_non_ascii_author_and_multiline_body_survive() {
        let dir = repo();
        commit(
            dir.path(),
            "a.txt",
            "feat: 支持中文标题\n\n正文第一行\n正文第二行\n\nBREAKING CHANGE: 旧配置不再兼容\n",
        );

        let page = list(dir.path(), 0, 200).expect("list ok");
        assert_eq!(page.total, 1);
        let c = &page.commits[0];
        assert_eq!(c.subject, "feat: 支持中文标题");
        assert_eq!(c.author_name, "测试者", "中文作者名不该被转义或截断");
        assert_eq!(
            c.body,
            "正文第一行\n正文第二行\n\nBREAKING CHANGE: 旧配置不再兼容"
        );
        assert_eq!(c.summary.commit_type.as_deref(), Some("feat"));
        assert!(
            c.summary.breaking,
            "footer 的 BREAKING CHANGE 应标记 breaking"
        );
        assert!(!c.merge);
        assert_eq!(c.id.len(), 40);
        assert!(c.time > 0);
    }

    #[test]
    fn pages_are_contiguous_and_total_counts_everything() {
        let dir = repo();
        for i in 0..3 {
            commit(dir.path(), &format!("f{i}.txt"), &format!("chore: 第{i}条"));
        }

        let first = list(dir.path(), 0, 2).expect("first page");
        assert_eq!(first.total, 3, "total 是全量条数，不是本页条数");
        assert_eq!(first.commits.len(), 2);
        assert_eq!(
            subject_of(dir.path(), 2, 2),
            vec!["chore: 第0条"],
            "最新提交在前，尾页应剩最旧的一条"
        );
    }

    #[test]
    fn merge_commit_is_flagged_by_parent_count() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        commit(dir.path(), "b.txt", "feat: 支线");
        // 用 `checkout -` 回到上一个分支，避免把默认分支名写死成 master 或 main
        git_in(dir.path(), &["checkout", "-q", "-"]);
        commit(dir.path(), "c.txt", "feat: 主干另一条");
        git_in(
            dir.path(),
            &["merge", "--no-ff", "-q", "-m", "chore: 合并支线", "side"],
        );

        let page = list(dir.path(), 0, 10).expect("list ok");
        let head = &page.commits[0];
        assert_eq!(head.subject, "chore: 合并支线");
        assert!(head.merge, "两个父提交的记录必须标成 merge");
        assert!(
            page.commits[1..].iter().all(|c| !c.merge),
            "普通提交不该被误标为 merge"
        );
    }

    /// 列表顺序和图顺序必须是同一次遍历的结果：两边各排各的，泳道就会接到隔壁行上
    #[test]
    fn list_order_matches_the_graph_walk() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        commit(dir.path(), "b.txt", "feat: 支线一");
        commit(dir.path(), "c.txt", "feat: 支线二");
        git_in(dir.path(), &["checkout", "-q", "-"]);
        commit(dir.path(), "d.txt", "feat: 主干");
        git_in(
            dir.path(),
            &["merge", "--no-ff", "-q", "-m", "chore: 合并支线", "side"],
        );

        let page = list(dir.path(), 0, 10).expect("list ok");
        let listed: Vec<&str> = page.commits.iter().map(|c| c.id.as_str()).collect();
        let walked: Vec<String> = crate::git::graph::history(dir.path())
            .expect("读父子")
            .into_iter()
            .map(|(sha, _)| sha)
            .collect();

        assert_eq!(listed, walked, "分页列表和图走的不是同一个顺序");
    }

    #[test]
    fn revert_commit_is_flagged() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 加个开关");
        commit(dir.path(), "b.txt", "fix: 修一下");
        // git revert 不认短选项 -q（本机 git 2.54 实测 exit 129），只能用长形式
        git_in(dir.path(), &["revert", "--no-edit", "HEAD"]);

        let page = list(dir.path(), 0, 10).expect("list ok");
        assert!(
            page.commits[0].revert,
            "git revert 生成的 Revert \"...\" 应标记 revert"
        );
    }

    #[test]
    fn stray_unit_separator_stays_inside_the_body() {
        // %b 在最后一位，splitn 让正文里的 0x1f 变成正文的一部分，而不是多切出一个字段
        let c = parse_commit(&record_for("", "正文里有个\u{1f}怪字符")).expect("该解析成功");
        assert_eq!(c.author_name, "张三");
        assert_eq!(c.body, "正文里有个\u{1f}怪字符");
        assert!(c.refs.is_empty() && !c.head, "空装饰段就是没有徽标");
    }

    #[test]
    fn misaligned_fields_are_reported_as_parse_failure() {
        // 时间戳位置落到了非数字上，说明分隔符数量不对，必须显式失败而不是静默错位
        let record = format!(
            "deadbeef{FIELD_SEP}张三{FIELD_SEP}z@e.com{FIELD_SEP}不是数字{FIELD_SEP}{FIELD_SEP}{FIELD_SEP}fix: 标题{FIELD_SEP}正文"
        );
        assert!(matches!(
            parse_commit(&record),
            Err(GitError::ParseFailure { .. })
        ));

        // 少一个分隔符就少一个字段：%D 加进来后总数是 8，旧的 7 段形态算坏数据
        let too_few = format!("abc{FIELD_SEP}张三");
        assert!(matches!(
            parse_commit(&too_few),
            Err(GitError::ParseFailure { .. })
        ));
    }

    /// 按 FIELDS 的顺序拼一条记录，只给 %D 和 %b 传内容，其余字段固定
    fn record_for(decoration: &str, body: &str) -> String {
        format!(
            "d17a5a3aa7ad14e4b6ddc4bb2b7cd2a25a0e0aa5{FIELD_SEP}张三{FIELD_SEP}z@e.com{FIELD_SEP}1700000000{FIELD_SEP}{FIELD_SEP}{decoration}{FIELD_SEP}fix: 标题{FIELD_SEP}{body}"
        )
    }

    #[test]
    fn an_attached_head_marks_the_commit_and_keeps_its_branch_badge() {
        let c = parse_commit(&record_for("HEAD -> refs/heads/main", "正文")).expect("解析");
        assert!(c.head);
        assert_eq!(c.refs.len(), 1);
        assert_eq!(c.refs[0].name, "main");
        assert_eq!(c.refs[0].kind, RefKind::Branch);
        assert!(c.refs[0].head, "HEAD 指着的引用要单独标出来，界面靠它做当前样式");
    }

    #[test]
    fn a_detached_head_marks_the_commit_without_inventing_a_ref() {
        // 实测游离 HEAD 时 %D 是一段光秃秃的 `HEAD`，后面照常跟着别的引用
        let c = parse_commit(&record_for("HEAD, refs/heads/side, tag: refs/tags/v1", "正文"))
            .expect("解析");
        assert!(c.head, "游离 HEAD 也得知道这是当前提交");
        assert_eq!(c.refs.len(), 2, "游离 HEAD 不是一个引用，不该多出一个徽标");
        assert!(c.refs.iter().all(|badge| !badge.head));
        assert_eq!(
            c.refs[1].kind,
            RefKind::Tag,
            "附注和轻量标签都带 tag: 前缀，归类只看 refs/tags/"
        );
    }

    #[test]
    fn a_comma_inside_a_refname_stays_in_one_badge() {
        // 分支名里可以有逗号、不允许空格，所以按 ", " 切不会把一条名字切成两段（实测）
        let c = parse_commit(&record_for("HEAD -> refs/heads/feat,with-comma", "正文")).expect("解析");
        assert_eq!(c.refs[0].name, "feat,with-comma");
    }

    #[test]
    fn local_and_remote_of_the_same_short_name_are_told_apart() {
        // 这正是选 --decorate=full 的理由：短形式下这两条都显示成 feat/x
        let c = parse_commit(&record_for("refs/heads/feat/x, refs/remotes/feat/x", "正文")).expect("解析");
        let kinds: Vec<RefKind> = c.refs.iter().map(|badge| badge.kind).collect();
        assert_eq!(kinds, vec![RefKind::Branch, RefKind::Remote]);
    }

    #[test]
    fn decoration_items_that_are_not_refs_are_dropped() {
        let c = parse_commit(&record_for("grafted: 12ab34cd, HEAD -> refs/heads/main", "正文"))
            .expect("解析");
        assert_eq!(c.refs.len(), 1, "认不出前缀的装饰项画不出徽标");
        assert!(c.head);
    }

    #[test]
    fn an_empty_decoration_is_a_normal_commit_not_a_failure() {
        let c = parse_commit(&record_for("", "正文")).expect("解析");
        assert!(c.refs.is_empty());
        assert!(!c.head);
    }

    /// §7.3 的验收：徽标内容和 git 自己装饰出来的对得上，不是我们另算一套
    #[test]
    fn badges_match_the_refs_git_itself_decorates() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        git_in(dir.path(), &["branch", "side"]);
        git_in(dir.path(), &["tag", "-a", "v1", "-m", "发布说明"]);
        git_in(dir.path(), &["tag", "lw"]);
        let head = git_in(dir.path(), &["rev-parse", "HEAD"]);
        // 默认分支名由用户的 init.defaultBranch 决定（本机可能是 main 也可能是 master），
        // 这个文件里的用例一律不写死它
        let current = git_in(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        git_in(dir.path(), &["update-ref", "refs/remotes/origin/upstream", &head]);

        let page = list(dir.path(), 0, 10).expect("list ok");
        let top = &page.commits[0];
        assert!(top.head, "HEAD 那条要标当前");
        assert_eq!(top.refs.len(), 5, "实际徽标：{:?}", top.refs);
        for (name, kind) in [
            (current.as_str(), RefKind::Branch),
            ("side", RefKind::Branch),
            ("origin/upstream", RefKind::Remote),
            ("v1", RefKind::Tag),
            ("lw", RefKind::Tag),
        ] {
            assert!(
                top.refs
                    .iter()
                    .any(|badge| badge.name == name && badge.kind == kind),
                "列表里没有 {name}（{kind:?}），实际：{:?}",
                top.refs
            );
        }
        assert!(
            top.refs
                .iter()
                .filter(|badge| badge.head)
                .all(|badge| badge.name == current),
            "只有 HEAD 指向的那个引用算当前"
        );
    }

    /// 需求十第 2 步的验收（5 万 commit 首屏 < 1s）与十二节要求实测回填的数据。
    /// fixture 不进版本库，跑法：
    /// `GIT_TIDY_BENCH_REPO=<5万提交仓库> cargo test --release --lib -- --ignored --nocapture`
    #[test]
    #[ignore = "需要一个 5 万提交的本地 fixture 仓库"]
    fn first_page_on_a_large_repo_stays_under_a_second() {
        let Ok(path) = std::env::var("GIT_TIDY_BENCH_REPO") else {
            panic!("未设置 GIT_TIDY_BENCH_REPO，无法做首屏量测");
        };
        let repo = Path::new(&path);

        let started = std::time::Instant::now();
        let page = list(repo, 0, 200).expect("首屏读取");
        let elapsed = started.elapsed();

        assert_eq!(page.total, 50_000, "fixture 应当是 5 万提交");
        assert_eq!(page.commits.len(), 200);
        println!(
            "首屏 {} 条 / 共 {} 条，耗时 {elapsed:?}（{} ms）",
            page.commits.len(),
            page.total,
            elapsed.as_millis()
        );
        assert!(elapsed.as_millis() < 1000, "首屏超过 1s：{elapsed:?}");
    }
}
