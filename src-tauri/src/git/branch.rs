use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// 一次删除分支的结果。`unmerged` 是"还没并进 `base` 的提交数"，
/// 界面上拿它决定要不要把 `-d` 升级成 `-D`（§7.10 的验收要求这个数与 git 一致）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deletable {
    /// 未合并的提交数。`merge_base` 为 None 时（两条线没有共同祖先）不去猜，报错误让用户自己看
    pub unmerged: Option<usize>,
    pub merged: bool,
}

/// 建分支。`start` 为空则从 HEAD 起。
pub fn create(repo: &Path, name: &str, start: Option<&str>) -> Result<(), GitError> {
    check_name(repo, name, "refs/heads/")?;
    let mut args = vec!["branch", name];
    if let Some(start) = start {
        args.push(start);
    }
    process::run(Some(repo), &args)?.expect_success()?;
    Ok(())
}

/// 改名。git 不支持改名成已存在的名字，这条约束由 git 自己管。
pub fn rename(repo: &Path, from: &str, to: &str) -> Result<(), GitError> {
    check_name(repo, from, "refs/heads/")?;
    check_name(repo, to, "refs/heads/")?;
    process::run(Some(repo), &["branch", "-m", from, to])?.expect_success()?;
    Ok(())
}

/// 删除。`force` 为真用 `-D`，否则用 `-d`（未合并时 git 会拒绝，这是我们要的默认）。
pub fn delete(repo: &Path, name: &str, force: bool) -> Result<(), GitError> {
    check_name(repo, name, "refs/heads/")?;
    let flag = if force { "-D" } else { "-d" };
    process::run(Some(repo), &["branch", flag, name])?.expect_success()?;
    Ok(())
}

/// 删之前先算未合并提交数：`-D` 是真删，界面上必须能先把"会丢多少"摆出来。
pub fn deletable(repo: &Path, name: &str, base: &str) -> Result<Deletable, GitError> {
    check_name(repo, name, "refs/heads/")?;
    let merge_base = process::run(Some(repo), &["merge-base", name, base])?;
    if !merge_base.success {
        return Ok(Deletable {
            unmerged: None,
            merged: false,
        });
    }
    let range = name.to_string();
    let exclude = format!("^{}", merge_base.stdout.trim());
    let stdout = process::run(Some(repo), &["rev-list", "--count", &range, &exclude])?
        .expect_success()?;
    let unmerged: usize = stdout.trim().parse().map_err(|_| GitError::ParseFailure {
        snippet: process::snippet(&stdout),
    })?;
    Ok(Deletable {
        unmerged: Some(unmerged),
        merged: unmerged == 0,
    })
}

/// 切换。已有分支走 `switch`，新分支走 `switch -c`。
///
/// 刻意用 `switch` 而不是 `checkout`：checkout 的参数表里 `-p`/`--patch` 那批选项
/// 在这个场景下不该出现，而 switch 的语义就是"只切分支"，不会有歧义。
pub fn switch(repo: &Path, name: &str, create: bool, start: Option<&str>) -> Result<(), GitError> {
    check_name(repo, name, "refs/heads/")?;
    let mut args = vec!["switch"];
    if create {
        args.push("-c");
    }
    args.push(name);
    if create {
        if let Some(start) = start {
            args.push(start);
        }
    }
    process::run(Some(repo), &args)?.expect_success()?;
    Ok(())
}

/// 设上游。`upstream` 为 None 表示清掉跟踪配置。
pub fn set_upstream(repo: &Path, branch: &str, upstream: Option<&str>) -> Result<(), GitError> {
    check_name(repo, branch, "refs/heads/")?;
    match upstream {
        Some(upstream) => {
            process::run(
                Some(repo),
                &["branch", "--set-upstream-to", upstream, branch],
            )?
            .expect_success()?;
        }
        None => {
            process::run(Some(repo), &["branch", "--unset-upstream", branch])?
                .expect_success()?;
        }
    }
    Ok(())
}

/// 打标签。`message` 有值就是附注标签（`-a -F`），没有就是轻量标签。
///
/// 附注标签的正文走 `-F 临时文件` 而不是 `-m`：`-m` 在 Windows 上对多行正文有引号坑，
/// 而标签正文恰恰是常常多行的那种东西（§6.15）。
pub fn tag(repo: &Path, name: &str, message: Option<&str>) -> Result<(), GitError> {
    check_name(repo, name, "refs/tags/")?;
    let mut args = vec!["tag".to_string()];
    if let Some(message) = message {
        // 正文直接当一个 argv 传：不经 shell（§5.5），多行与引号都不是问题，
        // 所以不需要临时文件——多行正文恰恰是标签说明的常态
        args.push("-a".to_string());
        args.push("-m".to_string());
        args.push(message.to_string());
    }
    args.push(name.to_string());
    process::run(Some(repo), &process::strs(&args))?.expect_success()?;
    Ok(())
}

pub fn delete_tag(repo: &Path, name: &str) -> Result<(), GitError> {
    check_name(repo, name, "refs/tags/")?;
    process::run(Some(repo), &["tag", "-d", name])?.expect_success()?;
    Ok(())
}

/// 名字合法性交给 git 自己判（`check-ref-format`）。我们不另写一套规则：
/// 分支名允许哪些字符是 git 说了算的，这里重写一遍只会在某条边角规则上不一致。
pub(crate) fn check_name(repo: &Path, name: &str, namespace: &str) -> Result<(), GitError> {
    let trimmed = name.trim();
    let shaped = !trimmed.is_empty()
        && trimmed.len() <= 255
        && !trimmed.starts_with('-')
        && !trimmed.chars().any(|c| c.is_control() || c.is_whitespace());
    if !shaped {
        return Err(GitError::GitFailed {
            stderr: format!("不是合法的名字：{name}"),
        });
    }
    let full = format!("{namespace}{trimmed}");
    let out = process::run(Some(repo), &["check-ref-format", &full])?;
    if out.success {
        return Ok(());
    }
    Err(GitError::GitFailed {
        stderr: format!("不是合法的名字：{name}（git：{}）", out.stderr.trim()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        out.stdout.trim_end().to_string()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        git_in(dir.path(), &["config", "user.name", "测试者"]);
        git_in(dir.path(), &["config", "user.email", "t@example.com"]);
        commit(dir.path(), "a.txt", "1\n", "feat: 基线");
        dir
    }

    fn commit(dir: &Path, file: &str, content: &str, message: &str) {
        fs::write(dir.join(file), content).expect("write");
        git_in(dir, &["add", "-A"]);
        git_in(dir, &["commit", "-q", "-m", message]);
    }

    fn branches(dir: &Path) -> Vec<String> {
        git_in(dir, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn create_switch_and_rename_a_branch() {
        let dir = repo();
        let path = dir.path();

        create(path, "feat/x", None).expect("建分支");
        assert!(branches(path).contains(&"feat/x".to_string()));

        switch(path, "feat/x", false, None).expect("切换");
        assert_eq!(
            git_in(path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "feat/x"
        );

        rename(path, "feat/x", "feat/y").expect("改名");
        assert!(branches(path).contains(&"feat/y".to_string()));
        assert!(!branches(path).contains(&"feat/x".to_string()));
    }

    #[test]
    fn creating_from_an_arbitrary_rev_starts_from_that_commit() {
        let dir = repo();
        let path = dir.path();
        let base = git_in(path, &["rev-parse", "HEAD"]);

        create(path, "old", Some(&base)).expect("建分支");
        switch(path, "old", false, None).expect("切换");
        assert_eq!(git_in(path, &["rev-parse", "HEAD"]), base);
    }

    #[test]
    fn switch_can_create_in_one_step() {
        let dir = repo();
        switch(dir.path(), "new-thing", true, None).expect("建并切");
        assert_eq!(
            git_in(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
            "new-thing"
        );
    }

    /// 删除未合并分支前给出的提交数必须与 git 自己算的一致（§7.10 的验收）
    #[test]
    fn the_unmerged_count_matches_git() {
        let dir = repo();
        let path = dir.path();
        git_in(path, &["checkout", "-q", "-b", "side"]);
        commit(path, "b.txt", "1\n", "feat: 支线一");
        commit(path, "c.txt", "2\n", "feat: 支线二");
        git_in(path, &["checkout", "-q", "main"]);

        let expected: usize = git_in(
            path,
            &[
                "rev-list",
                "--count",
                "side",
                &format!("^{}", git_in(path, &["merge-base", "side", "main"])),
            ],
        )
        .parse()
        .expect("count");

        let probe = deletable(path, "side", "main").expect("探测");
        assert_eq!(probe.unmerged, Some(expected));
        assert!(!probe.merged);
        assert!(expected > 0, "这个 fixture 里支线确实没并进 main");
    }

    #[test]
    fn a_merged_branch_reports_zero_unmerged() {
        let dir = repo();
        let path = dir.path();
        git_in(path, &["branch", "same"]);
        let probe = deletable(path, "same", "main").expect("探测");
        assert_eq!(probe.unmerged, Some(0));
        assert!(probe.merged);
    }

    #[test]
    fn delete_refuses_an_unmerged_branch_unless_forced() {
        let dir = repo();
        let path = dir.path();
        git_in(path, &["checkout", "-q", "-b", "side"]);
        commit(path, "b.txt", "1\n", "feat: 支线");
        git_in(path, &["checkout", "-q", "main"]);

        let err = delete(path, "side", false).expect_err("默认不该删掉未合并的分支");
        assert!(matches!(err, GitError::GitFailed { .. }), "{err:?}");

        delete(path, "side", true).expect("强制删");
        assert!(!branches(path).contains(&"side".to_string()));
    }

    #[test]
    fn upstream_can_be_set_and_cleared() {
        let dir = repo();
        let path = dir.path();
        // 本地远程：建一个裸库当 origin
        let bare = tempfile::tempdir().expect("bare");
        git_in(bare.path(), &["init", "-q", "--bare", "."]);
        let origin = bare.path().to_string_lossy().to_string();
        git_in(path, &["remote", "add", "origin", &origin]);
        git_in(path, &["push", "-q", "-u", "origin", "main"]);

        set_upstream(path, "main", Some("origin/main")).expect("设上游");
        let upstream = git_in(path, &["rev-parse", "--abbrev-ref", "main@{upstream}"]);
        assert_eq!(upstream, "origin/main");

        set_upstream(path, "main", None).expect("清上游");
        assert!(
            process::run(Some(path), &["rev-parse", "--abbrev-ref", "main@{upstream}"])
                .map(|out| !out.success)
                .unwrap_or(false),
            "清掉之后不该再有上游"
        );
    }

    #[test]
    fn tags_are_lightweight_by_default_and_annotated_with_a_message() {
        let dir = repo();
        let path = dir.path();

        tag(path, "lw", None).expect("轻量标签");
        tag(path, "v1", Some("发布说明\n第二行\n")).expect("附注标签");

        let kinds = git_in(
            path,
            &["for-each-ref", "--format=%(refname:short) %(objecttype)", "refs/tags"],
        );
        assert!(kinds.contains("lw commit"), "轻量标签直接指提交：{kinds}");
        assert!(kinds.contains("v1 tag"), "附注标签指标签对象：{kinds}");

        // 多行正文原样落盘
        let body = git_in(path, &["tag", "-l", "v1", "--format=%(contents)"]);
        assert!(body.contains("发布说明"), "{body}");
        assert!(body.contains("第二行"), "多行正文不能只剩第一行：{body}");

        delete_tag(path, "lw").expect("删标签");
        assert!(!git_in(path, &["tag", "-l"]).contains("lw"));
    }

    #[test]
    fn illegal_names_are_refused_before_reaching_git() {
        let dir = repo();
        let path = dir.path();

        for bad in ["", "  ", "-x", "with space", "bad~name", "a..b", "带 控制\t符"] {
            assert!(
                create(path, bad, None).is_err(),
                "分支名该被拒：{bad:?}"
            );
        }
        // 与现有分支同名也要拒：那是 git 的约束，我们转述它而不是自己发明规则
        assert!(create(path, "main", None).is_err(), "同名分支不该被建出来");
        assert!(delete_tag(path, "--delete").is_err());
    }

    
}