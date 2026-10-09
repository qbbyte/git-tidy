use std::collections::BTreeMap;

use serde::Serialize;

use super::log::{self, Filter};
use super::message;
use super::process;
use crate::config::check;
use crate::config::spec::Spec;
use crate::error::GitError;

/// 一次最多看多少条提交。与符合率报告同一条上限口径：
/// 不做全量——生成一份发布说明不需要把十万级仓库整个拉进内存。
const SCAN_LIMIT: usize = 5000;

/// 生成区间。`from` 为空表示"从根算起"（仓库还没有 tag，用户也没手选起点）。
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Range {
    pub from: Option<String>,
    /// 通常是 HEAD
    pub to: Option<String>,
}

impl Range {
    fn to(&self) -> &str {
        self.to
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or("HEAD")
    }

    /// 交给 `git log` 的 rev 串。`from..to` 与 `from` 是两种合法写法，别拼错。
    fn rev(&self) -> String {
        match self
            .from
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
        {
            Some(from) => format!("{from}..{}", self.to()),
            None => self.to().to_string(),
        }
    }
}

/// 一条会进日志的提交。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub sha: String,
    /// 去掉 `type(scope): ` 之后的描述——那部分才是发布说明要给人看的话
    pub description: String,
    pub commit_type: String,
    pub scope: Option<String>,
    /// type 后的 `!` 或 footer 里的 `BREAKING CHANGE:`，两者都算（需求 6.5）
    pub breaking: bool,
    pub author_name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    /// 章节名来自 `Spec.groups`（可被 `git-tidy.config.json` / `.versionrc` / `cliff.toml` 覆盖）
    pub heading: String,
    /// 没有 scope 的条目直接挂在章节下，有 scope 的再分二级
    pub entries: Vec<Entry>,
    pub scoped: Vec<ScopedGroup>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedGroup {
    pub scope: String,
    pub entries: Vec<Entry>,
}

/// 没进日志的提交。**必须单独计数并说清为什么**（需求 6.5）：
/// 静默丢掉就是在骗人——用户以为这段历史全覆盖了。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    /// 不符合本仓库规范：规范提交才进发布说明
    pub non_conformant: usize,
    /// 合并提交：信息是 git 生成的，写进日志只会变成一句废话
    pub merges: usize,
    /// 前若干条示例，界面上给一个跳符合率报告的入口
    pub samples: Vec<SkippedSample>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedSample {
    pub sha: String,
    pub subject: String,
    pub author_name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changelog {
    pub from: Option<String>,
    pub to: String,
    /// 区间里一共扫了多少条
    pub scanned: usize,
    /// 实际进日志的条数
    pub included: usize,
    pub breaking: Vec<Entry>,
    pub sections: Vec<Section>,
    pub skipped: Skipped,
    pub truncated: bool,
    /// 渲染好的 Markdown，界面直接预览 / 复制 / 写文件
    pub markdown: String,
    /// 目标文件当前内容的指纹。追加写入时拿它做乐观并发校验：
    /// 预览之后文件被别人改过就拒写，而不是把别人的改动冲掉
    pub target_digest: String,
}

/// 上一个 tag（`git describe --tags --abbrev=0`）。没有 tag 返回 None——
/// 那不是错误，是"请用户手选起点"（需求 6.5）。
pub fn previous_tag(repo: &std::path::Path, to: &str) -> Option<String> {
    let out = process::run(Some(repo), &["describe", "--tags", "--abbrev=0", to]).ok()?;
    out.success
        .then(|| out.stdout.trim().to_string())
        .filter(|tag| !tag.is_empty())
}

pub fn build(repo: &std::path::Path, spec: &Spec, range: &Range) -> Result<Changelog, GitError> {
    let filter = Filter {
        rev: Some(range.rev()),
        ..Filter::default()
    };
    let commits = log::scan(repo, &filter, SCAN_LIMIT)?;
    let truncated = commits.len() >= SCAN_LIMIT;

    let mut buckets: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    let mut scoped: BTreeMap<(String, String), Vec<Entry>> = BTreeMap::new();
    let mut breaking = Vec::new();
    let mut samples = Vec::new();
    let mut non_conformant = 0usize;
    let mut merges = 0usize;

    for commit in &commits {
        if commit.merge {
            merges += 1;
            continue;
        }
        let summary = message::summarize(&commit.subject, &commit.body);
        // 判定内核只有这一处：报告、hook、CHANGELOG 用的是同一把尺子
        if !check::evaluate(spec, &commit.subject, &commit.body).conformant {
            non_conformant += 1;
            if samples.len() < 20 {
                samples.push(SkippedSample {
                    sha: commit.id.clone(),
                    subject: commit.subject.clone(),
                    author_name: commit.author_name.clone(),
                });
            }
            continue;
        }

        let commit_type = summary
            .commit_type
            .clone()
            .unwrap_or_else(|| "other".to_string());
        let entry = Entry {
            sha: commit.id.clone(),
            description: strip_header(&commit.subject),
            commit_type: commit_type.clone(),
            scope: summary.scope.clone(),
            breaking: summary.breaking,
            author_name: commit.author_name.clone(),
        };
        if entry.breaking {
            breaking.push(entry.clone());
        }
        match &entry.scope {
            Some(scope) if !scope.is_empty() => scoped
                .entry((commit_type, scope.clone()))
                .or_default()
                .push(entry),
            _ => buckets.entry(commit_type).or_default().push(entry),
        }
    }

    let included = buckets.values().map(Vec::len).sum::<usize>()
        + scoped.values().map(Vec::len).sum::<usize>();

    // 章节顺序 = `Spec.groups` 的顺序；配置里没列到的 type 归到"其他"，
    // 不认识的 type 不会因为没分组就凭空消失
    let mut order: Vec<&str> = spec.groups.iter().map(|group| group.ty.as_str()).collect();
    order.push(OTHER_SECTION);
    let mut seen = Vec::new();
    order.retain(|ty| {
        if seen.contains(ty) {
            return false;
        }
        seen.push(ty);
        true
    });

    let mut sections = Vec::new();
    for ty in &order {
        let heading = spec
            .groups
            .iter()
            .find(|group| group.ty == **ty)
            .map(|group| group.section.clone())
            .unwrap_or_else(|| "其他".to_string());
        let entries = buckets.remove(*ty).unwrap_or_default();
        let scoped_entries: Vec<ScopedGroup> = scoped
            .iter()
            .filter(|((group_ty, _), _)| group_ty == ty)
            .map(|((_, scope), items)| ScopedGroup {
                scope: scope.clone(),
                entries: items.clone(),
            })
            .collect();
        if entries.is_empty() && scoped_entries.is_empty() {
            continue;
        }
        sections.push(Section {
            heading,
            entries,
            scoped: scoped_entries,
        });
    }

    // groups 里没出现过的 type 也不能丢：用户配了 feat/fix 之后随手写了 chore，
    // 它该落在"其他"里，而不是从日志里消失
    for (ty, entries) in &buckets {
        sections.push(Section {
            heading: ty.to_string(),
            entries: entries.clone(),
            scoped: Vec::new(),
        });
    }

    let changelog = Changelog {
        from: range.from.clone(),
        to: range.to().to_string(),
        scanned: commits.len(),
        included,
        breaking,
        sections,
        skipped: Skipped {
            non_conformant,
            merges,
            samples,
        },
        truncated,
        markdown: String::new(),
        target_digest: String::new(),
    };
    let mut changelog = changelog;
    changelog.markdown = render(&changelog);
    changelog.target_digest = target_digest(repo);
    Ok(changelog)
}

const OTHER_SECTION: &str = "other";

/// 渲染成 Markdown（需求 6.5：分组 + scope 二级 + Breaking 单独一节）。
///
/// 快照式测试直接逐行比对这个函数的输出，所以格式改动必须是有意的，
/// 而不是"顺手调一下排版"。
pub fn render(changelog: &Changelog) -> String {
    let title = match &changelog.from {
        Some(from) => format!("{from}..{}", changelog.to),
        None => changelog.to.clone(),
    };
    let mut out = String::from("# Changelog\n\n");
    out.push_str(&format!("## {title}\n\n"));

    if changelog.included == 0 {
        out.push_str("_这个区间里没有可收录的规范提交。_\n");
        return out;
    }

    if !changelog.breaking.is_empty() {
        out.push_str("### ⚠ Breaking Changes\n\n");
        for entry in &changelog.breaking {
            out.push_str(&bullet(entry));
        }
        out.push('\n');
    }

    for section in &changelog.sections {
        out.push_str(&format!("### {}\n\n", section.heading));
        for group in &section.scoped {
            out.push_str(&format!("#### {}\n\n", group.scope));
            for entry in &group.entries {
                out.push_str(&plain_bullet(entry));
            }
        }
        for entry in &section.entries {
            out.push_str(&bullet(entry));
        }
        // 章节之间空一行就够：每组之后再空一行会让 Markdown 里出现连续两个空行
        out.push('\n');
    }

    // 排除了什么必须写在文件里，而不是只留在界面上：导出出去的 CHANGELOG
    // 离开这个工具之后就没有人知道它漏了什么
    if changelog.skipped.non_conformant > 0 || changelog.skipped.merges > 0 {
        out.push_str("<!-- ");
        if changelog.skipped.non_conformant > 0 {
            out.push_str(&format!(
                "另有 {} 条不合规提交未收录（见符合率报告）",
                changelog.skipped.non_conformant
            ));
        }
        if changelog.skipped.merges > 0 {
            if changelog.skipped.non_conformant > 0 {
                out.push('；');
            }
            out.push_str(&format!("已排除 {} 条合并提交", changelog.skipped.merges));
        }
        out.push_str(" -->\n");
    }

    out
}

fn bullet(entry: &Entry) -> String {
    match &entry.scope {
        Some(scope) => format!(
            "- **{scope}**: {} (`{}`)\n",
            entry.description,
            short(&entry.sha)
        ),
        None => plain_bullet(entry),
    }
}

/// 已经有 `#### scope` 小节时，条目里再写一遍 scope 就是重复。
fn plain_bullet(entry: &Entry) -> String {
    format!("- {} (`{}`)\n", entry.description, short(&entry.sha))
}

/// 去掉 `type(scope)!: ` 前缀，只留描述。发布说明给用户看的是"改了什么"，
/// 不是"用了哪个 type"——type 已经被分到章节里去了。
fn strip_header(subject: &str) -> String {
    match subject.split_once(':') {
        Some((_, rest)) => rest.trim().to_string(),
        None => subject.trim().to_string(),
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// 仓库根 `CHANGELOG.md` 当前内容的指纹。文件不存在时给"空内容的指纹"——
/// 不存在与空文件对追加来说是同一件事，指纹必须一致，否则第一次追加就被自己拒掉。
pub fn target_digest(repo: &std::path::Path) -> String {
    let path = repo.join("CHANGELOG.md");
    match std::fs::read(&path) {
        Ok(bytes) => digest(&bytes),
        Err(_) => digest(&[]),
    }
}

pub fn digest(bytes: &[u8]) -> String {
    // FNV-1a：只需要"变了没有"，不需要密码学强度
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::spec::TypeGroup;

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

    /// 需求 6.5 的验收原文：`!`、`BREAKING CHANGE`、scope、revert、不合规各一条，
    /// 生成结果逐行匹配快照。
    #[test]
    fn the_fixture_renders_line_by_line_as_snapshotted() {
        let dir = repo_with(&[
            "feat(cli): 支持按地址只读浏览",
            "fix(parser)!: 换掉解析器入口",
            "feat(api): 换掉存储引擎\n\nBREAKING CHANGE: 旧配置不再兼容",
            "revert: 撤回上一条",
            "随手写的标题",
        ]);
        // tag 打在第一条上，所以区间覆盖后四条（含那条不合规的）
        git_in(dir.path(), &["tag", "v0.1.0", "HEAD~4"]);

        let changelog = build(
            dir.path(),
            &Spec::default(),
            &Range {
                from: Some("v0.1.0".into()),
                to: None,
            },
        )
        .expect("build");

        let snapshot = "\
# Changelog

## v0.1.0..HEAD

### ⚠ Breaking Changes

- **api**: 换掉存储引擎 (`xxxxxxx`)
- **parser**: 换掉解析器入口 (`xxxxxxx`)

### Features

#### api

- 换掉存储引擎 (`xxxxxxx`)

### Bug Fixes

#### parser

- 换掉解析器入口 (`xxxxxxx`)

### Reverts

- 撤回上一条 (`xxxxxxx`)

<!-- 另有 1 条不合规提交未收录（见符合率报告） -->
";
        let normalized: String = changelog
            .markdown
            .lines()
            .map(|line| {
                // sha 每次都不一样，快照里用 x 占位
                if line.contains("(`") && line.ends_with("`)") {
                    let head = line.split("(`").next().unwrap_or(line);
                    format!("{head}(`xxxxxxx`)")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(normalized.trim_end(), snapshot.trim_end());
        assert_eq!(changelog.skipped.non_conformant, 1, "不合规的要单独计数");
    }

    #[test]
    fn the_previous_tag_drives_the_default_range() {
        let dir = repo_with(&["feat: 一"]);
        assert_eq!(previous_tag(dir.path(), "HEAD"), None, "没有 tag 不是错误");

        git_in(dir.path(), &["tag", "v1.2.3"]);
        assert_eq!(
            previous_tag(dir.path(), "HEAD").as_deref(),
            Some("v1.2.3"),
            "默认区间就是上一个 tag..HEAD"
        );
    }

    #[test]
    fn a_type_outside_the_configured_groups_lands_in_other() {
        let spec = Spec {
            groups: vec![TypeGroup {
                ty: "feat".into(),
                section: "新功能".into(),
            }],
            ..Spec::default()
        };
        let dir = repo_with(&["feat: 一", "chore: 整理依赖"]);
        let changelog = build(dir.path(), &spec, &Range::default()).expect("build");
        let headings: Vec<&str> = changelog
            .sections
            .iter()
            .map(|section| section.heading.as_str())
            .collect();
        assert!(
            headings.contains(&"chore"),
            "没被配置的 type 要进\"其他\"而不是消失：{headings:?}"
        );
    }

    #[test]
    fn merges_are_counted_but_never_listed() {
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

        let changelog = build(dir.path(), &Spec::default(), &Range::default()).expect("build");
        assert_eq!(changelog.skipped.merges, 1);
        assert!(
            !changelog.markdown.contains("Merge branch"),
            "合并提交不该出现在发布说明里"
        );
        assert!(
            changelog.markdown.contains("已排除 1 条合并提交"),
            "排除了什么要写在文件里"
        );
    }

    #[test]
    fn an_empty_range_renders_a_statement_rather_than_an_empty_file() {
        let dir = repo_with(&["随手写的标题"]);
        let changelog = build(dir.path(), &Spec::default(), &Range::default()).expect("build");
        assert_eq!(changelog.included, 0);
        assert!(changelog.markdown.contains("没有可收录的规范提交"));
    }

    #[test]
    fn the_target_digest_changes_with_the_file() {
        let dir = repo_with(&["feat: 一"]);
        assert_eq!(
            target_digest(dir.path()),
            digest(&[]),
            "文件不存在时指纹等于空内容"
        );
        std::fs::write(dir.path().join("CHANGELOG.md"), "# Changelog\n").expect("write");
        let first = target_digest(dir.path());
        assert!(!first.is_empty());
        std::fs::write(dir.path().join("CHANGELOG.md"), "# Changelog\n\nmore\n").expect("write");
        assert_ne!(first, target_digest(dir.path()));
    }
}
