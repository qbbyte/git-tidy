use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// git 进度里的一条阶段信息。`--progress` 强制输出，否则非 tty 时 git 根本不打印。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub percent: u8,
}

/// 解析一行 stderr。只认 `Phase:  NN% (...)` 这一种形态，其他行（`Counting objects: 12, done.`
/// 这种没有百分号的）返回 None，由调用方忽略。
pub fn parse_progress(line: &str) -> Option<Progress> {
    let line = line.trim_start();
    let line = line.strip_prefix("remote:").unwrap_or(line).trim_start();
    let (phase, tail) = line.split_once(": ")?;
    let tail = tail.trim_start();
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || !tail[digits.len()..].starts_with('%') {
        return None;
    }
    Some(Progress {
        phase: phase.trim().to_string(),
        percent: digits.parse().ok()?,
    })
}

/// 仓库地址白名单。这一步不是格式洁癖：git 能把 `ext::sh -c ...`、`fd::`、`local::`
/// 这类形态的 URL 当命令执行，而参数是从界面传进来的。
pub fn validate_url(url: &str) -> Result<(), GitError> {
    if url.is_empty()
        || url.trim() != url
        || url.starts_with('-')
        || url.chars().any(char::is_whitespace)
    {
        return Err(GitError::BadRepoUrl);
    }

    let allowed_scheme = ["https://", "http://", "ssh://", "git://"]
        .iter()
        .any(|scheme| url.starts_with(scheme));
    if allowed_scheme {
        return Ok(());
    }

    // scp 式写法 `git@host:org/repo.git`。要求冒号左边是合法主机段、
    // 右边不以冒号/斜杠/连字符开头，这样 `ext::sh` 之类会被挡在外面。
    if url.contains("://") {
        return Err(GitError::BadRepoUrl);
    }
    let (host, path) = url.split_once(':').ok_or(GitError::BadRepoUrl)?;
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'));
    let path_ok = !path.is_empty()
        && !path.starts_with(':')
        && !path.starts_with('/')
        && !path.starts_with('-');
    if host_ok && path_ok {
        Ok(())
    } else {
        Err(GitError::BadRepoUrl)
    }
}

/// 只下 commit 对象、不建工作区的克隆。实测 git/git：82443 个 commit、37MB、36s，
/// 而同样的仓库完整克隆是 364MB；提交列表和改动文件都够用。
pub fn clone_browse(
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(Progress),
) -> Result<(), GitError> {
    let dest_str = dest
        .to_str()
        .ok_or_else(|| GitError::Internal("克隆目录路径含非法字符".into()))?;

    let out = process::run_streaming(
        None,
        &[
            "clone",
            "--filter=tree:0",
            "--no-checkout",
            "--progress",
            "--",
            url,
            dest_str,
        ],
        |line| {
            if let Some(progress) = parse_progress(line) {
                on_progress(progress);
            }
        },
    )?;

    if !out.success {
        return Err(GitError::GitFailed { stderr: out.stderr });
    }
    Ok(())
}

/// 把只读浏览仓库补成完整工作区仓库。四条命令的顺序不能换：
/// 实测先 `--refetch` 再解配置的话，`remote.origin.partialclonefilter` 和 `promisor`
/// 会原样留在库里，仓库仍是"缺对象、按需回源"的混合态。
///
/// `partialclonefilter` 留着是错的（后续 fetch 继续只下半边），所以它在补齐后必须读不到；
/// `promisor` 只是"允许回源"的能力开关，对象齐了它不起作用，因此不作断言，交给离线校验判定。
pub fn to_worktree(
    repo: &Path,
    branch: &str,
    mut on_progress: impl FnMut(Progress),
) -> Result<(), GitError> {
    clear_config(repo, "remote.origin.partialclonefilter")?;
    clear_config(repo, "remote.origin.promisor")?;

    let fetch = process::run_streaming(
        Some(repo),
        &["fetch", "--refetch", "--progress", "origin"],
        |line| {
            if let Some(progress) = parse_progress(line) {
                on_progress(progress);
            }
        },
    )?;
    if !fetch.success {
        return Err(GitError::GitFailed {
            stderr: fetch.stderr,
        });
    }

    let checkout = process::run(Some(repo), &["checkout", "-f", branch])?;
    if !checkout.success {
        return Err(GitError::GitFailed {
            stderr: checkout.stderr,
        });
    }

    verify_objects_local(repo)
}

/// 抽一个最早提交的文件清单读一遍，且禁掉按需回源。
/// filter 是 tree:0，缺的就是 tree——这条命令能离线跑通说明补齐真的做完了。
fn verify_objects_local(repo: &Path) -> Result<(), GitError> {
    let roots =
        process::run(Some(repo), &["rev-list", "--max-parents=0", "HEAD"])?.expect_success()?;
    // 命令成功却一行输出都没有 = 历史被截断（浅克隆到不了根），这同样不能算对象齐全
    let root = roots
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .ok_or_else(|| GitError::Internal("仓库里没有根提交，无法校验对象完整性".into()))?;

    let offline = process::run_with_env(
        Some(repo),
        &["ls-tree", "-r", "--name-only", root],
        &[("GIT_NO_LAZY_FETCH", "1")],
    )?;
    if !offline.success {
        return Err(GitError::GitFailed {
            stderr: offline.stderr,
        });
    }
    Ok(())
}

fn clear_config(repo: &Path, key: &str) -> Result<(), GitError> {
    // 键本来就不存在时 git config --unset 退出码 5，这不算失败，所以看结果不看退出码
    let _ = process::run(Some(repo), &["config", "--unset", key])?;
    let read = process::run(Some(repo), &["config", "--get", key])?;
    if read.success {
        return Err(GitError::GitFailed {
            stderr: format!("无法清除 git 配置 {key}"),
        });
    }
    Ok(())
}

/// 从地址取克隆目录名。取最后一段、去掉 `.git`，再把文件系统不接受的字符换成 `-`。
/// 这一步不能省：`repo::derive_name` 是按路径取的，而地址可能是 scp 式（冒号分隔）
/// 或带尾斜杠，直接拿来当目录名会在 Windows 上撞上 `<>:"\|?*`。
pub fn dest_name(url: &str) -> String {
    let tail = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".git");
    let sanitized: String = tail
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    // 全是点（`..`、`.`）或空：不是可用的目录名
    if sanitized.is_empty() || sanitized.chars().all(|c| c == '.') {
        "repo".to_string()
    } else {
        sanitized
    }
}

/// 读一个目录当前记录的远程地址，用于判断"这个目录是不是就是用户要的那个仓库"。
pub fn remote_url(repo: &Path) -> Option<String> {
    let out = process::run(Some(repo), &["config", "--get", "remote.origin.url"]).ok()?;
    if out.success {
        let url = out.stdout.trim().to_string();
        (!url.is_empty()).then_some(url)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn progress(line: &str) -> Option<(String, u8)> {
        parse_progress(line).map(|p| (p.phase, p.percent))
    }

    #[test]
    fn real_progress_lines_become_phase_and_percent() {
        assert_eq!(
            progress("Receiving objects:  42% (3/7), 12.0 KiB | 1.0 MiB/s"),
            Some(("Receiving objects".to_string(), 42))
        );
        assert_eq!(
            progress("remote: Compressing objects: 100% (5/5), done."),
            Some(("Compressing objects".to_string(), 100))
        );
        assert_eq!(
            progress("Resolving deltas: 7% (1/14)"),
            Some(("Resolving deltas".to_string(), 7))
        );
    }

    #[test]
    fn lines_without_a_percent_are_ignored_not_guessed() {
        for line in [
            "remote: Enumerating objects: 12, done.",
            "Cloning into 'browse'...",
            "warning: --filter is ignored in local clones",
            "",
        ] {
            assert_eq!(progress(line), None, "不该从 {line:?} 编出进度");
        }
    }

    #[test]
    fn executable_url_shapes_are_rejected() {
        // 这三个都是 git 真会当命令执行或回源到本地文件系统的形态
        for url in [
            "ext::sh -c whoami",
            "ext::sh",
            "fd::1:2",
            "local:/tmp/x",
            "file:///etc/passwd",
            "--upload-pack=touch /tmp/pwned",
            "https://example.com/a b",
            "https://example.com/x ",
        ] {
            assert!(
                matches!(validate_url(url), Err(GitError::BadRepoUrl)),
                "{url:?} 必须被拒绝"
            );
        }
    }

    #[test]
    fn ordinary_repo_urls_are_accepted() {
        for url in [
            "https://github.com/qbbyte/git-tidy.git",
            "http://git.internal/team/app",
            "git://github.com/git/git.git",
            "ssh://git@github.com/qbbyte/git-tidy.git",
            "git@github.com:qbbyte/git-tidy.git",
            "user@host:org/repo.git",
        ] {
            validate_url(url).unwrap_or_else(|err| panic!("{url:?} 应当可用：{err:?}"));
        }
    }

    #[test]
    fn an_empty_or_blank_url_is_rejected() {
        assert!(matches!(validate_url(""), Err(GitError::BadRepoUrl)));
        assert!(matches!(validate_url("  "), Err(GitError::BadRepoUrl)));
    }

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        out.stdout.trim().to_string()
    }

    #[test]
    fn clearing_a_config_key_verifies_the_result_not_the_exit_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "."]);
        git_in(
            dir.path(),
            &["config", "remote.origin.partialclonefilter", "tree:0"],
        );

        clear_config(dir.path(), "remote.origin.partialclonefilter").expect("clear");
        let read = process::run(
            Some(dir.path()),
            &["config", "--get", "remote.origin.partialclonefilter"],
        )
        .expect("read config");
        assert!(!read.success, "配置该读不到了，实际：{}", read.stdout);

        // 键本来就不存在：不能报错，否则没做过 partial clone 的仓库补不成人
        clear_config(dir.path(), "remote.origin.promisor").expect("clear missing key");
    }

    fn repo_with_one_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "."]);
        std::fs::write(dir.path().join("a.txt"), "one\n").expect("write");
        git_in(dir.path(), &["add", "a.txt"]);
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
                "feat: a",
            ],
        );
        dir
    }

    #[test]
    fn dest_name_takes_the_last_segment_without_the_git_suffix() {
        assert_eq!(
            dest_name("https://github.com/qbbyte/git-tidy.git"),
            "git-tidy"
        );
        assert_eq!(dest_name("https://github.com/qbbyte/git-tidy/"), "git-tidy");
        assert_eq!(dest_name("git@github.com:qbbyte/git-tidy.git"), "git-tidy");
        assert_eq!(dest_name("ssh://git@github.com/a/b"), "b");
    }

    /// Windows 目录名不能带 `<>:"\|?*`，而这些字符可能出现在地址的最后一段里。
    #[test]
    fn dest_name_is_safe_as_a_windows_directory_name() {
        assert_eq!(dest_name("https://host/team/re<po>1.git"), "re-po-1");
        assert_eq!(
            dest_name("https://host/a/.."),
            "repo",
            "相对路径写法不能逃出目录"
        );
        assert_eq!(dest_name("https://host/a/"), "a", "尾斜杠不能影响取名");
    }

    fn worktree_entries(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != ".git")
            .collect()
    }

    /// 跑通"贴地址 → 浏览 → 克隆补齐"整条链路，量的是真实服务端行为而不是本地 fixture：
    /// treeless 克隆能不能读出 HEAD、补齐之后工作区文件是否真的落地、promisor 配置是否清干净、
    /// 断网模式下最早提交的文件清单是否读得出。默认忽略：CI 不该依赖外网。
    ///
    /// `GIT_TIDY_LIVE_REPO=https://github.com/user/repo.git cargo test --lib -- --ignored --nocapture`
    ///
    /// 没有外网时用本机 git daemon 起一个真服务端，效果等价（`file://` 和路径克隆会忽略
    /// `--filter`，量不出服务端过滤，所以不能拿它代替）：
    ///
    /// ```text
    /// # 被 serve 的仓库要开这个开关，否则 upload-pack 不认 filter
    /// git -C <仓库> config uploadpack.allowFilter true
    /// git-daemon --reuseaddr --listen=127.0.0.1 --port=9418 \
    ///   --export-all --base-path=<仓库的父目录>
    /// GIT_TIDY_LIVE_REPO=git://127.0.0.1:9418/<仓库名> cargo test --lib -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "需要联网访问真实远程仓库"]
    fn live_treeless_clone_converts_to_a_normal_worktree() {
        use crate::git::repo;

        let Ok(url) = std::env::var("GIT_TIDY_LIVE_REPO") else {
            panic!("未设置 GIT_TIDY_LIVE_REPO，无法做联网量测");
        };
        validate_url(&url).expect("地址应通过白名单");

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("repo");
        let mut percents: Vec<u8> = Vec::new();
        clone_browse(&url, &dest, |p| percents.push(p.percent)).expect("treeless 克隆");

        let info = repo::probe(&dest).expect("browse 仓库要能探测");
        assert!(info.head_commit.is_some(), "commit 对象齐，HEAD 才读得到");
        assert!(
            worktree_entries(&dest).is_empty(),
            "--no-checkout 不该建工作区，实际落地：{:?}",
            worktree_entries(&dest)
        );
        let residue: Vec<String> = ["remote.origin.partialclonefilter", "remote.origin.promisor"]
            .iter()
            .filter_map(|key| {
                process::run(Some(&dest), &["config", "--get", key])
                    .ok()
                    .filter(|read| read.success)
                    .map(|read| format!("{key}={}", read.stdout.trim()))
            })
            .collect();
        println!("browse 阶段留下的 promisor 配置：{residue:?}");

        // 负对照：库里记着 partialclonefilter 就说明服务端真做了过滤、tree 对象是缺的，
        // 此时离线读取必须失败，否则后面的"补齐成功"没有信息量。
        // 服务端忽略 filter 时（本地路径克隆就是这样）对象本来就齐，这条不该判。
        if residue
            .iter()
            .any(|entry| entry.starts_with("remote.origin.partialclonefilter"))
        {
            let err = match verify_objects_local(&dest) {
                Ok(()) => panic!("browse 阶段居然离线读到了旧提交的树，filter 没生效？"),
                Err(err) => err,
            };
            println!("browse 阶段离线读取按预期失败：{err:?}");
        }

        let branch = info.branch.expect("browse 克隆的 HEAD 应指向默认分支");
        to_worktree(&dest, &branch, |p| percents.push(p.percent)).expect("补齐成完整仓库");
        assert!(
            !worktree_entries(&dest).is_empty(),
            "补齐之后工作区必须真的落地文件"
        );
        let filter = process::run(
            Some(&dest),
            &["config", "--get", "remote.origin.partialclonefilter"],
        )
        .expect("config");
        assert!(
            !filter.success,
            "补齐后 partialclonefilter 必须被清掉，否则后续 fetch 还是只下半边：{}",
            filter.stdout.trim()
        );
        // promisor 是"允许回源"的能力开关，对象齐了就不起作用，所以只记录不断言；
        // 真正的判定是下面那次禁回源的离线读取
        let promisor = process::run(Some(&dest), &["config", "--get", "remote.origin.promisor"])
            .expect("config");
        println!(
            "补齐后 promisor 开关：{}",
            if promisor.success {
                promisor.stdout.trim()
            } else {
                "(已清除)"
            }
        );
        verify_objects_local(&dest).expect("断网模式下要能读最早提交的文件清单");
        assert!(!percents.is_empty(), "真实克隆至少该给出若干进度百分比");
        println!(
            "补齐后工作区文件 {} 项，进度事件 {} 条",
            worktree_entries(&dest).len(),
            percents.len()
        );
    }

    /// 这条是防"校验写成空壳永远绿"的：删掉根提交的 tree 对象，离线读取必须失败。
    /// 需要的是缺 tree 的场景——filter tree:0 缺的正是 tree，两者同一条读取路径。
    #[test]
    fn offline_check_fails_when_a_tree_object_is_missing() {
        let dir = repo_with_one_commit();
        let tree = git_in(dir.path(), &["rev-parse", "HEAD^{tree}"]);
        let (prefix, rest) = tree.split_at(2);
        let object = dir
            .path()
            .join(".git")
            .join("objects")
            .join(prefix)
            .join(rest);
        assert!(object.exists(), "期望树对象在 {object:?}");
        std::fs::remove_file(&object).expect("删掉树对象");

        let err = match verify_objects_local(dir.path()) {
            Ok(()) => panic!("树对象已经删了，离线校验不该通过"),
            Err(err) => err,
        };
        assert!(
            format!("{err:?}").contains("tree"),
            "报错要能看出是读不到树对象，实际：{err:?}"
        );
    }

    /// 空仓库（没有任何提交）必须被判为校验失败，而不是"没有东西要校验"直接放行。
    /// 实测走的是 rev-list 失败这条路径：`ambiguous argument 'HEAD'`。
    #[test]
    fn offline_check_fails_on_an_empty_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "."]);

        let err = match verify_objects_local(dir.path()) {
            Ok(()) => panic!("空仓库没有根提交，校验必须失败"),
            Err(err) => err,
        };
        assert!(
            format!("{err:?}").contains("HEAD"),
            "要能看出是 HEAD 解析不了，实际：{err:?}"
        );
    }
}
