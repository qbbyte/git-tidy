use std::path::Path;

use serde::Deserialize;

use super::log::{self, Commit};
use super::refs::{self, Interrupt};
use super::{process, status};
use crate::config::check::{self, evaluate, Outcome};
use crate::config::spec::Spec;
use crate::error::GitError;

/// 界面上那张表单的内容。三个字段分开传，是为了让"预览"和"落库的那条信息"由同一段代码算出来
/// （需求 6.2 的实时预览必须和最终提交一致，否则预览就是骗人的）。
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub subject: String,
    pub body: String,
    pub footer: String,
}

impl Draft {
    /// 校验时正文包含 footer：任务 ID 常写在 `Refs: TIDY-12` 这种脚注里。
    pub fn searchable_body(&self) -> String {
        [self.body.trim(), self.footer.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// 最终信息：主题 + 空行 + 正文 + 空行 + 脚注，末尾带一个换行。
    pub fn compose(&self) -> String {
        let parts = [
            self.subject.trim(),
            self.body.trim(),
            self.footer.trim(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        format!("{parts}\n")
    }
}

/// 用表单内容提交。规范判定在这里兜底（§7.5）：前端的禁用挡不住绕过界面的调用，
/// 而这条路径是唯一能写历史的入口之一，必须在 Rust 侧再判一次。
pub fn create(repo: &Path, spec: &Spec, draft: &Draft) -> Result<Commit, GitError> {
    // §7.3：中断态里 git commit 的语义是"给这次合并/摘取收尾"，产物是一个带两个父的
    // 合并提交，和用户在表单上点的东西不是一回事。界面已经把提交按钮禁掉，
    // 这里再挡一次——绕开界面直接调用也进不来。
    let info = refs::interrupt(repo)?;
    if info.kind != Interrupt::None {
        return Err(GitError::OperationInProgress { state: info.kind });
    }

    let outcome = check::evaluate(spec, &draft.subject, &draft.searchable_body());
    if !outcome.conformant {
        return Err(GitError::NotConformant {
            violations: outcome
                .violations
                .into_iter()
                .filter(|violation| violation.blocking)
                .collect(),
        });
    }

    let staged = status::list(repo)?
        .into_iter()
        .filter(|file| file.staged)
        .count();
    if staged == 0 {
        return Err(GitError::NothingStaged);
    }

    let message_file = temp_message(draft)?;
    let committed = process::run(Some(repo), &["commit", "-F", &message_file]);
    let _ = std::fs::remove_file(&message_file);
    committed?.expect_success()?;

    // 读回来而不是把我写进去的那份还回去：commit-msg 钩子有权改写提交信息
    let page = log::list(repo, 0, 1)?;
    page.commits
        .into_iter()
        .next()
        .ok_or_else(|| GitError::Internal("提交后读不回 HEAD".into()))
}

/// 校验但不提交，给表单实时提示用。
pub fn preview(spec: &Spec, draft: &Draft) -> Outcome {
    evaluate(spec, &draft.subject, &draft.searchable_body())
}

/// 提交信息写成临时文件，用 `commit -F` 读：走 `-m` 拼接会把正文和脚注的换行弄丢。
fn temp_message(draft: &Draft) -> Result<String, GitError> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos())
        .unwrap_or_default();
    let path = std::env::temp_dir().join(format!("git-tidy-msg-{}-{unique}.txt", std::process::id()));
    std::fs::write(&path, draft.compose())
        .map_err(|err| GitError::Internal(format!("写提交信息临时文件失败：{err}")))?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        out.stdout.trim().to_string()
    }

    fn repo_with_staged_change() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main", "."]);
        std::fs::write(dir.path().join("a.txt"), "内容\n").expect("write");
        git(dir.path(), &["add", "a.txt"]);
        // 只配身份，不动用户的 git 配置
        git(dir.path(), &["config", "user.name", "张三"]);
        git(dir.path(), &["config", "user.email", "z@example.com"]);
        dir
    }

    fn draft(subject: &str) -> Draft {
        Draft {
            subject: subject.into(),
            body: String::new(),
            footer: String::new(),
        }
    }

    #[test]
    fn compose_joins_the_three_fields_with_blank_lines() {
        let draft = Draft {
            subject: "feat: 支持中文标题".into(),
            body: "第一行正文\n第二行正文".into(),
            footer: "Refs: TIDY-12".into(),
        };
        assert_eq!(
            draft.compose(),
            "feat: 支持中文标题\n\n第一行正文\n第二行正文\n\nRefs: TIDY-12\n"
        );
    }

    #[test]
    fn compose_skips_empty_fields_instead_of_leaving_blank_lines() {
        let draft = Draft {
            subject: "fix: 只有一行".into(),
            body: "   ".into(),
            footer: "".into(),
        };
        assert_eq!(draft.compose(), "fix: 只有一行\n");
    }

    #[test]
    fn a_conformant_draft_lands_as_the_head_commit() {
        let dir = repo_with_staged_change();
        let commit = create(
            dir.path(),
            &Spec::default(),
            &draft("feat: 添加按地址浏览仓库"),
        )
        .expect("提交应成功");
        assert_eq!(commit.subject, "feat: 添加按地址浏览仓库");
        assert_eq!(commit.author_name, "张三");
        assert_eq!(git(dir.path(), &["rev-list", "--count", "HEAD"]), "1");
    }

    /// 需求 7.5：拦截不能只写在前端。这里直接调 create，绕过一切界面，仍必须被拒。
    #[test]
    fn a_non_conformant_draft_is_refused_and_creates_no_commit() {
        let dir = repo_with_staged_change();
        let err = match create(dir.path(), &Spec::default(), &draft("wip")) {
            Ok(commit) => panic!("无信息量的标题不该提交成功：{}", commit.id),
            Err(err) => err,
        };
        let rendered = format!("{err:?}");
        assert!(rendered.contains("message_not_conformant"), "{rendered}");
        let head = process::run(Some(dir.path()), &["rev-parse", "--verify", "-q", "HEAD"])
            .expect("spawn git");
        assert!(
            !head.success,
            "被拒之后仓库里不该多出一条提交，实际 HEAD={}",
            head.stdout.trim()
        );
    }

    #[test]
    fn the_rejection_carries_every_blocking_reason_with_a_hint() {
        let dir = repo_with_staged_change();
        let spec = Spec {
            scope_required: true,
            ..Spec::default()
        };
        let err = match create(dir.path(), &spec, &draft("feat: 缺少 scope")) {
            Ok(_) => panic!("缺 scope 的提交不该成功"),
            Err(err) => err,
        };
        let GitError::NotConformant { violations } = err else {
            panic!("要报规范不符，实际：{err:?}");
        };
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].reason, check::Reason::MissingScope);
        assert!(violations[0].hint.contains("scope"));
    }

    #[test]
    fn a_task_id_in_the_footer_satisfies_the_requirement() {
        let spec = Spec {
            task_id_pattern: Some(r"[A-Z]+-\d+".to_string()),
            ..Spec::default()
        };
        let draft = Draft {
            subject: "feat: 带单号的提交".into(),
            body: String::new(),
            footer: "Refs: TIDY-7".into(),
        };
        assert!(preview(&spec, &draft).conformant, "{:?}", preview(&spec, &draft).violations);
    }

    /// §7.3：中断态下的 `git commit` 语义是"给这次合并收尾"，产物是一个两个父的合并提交，
    /// 和用户在表单上点的东西不是一回事。界面禁用挡不住直接调用，这里必须自己挡住。
    #[test]
    fn an_interrupted_merge_blocks_the_commit_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main", "."]);
        git(dir.path(), &["config", "user.name", "张三"]);
        git(dir.path(), &["config", "user.email", "z@example.com"]);

        std::fs::write(dir.path().join("a.txt"), "base\n").expect("write");
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "feat: 基线"]);
        git(dir.path(), &["checkout", "-q", "-b", "side"]);
        std::fs::write(dir.path().join("a.txt"), "side\n").expect("write");
        git(dir.path(), &["commit", "-q", "-am", "feat: 支线改法"]);
        git(dir.path(), &["checkout", "-q", "main"]);
        std::fs::write(dir.path().join("a.txt"), "main\n").expect("write");
        git(dir.path(), &["commit", "-q", "-am", "feat: 主干改法"]);

        let before = git(dir.path(), &["rev-parse", "HEAD"]);
        let merged = process::run(Some(dir.path()), &["merge", "side"]).expect("spawn merge");
        assert!(!merged.success, "这次合并必须冲突：{}", merged.stdout);

        let err = match create(dir.path(), &Spec::default(), &draft("feat: 中断里还想提交")) {
            Ok(commit) => panic!("合并中断时不该提交成功：{}", commit.id),
            Err(err) => err,
        };
        let GitError::OperationInProgress { state } = &err else {
            panic!("合并中断要报专门的错误码，实际：{err:?}");
        };
        assert_eq!(*state, Interrupt::Merge, "错误里要带上是哪一种中断态");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("operation_in_progress") && rendered.contains("合并进行中"),
            "文案要能看出是哪一种中断，实际：{rendered}"
        );
        assert_eq!(
            git(dir.path(), &["rev-parse", "HEAD"]),
            before,
            "被拒之后 HEAD 必须还在冲突前的那条提交上"
        );
    }

    #[test]
    fn nothing_staged_is_its_own_clear_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main", "."]);
        std::fs::write(dir.path().join("a.txt"), "没 add\n").expect("write");
        let err = create(dir.path(), &Spec::default(), &draft("feat: 还没暂存"));
        match err {
            Err(GitError::NothingStaged) => {}
            Err(other) => panic!("未暂存要报专门的错误，实际：{other:?}"),
            Ok(commit) => panic!("未暂存居然提交成功了：{}", commit.id),
        }
    }

    #[test]
    fn multiline_body_with_chinese_survives_the_round_trip() {
        let dir = repo_with_staged_change();
        let draft = Draft {
            subject: "feat: 支持中文正文".into(),
            body: "第一段：说明动机。\n\n第二段：列出影响面。".into(),
            footer: "Refs: TIDY-1".into(),
        };
        create(dir.path(), &Spec::default(), &draft).expect("提交");
        let raw = git(dir.path(), &["log", "-1", "--format=%B"]);
        assert_eq!(raw, "feat: 支持中文正文\n\n第一段：说明动机。\n\n第二段：列出影响面。\n\nRefs: TIDY-1");
    }
}
