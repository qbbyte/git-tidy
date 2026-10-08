use std::path::Path;

use serde::{Deserialize, Serialize};

use super::refs::{self, Badge};
use super::{message, process};
use crate::config::check;
use crate::config::spec::Spec;
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

#[derive(Serialize, Clone)]
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
    /// 父提交号。筛选态下图只按可见集合算泳道（§7.7），要拿到父才知道哪条边被截断了
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
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
    /// HEAD 可达的提交总数，供分页与"共 N 条"使用。带筛选时是 git 自己按同一组条件数的
    pub total: usize,
    /// 只筛合规/不合规时，type 与判定在解析层做，只能一段段往前扫；
    /// 扫到上限就停，置 true。界面据此说明"下面未必覆盖全量"，不假装扫完了。
    #[serde(default)]
    pub truncated: bool,
}

/// 提交列表的筛选条件（§7.7）。
///
/// 两种能力被刻意分开：**git 认识的条件**（rev 范围、作者、时间、关键词、路径）直接
/// 映射成 `log` 的位置参数与 `--grep/--author/--since/--until`；**git 不认识的**
/// （Conventional type、是否合规）在解析层过滤——git 不知道什么是合规提交。
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    /// rev 范围：分支名、`a..b`、`HEAD~3`。空则用 HEAD
    pub rev: Option<String>,
    pub authors: Vec<String>,
    pub grep: Vec<String>,
    /// 只看动过某个路径的提交
    pub path: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    /// Conventional type 白名单（小写比对）。空 = 不限
    pub types: Vec<String>,
    /// Some(true) 只留合规，Some(false) 只留不合规，None 不限
    pub conformant: Option<bool>,
}

impl Filter {
    /// 没有任何条件时走"只带 skip/limit 的一次 log"，不做分片扫描。
    pub fn is_empty(&self) -> bool {
        self.rev.is_none()
            && self.authors.is_empty()
            && self.grep.is_empty()
            && self.path.is_none()
            && self.since.is_none()
            && self.until.is_none()
            && self.types.is_empty()
            && self.conformant.is_none()
    }

    /// 解析层要动手了：这个 filter 不能一次交给 git 算完。
    pub fn needs_post_filter(&self) -> bool {
        !self.types.is_empty() || self.conformant.is_some()
    }

    /// 缓存键。图缓存按"可见集合"存，键里必须带上全部条件，否则换一个筛选
    /// 就会拿到上一份可见集合算出来的泳道——那是最难发现的一类错图。
    pub fn key(&self) -> String {
        format!(
            "rev={}|authors={}|grep={}|path={}|since={}|until={}|types={}|conformant={}",
            self.rev.as_deref().unwrap_or("HEAD"),
            self.authors.join(","),
            self.grep.join(","),
            self.path.as_deref().unwrap_or(""),
            self.since.as_deref().unwrap_or(""),
            self.until.as_deref().unwrap_or(""),
            self.types.join(","),
            match self.conformant {
                Some(true) => "yes",
                Some(false) => "no",
                None => "any",
            },
        )
    }
}

/// 解析层筛选时一次从 git 取多少条。
///
/// 取多了不划算（一页 200 条命中率高时用不到），取少了进程数翻倍——所以取 500 条，
/// 命中率高与命中低都不算离谱。
const SCAN_CHUNK: usize = 500;
/// 解析层筛选最多往前扫多少条。筛选得越窄，一次要扫的历史越长，不封顶就会变成
/// "选一个罕见 type 等到界面转圈"。超了置 `truncated`，界面明说只扫了前一段。
const SCAN_MAX_ROWS: usize = 50_000;

/// 按 skip/limit 读一页提交。需求五第 9 条：10 万级仓库禁止全量拉取。
///
/// `filter` 为空时就是原来那条路径：一次 `log` 带 `--skip/-n`。带筛选时：
/// git 认识的条件交给 `log`/`rev-list`（总数也用它数，需求 7.7 要求两边一致），
/// type 与合规判定在解析层做，所以要分片往前扫。
///
/// `spec` 只服务于合规判定；不筛合规时传什么都不会被读到。
pub fn list(
    repo: &Path,
    skip: usize,
    limit: usize,
    filter: &Filter,
    spec: &Spec,
) -> Result<CommitPage, GitError> {
    // 空仓库没有 HEAD。这是需要区分的正常状态，不是错误——返回空页。
    if filter.rev.is_none() {
        let head = process::run(Some(repo), &["rev-parse", "--verify", "-q", "HEAD"])?;
        if !head.success {
            return Ok(CommitPage {
                commits: Vec::new(),
                total: 0,
                truncated: false,
            });
        }
    }

    let total = count(repo, filter)?;
    let revision = filter.rev.clone().unwrap_or_else(|| "HEAD".to_string());

    if !filter.needs_post_filter() {
        let commits = log_window(repo, filter, &revision, skip, limit)?;
        return Ok(CommitPage {
            commits,
            total,
            truncated: false,
        });
    }

    let mut page = Vec::new();
    let mut wanted = skip;
    let mut scanned = 0usize;
    let mut truncated = false;

    while page.len() < limit {
        let chunk = log_window(repo, filter, &revision, scanned, SCAN_CHUNK)?;
        if chunk.is_empty() {
            break;
        }
        scanned += chunk.len();
        for commit in &chunk {
            if !keeps(commit, filter, spec) {
                continue;
            }
            if wanted > 0 {
                wanted -= 1;
                continue;
            }
            page.push(commit.clone());
            if page.len() == limit {
                break;
            }
        }
        if scanned >= SCAN_MAX_ROWS {
            truncated = true;
            break;
        }
        // 最后一页本来就不满，再来一次只会拿到空结果
        if chunk.len() < SCAN_CHUNK {
            break;
        }
    }

    Ok(CommitPage {
        commits: page,
        total,
        truncated,
    })
}

/// 筛选态下的可见集合，交给图算泳道（§7.7）。
///
/// **顺序必须是 `--topo-order`**：子一定在父之前。取的是可见集合的子集，
/// topo 序的子集仍是 topo 序，所以图算得出来。被筛掉的父记成 `dangling`。
pub fn visible_nodes(
    repo: &Path,
    filter: &Filter,
    spec: &Spec,
) -> Result<Vec<super::graph::Node>, GitError> {
    let revision = filter.rev.clone().unwrap_or_else(|| "HEAD".to_string());
    // 分片扫描上限与列表一致：两边的可见集合必须同一份，不能一个看到头一个看不到
    let mut commits: Vec<Commit> = Vec::new();
    let mut scanned = 0usize;
    loop {
        let chunk = log_window(repo, filter, &revision, scanned, SCAN_CHUNK)?;
        if chunk.is_empty() {
            break;
        }
        scanned += chunk.len();
        commits.extend(
            chunk
                .iter()
                .filter(|commit| keeps(commit, filter, spec))
                .cloned(),
        );
        if scanned >= SCAN_MAX_ROWS || commits.len() >= SCAN_MAX_ROWS || chunk.len() < SCAN_CHUNK {
            break;
        }
    }

    let visible: std::collections::HashSet<&str> =
        commits.iter().map(|commit| commit.id.as_str()).collect();
    Ok(commits
        .iter()
        .map(|commit| super::graph::Node {
            sha: commit.id.clone(),
            parents: commit
                .parents
                .iter()
                .filter(|parent| visible.contains(parent.as_str()))
                .cloned()
                .collect(),
            dangling: commit
                .parents
                .iter()
                .any(|parent| !visible.contains(parent.as_str())),
        })
        .collect())
}

/// 解析层判定：type 白名单 + 合规。git 不认这两样，所以只能在我们这边判。
///
/// 合规判定走 `check::evaluate`，与提交表单、commit-msg hook 共用同一份规则（需求 6.7）——
/// 筛选出来的"合规"集合和表单里亮绿灯的必须是同一个定义。
fn keeps(commit: &Commit, filter: &Filter, spec: &Spec) -> bool {
    if !filter.types.is_empty() {
        let Some(commit_type) = commit.summary.commit_type.as_deref() else {
            return false;
        };
        if !filter
            .types
            .iter()
            .any(|wanted| wanted.eq_ignore_ascii_case(commit_type))
        {
            return false;
        }
    }
    match filter.conformant {
        Some(wanted) => check::evaluate(spec, &commit.subject, &commit.body).conformant == wanted,
        None => true,
    }
}

/// 总数。必须带**同一组**筛选条件跑，否则"共 N 条"与实得条数对不上（需求 7.7）。
///
/// `rev-list` 不认 `--format/-n/--skip`，所以这里单独拼一套：git 认识的条件是一样的。
fn count(repo: &Path, filter: &Filter) -> Result<usize, GitError> {
    let revision = filter.rev.clone().unwrap_or_else(|| "HEAD".to_string());
    let mut args = vec!["rev-list", "--count"];
    args.extend(condition_args(filter));
    args.push(revision.as_str());
    if let Some(path) = filter.path.as_deref() {
        args.push("--");
        args.push(path);
    }
    let out = process::run(Some(repo), &args)?.expect_success()?;
    parse_count(out.trim())
}

/// 一次 `log`，取 [offset, offset+limit) 这扇窗口。筛选条件原样传下去。
fn log_window(
    repo: &Path,
    filter: &Filter,
    revision: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<Commit>, GitError> {
    let format = format!("--format={}", FIELDS.join(FIELD_HEX));
    let offset = offset.to_string();
    let limit = limit.to_string();
    let mut args = vec![
        // --topo-order：子一定在父之前，且一条支线不被日期切散。
        // 图列（git/graph.rs）按同一个顺序算泳道，两边顺序必须一致，
        // 否则第 N 行的连线会接到隔壁那行身上。
        "log",
        "-z",
        "--topo-order",
        // 短形式下本地分支 feat/x 和远程 feat/x 长得一模一样（实测），
        // 徽标要分得开类别就只有限定完整 refname 这一条路。
        "--decorate=full",
    ];
    args.push(&format);
    args.extend(condition_args(filter));
    args.push("--skip");
    args.push(&offset);
    args.push("-n");
    args.push(&limit);
    args.push(revision);
    if let Some(path) = filter.path.as_deref() {
        args.push("--");
        args.push(path);
    }

    let stdout = process::run(Some(repo), &args)?.expect_success()?;
    parse(&stdout)
}

/// git 认识的那几项筛选。两个调用点（`log` / `rev-list`）共用，避免两边漏传一个条件
/// 导致总数和列表对不上。
fn condition_args(filter: &Filter) -> Vec<&str> {
    let mut args = Vec::new();
    for author in &filter.authors {
        if author.trim().is_empty() {
            continue;
        }
        args.push("--author");
        args.push(author.as_str());
    }
    for keyword in &filter.grep {
        if keyword.trim().is_empty() {
            continue;
        }
        args.push("--grep");
        args.push(keyword.as_str());
    }
    if let Some(since) = filter.since.as_deref().filter(|s| !s.trim().is_empty()) {
        args.push("--since");
        args.push(since);
    }
    if let Some(until) = filter.until.as_deref().filter(|s| !s.trim().is_empty()) {
        args.push("--until");
        args.push(until);
    }
    args
}

/// 读一条提交本身（标题/作者/正文/父）。
///
/// 列表页已经有行数据，不必再取一次；从文件历史、blame 这些"从别处跳过来"的地方
/// 才会用到那里——那条提交不在已读出的那几页里，没有这一条就只有一个 sha 和一句标题。
pub fn show(repo: &Path, sha: &str) -> Result<Commit, GitError> {
    let format = format!("--format={}", FIELDS.join(FIELD_HEX));
    let stdout = process::run(
        Some(repo),
        &[
            "log",
            "-z",
            "--topo-order",
            "--decorate=full",
            &format,
            "-n",
            "1",
            sha,
        ],
    )?
    .expect_success()?;

    parse(&stdout)?
        .into_iter()
        .next()
        .ok_or(GitError::ParseFailure {
            snippet: process::snippet(&stdout),
        })
}

/// 某个文件的全部改动（§7.6）。`--follow` 让历史跟着改名走，所以要一次只传一个路径。
///
/// 返回值结构与列表页一样（含总数与分页），界面上就当成一个筛窄了的列表用。
pub fn file_history(
    repo: &Path,
    path: &str,
    skip: usize,
    limit: usize,
) -> Result<CommitPage, GitError> {
    let format = format!("--format={}", FIELDS.join(FIELD_HEX));
    let offset = skip.to_string();
    let count = limit.to_string();
    let pathspec = format!(":(literal){path}");
    let stdout = process::run(
        Some(repo),
        &[
            "log",
            "-z",
            "--topo-order",
            "--decorate=full",
            "--follow",
            &format,
            "--skip",
            &offset,
            "-n",
            &count,
            "--",
            &pathspec,
        ],
    )?
    .expect_success()?;

    Ok(CommitPage {
        commits: parse(&stdout)?,
        // --follow 的总数只能整个数出来（限制要跟着 --follow 一起用才有意义），
        // 所以这里只报本页条数——"共 N 条"留给列表页说，文件历史页说"已列出多少条"
        total: 0,
        truncated: false,
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
        parents: parts[4].split_whitespace().map(str::to_string).collect(),
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
        commit_at(dir, file, msg, None);
    }

    /// 带指定作者/提交时间的提交（`date` 形如 `2020-01-02T03:04:05+08:00`）。
    /// 靠 `GIT_AUTHOR_DATE`/`GIT_COMMITTER_DATE` 环境变量注入，而 %at / %ct 读的就是它们。
    fn commit_at(dir: &Path, file: &str, msg: &str, date: Option<&str>) {
        // 内容里拼上标题：同一个文件反复提交时内容要变，否则第二次 commit 是空的
        fs::write(dir.join(file), format!("{file}: {msg}\n")).expect("write");
        git_in(dir, &["add", file]);
        let args = vec!["commit", "-q", "-m", msg];
        match date {
            Some(date) => {
                let out = process::run_with_env(
                    Some(dir),
                    &args,
                    &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
                )
                .expect("spawn git");
                assert!(out.success, "commit 失败：{}", out.stderr);
            }
            None => {
                git_in(dir, &args);
            }
        }
    }

    /// 测试里绝大多数用例不筛选：给一个空 filter 与默认规范，
    /// 让用例主体继续只关心分页与解析
    fn plain(dir: &Path, skip: usize, limit: usize) -> CommitPage {
        list(dir, skip, limit, &Filter::default(), &Spec::default()).expect("list ok")
    }

    fn subject_of(dir: &Path, skip: usize, limit: usize) -> Vec<String> {
        plain(dir, skip, limit)
            .commits
            .into_iter()
            .map(|c| c.subject)
            .collect()
    }

    #[test]
    fn empty_repo_is_an_empty_page_not_an_error() {
        let dir = repo();
        let page = plain(dir.path(), 0, 200);
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

        let page = plain(dir.path(), 0, 200);
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

        let first = plain(dir.path(), 0, 2);
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

        let page = plain(dir.path(), 0, 10);
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

        let page = plain(dir.path(), 0, 10);
        let listed: Vec<&str> = page.commits.iter().map(|c| c.id.as_str()).collect();
        let walked: Vec<String> = crate::git::graph::history(dir.path())
            .expect("读父子")
            .into_iter()
            .map(|node| node.sha)
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

        let page = plain(dir.path(), 0, 10);
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

        let page = plain(dir.path(), 0, 10);
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
    /// 带筛选的读法：给一组条件 + 一个规范，拿到一页结果
    fn filtered_list(
        dir: &Path,
        filter: Filter,
        spec: Spec,
        skip: usize,
        limit: usize,
    ) -> CommitPage {
        list(dir, skip, limit, &filter, &spec).expect("筛选读列表 ok")
    }

    #[test]
    fn an_empty_filter_takes_the_single_process_path() {
        assert!(Filter::default().is_empty());
        assert!(!Filter {
            grep: vec!["修复".into()],
            ..Filter::default()
        }
        .is_empty());
        assert!(
            Filter {
                conformant: Some(false),
                ..Filter::default()
            }
            .needs_post_filter(),
            "合规判定 git 不认识，必须在解析层做"
        );
        assert!(
            !Filter {
                grep: vec!["修复".into()],
                ..Filter::default()
            }
            .needs_post_filter(),
            "只有 git 不认识的条件下才需要解析层动手"
        );
    }

    /// 需求 7.7 的硬要求："共 N 条"必须带同一组条件数
    #[test]
    fn the_total_counts_the_filtered_set_too() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 甲");
        commit(dir.path(), "b.txt", "fix: 乙");
        commit(dir.path(), "c.txt", "chore: 丙");

        let all = filtered_list(dir.path(), Filter::default(), Spec::default(), 0, 10);
        assert_eq!(all.total, 3);

        let author = filtered_list(
            dir.path(),
            Filter {
                authors: vec!["测试者".into()],
                ..Filter::default()
            },
            Spec::default(),
            0,
            10,
        );
        assert_eq!(author.total, 3, "作者条件命中全部三条");
        assert_eq!(author.commits.len(), 3);

        let nobody = filtered_list(
            dir.path(),
            Filter {
                authors: vec!["不存在的人".into()],
                ..Filter::default()
            },
            Spec::default(),
            0,
            10,
        );
        assert_eq!(nobody.total, 0, "筛空的集合总数也该是 0，不能报全量");
        assert!(nobody.commits.is_empty());
    }

    #[test]
    fn keywords_reach_gits_own_grep() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 支持中文标题");
        commit(dir.path(), "b.txt", "fix: 修一下另一个问题");

        let page = filtered_list(
            dir.path(),
            Filter {
                grep: vec!["中文".into()],
                ..Filter::default()
            },
            Spec::default(),
            0,
            10,
        );
        assert_eq!(page.commits.len(), 1);
        assert_eq!(page.commits[0].subject, "feat: 支持中文标题");
        assert_eq!(page.total, 1);
    }

    /// 时间范围同样走 git 的 `--since/--until`，git 自己认这个格式
    #[test]
    fn a_time_window_is_passed_through_to_git() {
        let dir = repo();
        commit_at(dir.path(), "old.txt", "feat: 很早以前", Some("2020-01-02T03:04:05+00:00"));
        commit_at(dir.path(), "new.txt", "feat: 最近", None);

        let recent = filtered_list(
            dir.path(),
            Filter {
                since: Some("2024-01-01".into()),
                ..Filter::default()
            },
            Spec::default(),
            0,
            10,
        );
        assert_eq!(recent.commits.len(), 1, "只该命中最近那条");
        assert_eq!(recent.commits[0].subject, "feat: 最近");
        assert_eq!(recent.total, 1, "总数也要带同一组时间条件");

        let old = filtered_list(
            dir.path(),
            Filter {
                until: Some("2024-01-01".into()),
                ..Filter::default()
            },
            Spec::default(),
            0,
            10,
        );
        assert_eq!(old.commits.len(), 1);
        assert_eq!(old.commits[0].subject, "feat: 很早以前");
    }

    /// type 与合规是解析层过滤（git 不认识），所以翻页要跨着被筛掉的那些条目数
    #[test]
    fn type_and_conformance_are_filtered_while_paging() {
        let dir = repo();
        for subject in [
            "feat: 甲",
            "随手改的",
            "fix: 乙",
            "随手改的",
            "chore: 丙",
            "随手改的",
        ] {
            commit(dir.path(), "f.txt", subject);
        }
        let spec = Spec::default();

        let feats = filtered_list(
            dir.path(),
            Filter {
                types: vec!["feat".into()],
                ..Filter::default()
            },
            spec.clone(),
            0,
            10,
        );
        assert_eq!(feats.total, 6, "总数是 git 数的那一组条件，type 不在里面");
        assert_eq!(
            feats.commits
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["feat: 甲"]
        );

        // 第二页要跨过三条非 feat 之后才拿得到下一条可见的
        let fixes_page_two = filtered_list(
            dir.path(),
            Filter {
                types: vec!["fix".into()],
                ..Filter::default()
            },
            spec.clone(),
            1,
            10,
        );
        assert!(
            fixes_page_two.commits.is_empty(),
            "只有一条 fix，第二页就该是空的：{:?}",
            fixes_page_two
                .commits
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>()
        );

        let conformant = filtered_list(
            dir.path(),
            Filter {
                conformant: Some(true),
                ..Filter::default()
            },
            spec.clone(),
            0,
            10,
        );
        assert_eq!(conformant.commits.len(), 3, "三条规范提交");

        let bad = filtered_list(
            dir.path(),
            Filter {
                conformant: Some(false),
                ..Filter::default()
            },
            spec,
            0,
            10,
        );
        assert_eq!(bad.commits.len(), 3, "三条非规范提交");
        assert!(bad.commits.iter().all(|c| c.summary.commit_type.is_none()));
    }

    /// 筛选后的可见集合拿去做图：子一定在父之前，被筛掉的父记成截断边
    #[test]
    fn the_visible_set_is_a_topological_walk_with_dangling_parents() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        commit(dir.path(), "b.txt", "随手改的");
        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        commit(dir.path(), "c.txt", "随手改的");
        git_in(dir.path(), &["checkout", "-q", "-"]);
        commit(dir.path(), "d.txt", "feat: 乙");

        let nodes = visible_nodes(
            dir.path(),
            &Filter {
                types: vec!["feat".into()],
                ..Filter::default()
            },
            &Spec::default(),
        )
        .expect("可见集合");

        assert_eq!(nodes.len(), 2, "只有两条 feat 可见");
        assert!(
            nodes[0].dangling,
            "最新那条的父是 side 上被筛掉的一条，所以那段边没有落点"
        );
        assert!(
            !nodes[1].dangling,
            "基线是根提交：它下面根本没有边可截断"
        );
        assert!(nodes[1].parents.is_empty());
        assert!(
            nodes.iter().all(|node| node.parents.iter().all(|parent| nodes
                .iter()
                .any(|other| &other.sha == parent))),
            "可见节点里不该还挂着不可见的父"
        );
    }

    /// 文件历史要跟着改名走（§7.6）：`--follow` 前后条数与 `git log --follow --stat` 一致
    #[test]
    fn a_file_history_follows_a_rename() {
        let dir = repo();
        commit(dir.path(), "old.txt", "feat: 改名之前");
        git_in(dir.path(), &["mv", "old.txt", "new.txt"]);
        git_in(dir.path(), &["commit", "-q", "-m", "refactor: 改了名字"]);
        commit(dir.path(), "other.txt", "feat: 别的文件");

        let page = file_history(dir.path(), "new.txt", 0, 50).expect("文件历史");
        assert_eq!(
            page.commits
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["refactor: 改了名字", "feat: 改名之前"],
            "改名前后的改动都要在，同名文件的那条不该算进来"
        );

        // 带改名的路径不会误伤其他文件：文件名里的 * 不当通配
        assert!(file_history(dir.path(), "other.txt", 0, 50)
            .expect("文件历史")
            .commits
            .len()
            == 1);
    }

    #[test]
    #[ignore = "需要一个 5 万提交的本地 fixture 仓库"]
    fn first_page_on_a_large_repo_stays_under_a_second() {
        let Ok(path) = std::env::var("GIT_TIDY_BENCH_REPO") else {
            panic!("未设置 GIT_TIDY_BENCH_REPO，无法做首屏量测");
        };
        let repo = Path::new(&path);

        let started = std::time::Instant::now();
        let page = plain(repo, 0, 200);
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
