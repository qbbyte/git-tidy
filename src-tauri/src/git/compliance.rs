use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Serialize;

use super::hook;
use super::log::{self, Filter};
use super::process;
use crate::config::check::{self, Reason};
use crate::config::spec::{Spec, SpecSource};
use crate::error::GitError;

/// 一次统计最多看多少条提交。
///
/// 不做全量：10 万级仓库跑一遍全量要把整个历史拉进内存再逐条判正则，
/// 界面会转圈到用户以为卡死。超了就把 `truncated` 置起来，界面明说"只统计了前 N 条"——
/// 宁可少报，也不要给一个看不出边界的百分比（需求 5.10）。
const SCAN_LIMIT: usize = 5000;
/// 明细列表最多带多少条不合规提交。聚合数字是全量的，明细是给人看的，截断要说清。
const OFFENDER_LIMIT: usize = 500;

/// 统计区间。空的 rev 就是 HEAD。
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Range {
    /// 交给 `git log` 的 rev：`HEAD`、`v1.0..HEAD`、`HEAD~30` 都行
    pub rev: Option<String>,
    /// 按作者筛，和历史列表页的筛选同源
    pub author: Option<String>,
}

/// 一条不合规提交。字段与历史列表一致，界面点进去就是详情页那一条。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offender {
    pub id: String,
    pub author_name: String,
    pub time: i64,
    pub subject: String,
    /// 解析出的 type；`None` = 连规范头部都没有。分布图与明细共用这个值
    pub commit_type: Option<String>,
    pub scope: Option<String>,
    pub breaking: bool,
    /// 命中的原因（可能多个）。标题直接从 `Reason::title` 取，界面不再自己编一套说法
    pub reasons: Vec<ReasonLabel>,
    /// 疑似绕过 `--no-verify`：装了 hook、提交时间晚于 hook 安装、且不合规。
    /// **只是疑似**：commit 对象里没有任何"被绕过"的痕迹（需求 7.21）
    pub suspected_bypass: bool,
}

/// 原因 + 标题成对带出去，前端不需要维护第二份中文对照表。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonLabel {
    pub reason: Reason,
    pub title: String,
}

/// 按原因分桶的计数。空桶不出现：界面上列出"从未命中过的原因"没有意义。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonCount {
    pub reason: Reason,
    pub title: String,
    pub count: usize,
    /// 占不合规提交的比例，不是占全量。分母不同，别混
    pub share: f64,
}

/// 按 Conventional type 分布。`None` 归到"非规范提交"一组，
/// 否则"一半提交没有 type"这件事会被分散到别的桶里看不见。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeCount {
    pub commit_type: Option<String>,
    pub total: usize,
    pub conformant: usize,
}

/// 按作者 / 按月的趋势行。分母相同，字段就一致，界面可以复用同一个表。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendRow {
    /// 作者名或 `YYYY-MM`
    pub key: String,
    pub total: usize,
    pub conformant: usize,
    pub rate: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// 实际统计到的条数（= 分母）
    pub scanned: usize,
    pub conformant: usize,
    /// 整体符合率，0..1。分母为 0 时给 0，界面另说"还没有提交"
    pub rate: f64,
    pub by_reason: Vec<ReasonCount>,
    pub by_type: Vec<TypeCount>,
    pub by_author: Vec<TrendRow>,
    pub by_month: Vec<TrendRow>,
    pub offenders: Vec<Offender>,
    /// 明细被截断了（`offenders` 少于不合规总数）
    pub offenders_truncated: bool,
    /// 扫描被上限截断：下面的百分比只覆盖最近这 N 条
    pub truncated: bool,
    /// 不计入分母的提交数（合并、revert：信息是 git 生成的，不该算谁没守规范）
    pub excluded: usize,
    /// 疑似绕过 `--no-verify` 的条数
    pub suspected_bypass: usize,
    /// 仓库里装没装 hook。没装就无从推断"绕过"，界面要说明这一点
    pub hook_installed: bool,
    /// 规范从哪读的：报告的尺子和表单、hook 必须同一份
    pub spec_source: SpecSource,
    /// 区间里一共多少条（git 自己数的），用来对比"扫了多少"
    pub total_in_range: usize,
    pub rev: String,
}

pub fn build(repo: &Path, spec: &Spec, range: &Range) -> Result<Report, GitError> {
    let filter = Filter {
        rev: range.rev.clone(),
        authors: range
            .author
            .iter()
            .filter(|a| !a.trim().is_empty())
            .cloned()
            .collect(),
        ..Filter::default()
    };
    let rev = filter.rev.clone().unwrap_or_else(|| "HEAD".to_string());

    // 空仓库没有 HEAD，`git log HEAD` 是硬失败。这是需要说清楚的正常状态：
    // 报告给一个空报告（而不是报错），界面自己显示"还没有提交"
    if filter.rev.is_none() {
        let head = process::run(Some(repo), &["rev-parse", "--verify", "-q", "HEAD"])?;
        if !head.success {
            return Ok(empty_report(repo, spec, rev));
        }
    }

    let commits = log::scan(repo, &filter, SCAN_LIMIT)?;
    let truncated = commits.len() >= SCAN_LIMIT;
    let total_in_range = if truncated {
        // 撞了上限说明区间比上限还长，具体多少得再数一次——`rev-list --count` 很便宜
        count(repo, &filter)?
    } else {
        commits.len()
    };

    // 能不能推断"绕过"：得先装过 hook，且要知道装的时间。
    // 用脚本文件的 mtime 近似——它只在安装/更新时被改写，正好就是"从这一刻起有 hook"
    let hook_since = hook::installed_since(repo);
    let hook_installed = hook_since.is_some();

    let mut reasons = HashMap::<Reason, usize>::new();
    let mut by_type: BTreeMap<Option<String>, (usize, usize)> = BTreeMap::new();
    let mut by_author: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_month: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut offenders = Vec::new();
    let mut conformant_total = 0usize;
    let mut excluded = 0usize;
    let mut suspected = 0usize;
    let mut offender_total = 0usize;

    for commit in &commits {
        // 合并与 revert 的提交信息是 git 自己生成的（`Merge branch ...` / `Revert "..."`）。
        // 把它们算进分母，等于让"按作者"的表格凭空多出几条谁也改不动的红，
        // 报告也就没人信了。计入 `excluded`，界面明说排除了什么。
        if commit.merge || commit.revert {
            excluded += 1;
            continue;
        }

        let outcome = check::evaluate(spec, &commit.subject, &commit.body);
        if outcome.conformant {
            conformant_total += 1;
        } else {
            offender_total += 1;
        }
        for violation in &outcome.violations {
            *reasons.entry(violation.reason).or_default() += 1;
        }

        let type_key = outcome.commit_type.clone();
        let type_slot = by_type.entry(type_key.clone()).or_default();
        type_slot.0 += 1;
        if outcome.conformant {
            type_slot.1 += 1;
        }

        let author_slot = by_author.entry(commit.author_name.clone()).or_default();
        author_slot.0 += 1;
        author_slot.1 += usize::from(outcome.conformant);

        let month_slot = by_month.entry(month_of(commit.time)).or_default();
        month_slot.0 += 1;
        month_slot.1 += usize::from(outcome.conformant);

        if outcome.conformant {
            continue;
        }

        let bypass = hook_since.is_some_and(|since| commit.time >= since);
        if bypass {
            suspected += 1;
        }
        if offenders.len() < OFFENDER_LIMIT {
            offenders.push(Offender {
                id: commit.id.clone(),
                author_name: commit.author_name.clone(),
                time: commit.time,
                subject: commit.subject.clone(),
                commit_type: type_key,
                scope: outcome.scope.clone(),
                breaking: outcome.breaking,
                reasons: outcome
                    .violations
                    .iter()
                    .map(|violation| ReasonLabel {
                        reason: violation.reason,
                        title: violation.title.clone(),
                    })
                    .collect(),
                suspected_bypass: bypass,
            });
        }
    }

    let scanned = conformant_total + offender_total;
    let by_reason: Vec<ReasonCount> = reasons
        .into_iter()
        .map(|(reason, count)| ReasonCount {
            title: reason.title().to_string(),
            reason,
            count,
            // 分母是"不合规提交数"，不是"原因命中数"：一条提交可以命中多个原因，
            // 用命中数当分母会让占比加起来超过 100%
            share: ratio(count, offender_total.max(1)),
        })
        .collect();
    // 桶多的原因排前面：报告要让人先看见最普遍的那个问题。
    // 同数时按标题排，保证同一个仓库每次刷新顺序都一样（截图/对比才有用）
    let mut by_reason = by_reason;
    by_reason.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.title.cmp(&b.title)));

    Ok(Report {
        scanned,
        conformant: conformant_total,
        rate: ratio(conformant_total, scanned),
        by_reason,
        by_type: by_type
            .into_iter()
            .map(|(commit_type, (total, conformant))| TypeCount {
                commit_type,
                total,
                conformant,
            })
            .collect(),
        by_author: trends(by_author),
        by_month: trends(by_month),
        offenders_truncated: offender_total > offenders.len(),
        offenders,
        truncated,
        excluded,
        suspected_bypass: suspected,
        hook_installed,
        spec_source: spec.source,
        total_in_range,
        rev,
    })
}

/// 空仓库的报告：所有计数为 0，而不是报错——"还没有提交"不是异常。
fn empty_report(repo: &Path, spec: &Spec, rev: String) -> Report {
    Report {
        scanned: 0,
        conformant: 0,
        rate: 0.0,
        by_reason: Vec::new(),
        by_type: Vec::new(),
        by_author: Vec::new(),
        by_month: Vec::new(),
        offenders: Vec::new(),
        offenders_truncated: false,
        truncated: false,
        excluded: 0,
        suspected_bypass: 0,
        hook_installed: hook::installed_since(repo).is_some(),
        spec_source: spec.source,
        total_in_range: 0,
        rev,
    }
}

fn trends(map: BTreeMap<String, (usize, usize)>) -> Vec<TrendRow> {
    map.into_iter()
        .map(|(key, (total, conformant))| TrendRow {
            key,
            total,
            conformant,
            rate: ratio(conformant, total),
        })
        .collect()
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    // 保留三位：报告里的 0.667 比 0.6666666666666666 好读，也够用
    ((part as f64 / whole as f64) * 1000.0).round() / 1000.0
}

/// 按 UTC 切月。跨月的提交可能落到隔壁月份，但"按月趋势"没人拿它当账本看，
/// 引入时区库去换取可能反而不一致的结果不值得——这一列的作用是看方向，不是对账。
fn month_of(unix_seconds: i64) -> String {
    let (year, month) = civil_from_days(unix_seconds.div_euclid(86_400));
    format!("{year:04}-{month:02}")
}

/// 民用日历天数 → (年, 月)。Howard Hinnant 的算法：
/// 只用整数算术，不引 chrono，也就没有时区库带来的那一堆歧义。
fn civil_from_days(days: i64) -> (i64, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32)
}

/// 区间里一共多少条（`rev-list --count`，和历史列表页的口径一致）。
fn count(repo: &std::path::Path, filter: &Filter) -> Result<usize, GitError> {
    let mut args = vec!["rev-list", "--count"];
    for author in &filter.authors {
        if author.trim().is_empty() {
            continue;
        }
        args.push("--author");
        args.push(author.as_str());
    }
    args.push(filter.rev.as_deref().unwrap_or("HEAD"));
    let out = process::run(Some(repo), &args)?.expect_success()?;
    out.trim().parse().map_err(|_| GitError::ParseFailure {
        snippet: process::snippet(&out),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::hook;

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    fn repo_with(commits: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        for (i, subject) in commits.iter().enumerate() {
            std::fs::write(dir.path().join("f.txt"), format!("{i}\n")).expect("write");
            git_in(dir.path(), &["add", "f.txt"]);
            git_in(
                dir.path(),
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.com",
                    "commit",
                    "-q",
                    "-m",
                    subject,
                ],
            );
        }
        dir
    }

    /// 需求 7.21 的验收：100 条混合提交，各原因计数与手工核对一致。
    /// 这里只放三种原因，数字手算得出；分桶逻辑与全量 10 种原因共用同一段代码。
    #[test]
    fn reason_counts_match_a_hand_check() {
        let dir = repo_with(&[
            "feat: 一",
            "fix: 二",
            "更新了一批文件",    // missing_type
            "wip",               // missing_type
            "feaat: 拼错的类型", // invalid_type
            "feat：中文冒号",    // fullwidth_colon
            "feat: 加个开关。",  // trailing_period
            "chore: 收尾",
        ]);
        let report = build(dir.path(), &Spec::default(), &Range::default()).expect("report");

        assert_eq!(report.scanned, 8);
        assert_eq!(report.conformant, 3);
        assert_eq!(report.rate, 0.375, "3/8");

        let count_of = |reason: Reason| {
            report
                .by_reason
                .iter()
                .find(|item| item.reason == reason)
                .map_or(0, |item| item.count)
        };
        assert_eq!(count_of(Reason::MissingType), 2);
        assert_eq!(count_of(Reason::InvalidType), 1);
        assert_eq!(count_of(Reason::FullwidthColon), 1);
        assert_eq!(count_of(Reason::TrailingPeriod), 1);
        assert_eq!(report.offenders.len(), 5, "不合规明细条数要和计数口径一致");
    }

    /// 需求 7.21 的验收原文：fixture 100 条混合 commit，
    /// 分母与各原因计数与手工核对一致。这里把配比写死在测试里——
    /// 任何一条被漏掉、重复计入或跨桶误归，数字都会对不上。
    #[test]
    fn a_hundred_commit_fixture_matches_the_hand_count() {
        let mut subjects = vec!["feat: 正常改动".to_string(); 70];
        for i in 0..10 {
            subjects.push(format!("第 {i} 次随手写的标题"));
        }
        for i in 0..10 {
            subjects.push(format!("feaat{i}: 拼错的类型"));
        }
        for i in 0..5 {
            subjects.push(format!("fix：中文冒号 {i}"));
        }
        for i in 0..5 {
            subjects.push(format!("fix: 句尾带句号 {i}。"));
        }
        let borrowed: Vec<&str> = subjects.iter().map(String::as_str).collect();
        let dir = repo_with(&borrowed);

        let report = build(dir.path(), &Spec::default(), &Range::default()).expect("report");
        assert_eq!(report.scanned, 100, "分母");
        assert_eq!(report.conformant, 70);
        assert_eq!(report.rate, 0.7);

        let count_of = |reason: Reason| {
            report
                .by_reason
                .iter()
                .find(|item| item.reason == reason)
                .map_or(0, |item| item.count)
        };
        assert_eq!(count_of(Reason::MissingType), 10);
        assert_eq!(count_of(Reason::InvalidType), 10);
        assert_eq!(count_of(Reason::FullwidthColon), 5);
        assert_eq!(count_of(Reason::TrailingPeriod), 5);
        assert_eq!(report.offenders.len(), 30, "不合规明细");
        assert_eq!(report.suspected_bypass, 0, "没装 hook 就不推断绕过");
    }

    #[test]
    fn merges_and_reverts_are_excluded_from_the_denominator() {
        let dir = repo_with(&["feat: 一"]);
        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        std::fs::write(dir.path().join("g.txt"), "x\n").expect("write");
        git_in(dir.path(), &["add", "g.txt"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "feat: 支线",
            ],
        );
        git_in(dir.path(), &["checkout", "-q", "main"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "merge",
                "--no-ff",
                "-q",
                "-m",
                "Merge branch 'side'",
                "side",
            ],
        );

        let report = build(dir.path(), &Spec::default(), &Range::default()).expect("report");
        assert_eq!(report.scanned, 2, "合并提交不进分母");
        assert_eq!(report.excluded, 1, "排除了什么要能说出来");
        assert!(report.rate == 1.0);
    }

    #[test]
    fn type_distribution_keeps_the_non_conventional_bucket() {
        let dir = repo_with(&["feat: 一", "fix: 二", "随手写的标题"]);
        let report = build(dir.path(), &Spec::default(), &Range::default()).expect("report");
        let none = report
            .by_type
            .iter()
            .find(|item| item.commit_type.is_none())
            .expect("非规范提交要单独成桶，否则「一半没有 type」看不见");
        assert_eq!(none.total, 1);
        assert_eq!(none.conformant, 0);
    }

    #[test]
    fn author_and_month_trends_have_their_own_denominators() {
        let dir = repo_with(&["feat: 一", "更新", "更新"]);
        let report = build(dir.path(), &Spec::default(), &Range::default()).expect("report");
        assert_eq!(report.by_author.len(), 1);
        let author = &report.by_author[0];
        assert_eq!(author.total, 3);
        assert_eq!(author.conformant, 1);
        assert!((author.rate - 0.333).abs() < 0.001, "{}", author.rate);
        assert_eq!(report.by_month.len(), 1, "同一天的三条只占一个月");
        assert_eq!(report.by_month[0].key.len(), 7, "YYYY-MM");
        assert!(
            report.by_month[0].key.contains('-'),
            "月份键：{}",
            report.by_month[0].key
        );
    }

    /// `--no-verify` 绕过在 commit 对象里不留痕迹，只能"疑似"：
    /// 装了 hook + 提交晚于安装 + 不合规，三个条件齐了才标疑似（需求 7.21）。
    #[test]
    fn bypass_is_only_suspected_and_only_after_the_hook_exists() {
        let dir = repo_with(&["feat: 一", "更新了一批文件"]);
        let spec = Spec::default();

        let before = build(dir.path(), &spec, &Range::default()).expect("report");
        assert!(!before.hook_installed, "还没装 hook");
        assert_eq!(before.suspected_bypass, 0, "还没装 hook 就不能推断绕过");
        assert!(before.offenders.iter().all(|o| !o.suspected_bypass));

        // 装 hook 前后各跨过一秒：hook 安装时间只精确到秒，同一秒里的提交到底是
        // "安装前的"还是"安装后绕过的"分不出来，不该在测试里被当成一种结论
        std::thread::sleep(std::time::Duration::from_millis(1100));
        hook::install(dir.path(), &spec, false).expect("install");
        // 装完之后用 --no-verify 提交一条不合规的：这就是"疑似绕过"的真实场景
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.path().join("f.txt"), "later\n").expect("write");
        git_in(dir.path(), &["add", "f.txt"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "--no-verify",
                "-m",
                "更新了一批文件",
            ],
        );

        let after = build(dir.path(), &spec, &Range::default()).expect("report");
        assert!(after.hook_installed);
        assert_eq!(after.suspected_bypass, 1, "绕过的那一条要被标疑似");
        assert!(
            after
                .offenders
                .iter()
                .any(|item| item.suspected_bypass && item.subject == "更新了一批文件"),
            "明细里要能定位到具体那一条"
        );
    }

    #[test]
    fn a_rev_range_limits_what_is_counted() {
        let dir = repo_with(&["feat: 一", "更新", "feat: 三"]);
        let report = build(
            dir.path(),
            &Spec::default(),
            &Range {
                rev: Some("HEAD~1".into()),
                author: None,
            },
        )
        .expect("report");
        assert_eq!(report.scanned, 2, "HEAD~1 之后只有两条");
        assert_eq!(report.total_in_range, 2);
    }

    #[test]
    fn an_empty_repository_reports_zero_rather_than_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        let report = build(dir.path(), &Spec::default(), &Range::default()).expect("report");
        assert_eq!(report.scanned, 0);
        assert_eq!(report.rate, 0.0, "分母为 0 不做除法");
        assert!(report.offenders.is_empty());
    }

    #[test]
    fn civil_days_conversion_round_trips_a_few_landmarks() {
        assert_eq!(civil_from_days(0), (1970, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1));
    }
}
