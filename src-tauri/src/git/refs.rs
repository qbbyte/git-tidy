use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;

use super::process;
use crate::error::GitError;

const FIELD_SEP: char = '\u{1f}';

/// 引用的类别。
///
/// 归类只看完整 refname 的前缀，不看短名：短形式下本地分支 `feat/x` 和远程 `origin`
/// 里的分支 `x` 都显示成 `feat/x`，实测同名时根本分不开（本机 git 2.54）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RefKind {
    /// refs/heads/
    Branch,
    /// refs/remotes/
    Remote,
    /// refs/tags/
    Tag,
}

const PREFIXES: &[(&str, RefKind)] = &[
    ("refs/heads/", RefKind::Branch),
    ("refs/remotes/", RefKind::Remote),
    ("refs/tags/", RefKind::Tag),
];

/// 完整 refname → (类别, 去掉命名空间的名字)。
///
/// 认不出前缀返回 None：装饰里除了这三个命名空间还有 `grafted:` 这类项，
/// 那不是一个引用名字。
pub fn classify(full: &str) -> Option<(RefKind, &str)> {
    PREFIXES
        .iter()
        .find(|(prefix, _)| full.starts_with(prefix))
        .map(|(prefix, kind)| (*kind, &full[prefix.len()..]))
}

/// 去掉命名空间的展示名；认不出的原样返回（跟踪分支偶尔会指向 refs/heads/ 这种非远程名）
fn short_name(full: &str) -> String {
    classify(full)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| full.to_string())
}

/// 注册表里的一个引用：本地分支、远程跟踪分支或标签。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ref {
    /// `refs/heads/main` → `main`；`refs/remotes/origin/main` → `origin/main`
    pub name: String,
    pub kind: RefKind,
    /// 原始完整名字：写操作和跨命令比对都要用它，短名是会被两个命名空间共用的
    pub full_name: String,
    /// 引用落到的提交。附注标签存的是标签对象，必须取剥壳后的 `%(*objectname)`，
    /// 不然这个 sha 在提交列表里一条都对不上（实测标签对象的 sha 与提交 sha 不同）
    pub target: String,
    /// 跟踪的远程分支短名（`origin/main`）；没配跟踪分支是 None
    pub upstream: Option<String>,
    /// 与跟踪分支相比多出的提交数。两个都是 None 表示"没有这个数字"：
    /// 要么没配跟踪分支，要么已同步（`%(upstream:track)` 在同步时输出空串，实测），
    /// 要么 git 换了格式。有数时两个一起给，git 只写非零的那半、另一半是确定的 0。
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    /// 跟踪的远程分支已在远程被删掉（`%(upstream:track)` 给出 `[gone]`，实测）
    pub upstream_gone: bool,
}

/// for-each-ref 的原子表。这里的分隔符写法是 `%1f`，log 那边是 `%x1f`——
/// 两条命令各认一套转义（实测：都能出 0x1f）。
const REF_ATOMS: &[&str] = &[
    "%(refname)",
    "%(objectname)",
    "%(*objectname)",
    "%(upstream)",
    "%(upstream:track)",
];
const ATOM_SEP: &str = "%1f";

/// 一次读回全部本地分支、远程跟踪分支和标签。
///
/// for-each-ref 没有 `-z`（实测报 `unknown switch 'z'`），记录只能按换行切。
/// 这里敢按换行切是因为选中的原子一个都不含换行：refname 连空格都不允许。
pub fn list(repo: &Path) -> Result<Vec<Ref>, GitError> {
    let format = format!("--format={}", REF_ATOMS.join(ATOM_SEP));
    let stdout = process::run(
        Some(repo),
        &["for-each-ref", &format, "refs/heads", "refs/remotes", "refs/tags"],
    )?
    .expect_success()?;

    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(parse_ref)
        .collect()
}

fn parse_ref(line: &str) -> Result<Ref, GitError> {
    let failure = || GitError::ParseFailure {
        snippet: process::snippet(line),
    };
    let parts: Vec<&str> = line.splitn(REF_ATOMS.len(), FIELD_SEP).collect();
    if parts.len() != REF_ATOMS.len() {
        return Err(failure());
    }

    // 只问这三个命名空间，落不进任何一个说明 git 改了行为，不是解析层该兜的
    let (kind, name) = classify(parts[0]).ok_or_else(failure)?;
    let (counts, gone) = parse_track(parts[4]);
    let (ahead, behind) = match counts {
        Some((ahead, behind)) => (Some(ahead), Some(behind)),
        None => (None, None),
    };

    Ok(Ref {
        name: name.to_string(),
        kind,
        full_name: parts[0].to_string(),
        target: target_of(parts[1], parts[2]),
        upstream: (!parts[3].is_empty()).then(|| short_name(parts[3])),
        ahead,
        behind,
        upstream_gone: gone,
    })
}

/// 轻量标签和非标签引用没有剥壳值，%(objectname) 就是提交。
fn target_of(object: &str, peeled: &str) -> String {
    if peeled.is_empty() {
        object.to_string()
    } else {
        peeled.to_string()
    }
}

/// `%(upstream:track)` 的全部形态（实测）：`[ahead 1]`、`[behind 2]`、
/// `[ahead 1, behind 1]`、`[gone]`，以及已同步时的空串。
///
/// 没配跟踪分支也是空串，两者靠 `%(upstream)` 本身是不是空来分开。
fn parse_track(raw: &str) -> (Option<(usize, usize)>, bool) {
    let Some(inner) = raw
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return (None, false);
    };

    let mut ahead = 0;
    let mut behind = 0;
    let mut gone = false;
    let mut known = false;
    for item in inner.split(", ") {
        let mut words = item.split_whitespace();
        match (words.next(), words.next()) {
            (Some("ahead"), Some(n)) => {
                if let Ok(value) = n.parse() {
                    ahead = value;
                    known = true;
                }
            }
            (Some("behind"), Some(n)) => {
                if let Ok(value) = n.parse() {
                    behind = value;
                    known = true;
                }
            }
            (Some("gone"), _) => gone = true,
            _ => {}
        }
    }

    (known.then_some((ahead, behind)), gone)
}

/// 仓库停在哪个中断态上。它决定界面上哪些写入口可用（§7.3）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Interrupt {
    /// 没有任何中断操作在进行
    None,
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Interrupt {
    /// 给用户看的说法。错误文案和提示条共用这一份，免得两边叫法对不上。
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "没有中断操作",
            Self::Merge => "合并进行中",
            Self::Rebase => "变基进行中",
            Self::CherryPick => "摘取进行中",
            Self::Revert => "回滚进行中",
        }
    }
}

/// 判定中断态要看的标记，数组顺序就是优先级。每一项是 (状态, 标记名)。
///
/// 变基排在最前：交互式变基的每一步都是 sequencer 在摘提交，中断时 rebase-merge 和
/// CHERRY_PICK_HEAD 可能同时存在，报成 cherry-pick 会让人去找一个根本不存在的
/// cherry-pick 出口。
const MARKERS: &[(Interrupt, &str)] = &[
    (Interrupt::Rebase, "rebase-merge"),
    (Interrupt::Rebase, "rebase-apply"),
    (Interrupt::CherryPick, "CHERRY_PICK_HEAD"),
    (Interrupt::Revert, "REVERT_HEAD"),
    (Interrupt::Merge, "MERGE_HEAD"),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptInfo {
    pub kind: Interrupt,
    /// 变基时那个正在被变基的分支短名。
    ///
    /// 变基过程中 HEAD 是游离的（实测 status 只会说 `## HEAD (no branch)`），
    /// 不补这一条界面就只剩"游离 HEAD"，看不出是哪个分支在被变基。
    /// 其他中断态 HEAD 就落在原分支上，这里是 None。
    pub branch: Option<String>,
}

/// 读中断态。五个标记一次问完：`rev-parse` 重复给 `--git-path` 就按参数顺序
/// 一行一个输出（实测）。
///
/// 必须走 `--git-path` 而不是拼 `.git/`：linked worktree 的 MERGE_HEAD 落在
/// `.git/worktrees/<名>/` 下面，拼主仓库的 git-dir 会漏报（实测）。
///
/// 这里**不**用 `git status`：porcelain v1 不带 `--branch` 时压根没有头部行（实测），
/// 而带了 `--branch` 也不多出标记文件之外的信息；status 要扫整个工作区，大仓库上贵，
/// treeless 仓库上还会造出成片的假"脏"。
pub fn interrupt(repo: &Path) -> Result<InterruptInfo, GitError> {
    let mut args: Vec<&str> = vec!["rev-parse"];
    for (_, name) in MARKERS {
        args.push("--git-path");
        args.push(name);
    }

    let stdout = process::run(Some(repo), &args)?.expect_success()?;
    let paths: Vec<&str> = stdout.lines().collect();
    if paths.len() != MARKERS.len() {
        return Err(GitError::ParseFailure {
            snippet: process::snippet(&stdout),
        });
    }

    // MARKERS 的顺序即优先级：第一个存在的标记就是答案，后面的不再看
    let hit = MARKERS.iter().zip(&paths).find_map(|((kind, _), path)| {
        let full = resolve(repo, path);
        is_present(&full).then_some((*kind, full))
    });

    let Some((kind, dir)) = hit else {
        return Ok(InterruptInfo {
            kind: Interrupt::None,
            branch: None,
        });
    };

    // head-name 只有变基的两个目录里有；读不到就留 None，中断态本身已由目录判出
    let branch = (kind == Interrupt::Rebase)
        .then(|| read_head_name(&dir))
        .flatten();

    Ok(InterruptInfo { kind, branch })
}

/// `--git-path` 的输出形态不固定：普通仓库给相对的 `.git/MERGE_HEAD`，
/// linked worktree 给绝对的 `…/.git/worktrees/<名>/MERGE_HEAD`（实测）。
/// 相对路径是相对仓库目录的，而这个进程的当前目录不是仓库目录，必须显式接上去。
fn resolve(repo: &Path, git_path: &str) -> PathBuf {
    let path = Path::new(git_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.join(path)
    }
}

/// 目录型标记（rebase-*）要非空才算在：sequencer 收尾时先清空目录再删，
/// 中途被杀可能留下一个空目录，那时没有 head-name，也没有能续跑的东西。
fn is_present(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_dir() {
        return true;
    }
    path.read_dir()
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn read_head_name(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("head-name")).ok()?;
    let full = raw.trim();
    (!full.is_empty()).then(|| short_name(full))
}

/// §7.1 的仓库级状态摘要，给侧栏和最显眼处的提示条用。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoState {
    /// 当前分支短名；None = 游离 HEAD。空仓库里 HEAD 指向还没诞生的分支，这里照样有名字
    pub branch: Option<String>,
    /// 当前分支跟踪的远程分支短名
    pub upstream: Option<String>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub upstream_gone: bool,
    pub interrupt: Interrupt,
    /// 见 InterruptInfo.branch：只在变基时有值
    pub interrupt_branch: Option<String>,
}

/// 状态摘要。ahead/behind 直接从已经扫过的引用里取当前分支那一行，
/// 不为它单独跑一次 `rev-list --count A..B`——分支一多就这样把界面卡死（§7.3）。
pub fn state(repo: &Path, refs: &[Ref]) -> Result<RepoState, GitError> {
    let branch = current_branch(repo)?;
    let info = interrupt(repo)?;
    let row = branch
        .as_deref()
        .and_then(|name| refs.iter().find(|row| row.kind == RefKind::Branch && row.name == name));

    Ok(RepoState {
        branch,
        upstream: row.and_then(|row| row.upstream.clone()),
        ahead: row.and_then(|row| row.ahead),
        behind: row.and_then(|row| row.behind),
        upstream_gone: row.map(|row| row.upstream_gone).unwrap_or(false),
        interrupt: info.kind,
        interrupt_branch: info.branch,
    })
}

/// 当前分支。游离 HEAD 下这条命令按预期非零退出，那是需要区分的正常状态，不是错误。
fn current_branch(repo: &Path) -> Result<Option<String>, GitError> {
    let out = process::run(Some(repo), &["symbolic-ref", "--short", "-q", "HEAD"])?;
    Ok(out
        .success
        .then(|| out.stdout.trim().to_string())
        .filter(|name| !name.is_empty()))
}

/// 提交行上的引用徽标（§7.3）。名字已去掉命名空间，样式按 kind 分。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Badge {
    pub name: String,
    pub kind: RefKind,
    /// HEAD 正指向这个引用。游离 HEAD 时没有这样的引用，但那条提交的 head 仍是 true
    pub head: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        out.stdout.trim().to_string()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        git_in(dir.path(), &["config", "user.name", "测试者"]);
        git_in(dir.path(), &["config", "user.email", "t@example.com"]);
        dir
    }

    fn commit(dir: &Path, file: &str, msg: &str) -> String {
        fs::write(dir.join(file), format!("{file}\n")).expect("write");
        git_in(dir, &["add", file]);
        git_in(dir, &["commit", "-q", "-m", msg]);
        git_in(dir, &["rev-parse", "HEAD"])
    }

    /// 真远程：`git branch --set-upstream-to` 拒绝指向用 update-ref 造出来的假远程跟踪引用
    /// （实测报 "not a branch"），要 ahead/behind 只能走一次真 push。
    fn with_remote(dir: &Path) -> tempfile::TempDir {
        let remote = tempfile::tempdir().expect("tempdir remote");
        git_in(
            remote.path(),
            &["init", "-q", "--bare", "--initial-branch=main", "."],
        );
        git_in(
            dir,
            &["remote", "add", "origin", &remote.path().to_string_lossy()],
        );
        git_in(dir, &["push", "-q", "-u", "origin", "main"]);
        remote
    }

    fn find<'a>(refs: &'a [Ref], name: &str) -> &'a Ref {
        refs.iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("列表里没有 {name}，实际：{:?}", names(refs)))
    }

    fn names(refs: &[Ref]) -> Vec<String> {
        refs.iter().map(|row| row.name.clone()).collect()
    }

    #[test]
    fn classify_splits_local_from_remote_of_the_same_short_name() {
        // 实测的同名歧义：refs/heads/feat/x 与 refs/remotes/feat/x 短名都是 feat/x
        assert_eq!(
            classify("refs/heads/feat/x"),
            Some((RefKind::Branch, "feat/x"))
        );
        assert_eq!(
            classify("refs/remotes/feat/x"),
            Some((RefKind::Remote, "feat/x"))
        );
        assert_eq!(classify("refs/tags/v1"), Some((RefKind::Tag, "v1")));
        assert_eq!(classify("grafted: 12ab34"), None);
    }

    #[test]
    fn branches_remotes_and_tags_are_all_listed_with_a_commit_target() {
        let dir = repo();
        let head = commit(dir.path(), "a.txt", "feat: 基线");
        git_in(dir.path(), &["branch", "side"]);
        git_in(dir.path(), &["update-ref", "refs/remotes/origin/main", &head]);
        git_in(dir.path(), &["tag", "lw-tag"]);
        git_in(dir.path(), &["tag", "-a", "ann-tag", "-m", "发布说明"]);

        let refs = list(dir.path()).expect("读引用");
        assert_eq!(find(&refs, "main").kind, RefKind::Branch);
        assert_eq!(find(&refs, "side").kind, RefKind::Branch);
        assert_eq!(find(&refs, "origin/main").kind, RefKind::Remote);
        assert_eq!(find(&refs, "lw-tag").kind, RefKind::Tag);
        assert_eq!(find(&refs, "ann-tag").kind, RefKind::Tag);

        // 附注标签的 objectname 是标签对象，不剥壳就一条都对不上（实测两者不同）
        assert_eq!(find(&refs, "ann-tag").target, head);
        assert_eq!(find(&refs, "lw-tag").target, head);
        assert_eq!(find(&refs, "main").full_name, "refs/heads/main");
    }

    #[test]
    fn ahead_and_behind_match_git_rev_list_count() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        let _remote = with_remote(dir.path());

        commit(dir.path(), "b.txt", "feat: 本地一");
        commit(dir.path(), "c.txt", "feat: 本地二");

        let refs = list(dir.path()).expect("读引用");
        let main = find(&refs, "main");
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert_eq!((main.ahead, main.behind), (Some(2), Some(0)));

        // 与 git 自己的计数逐值对照，这是 §7.3 的验收项
        let counts = git_in(
            dir.path(),
            &["rev-list", "--left-right", "--count", "main...origin/main"],
        );
        assert_eq!(counts, "2\t0", "git 侧的实测值");
    }

    #[test]
    fn synced_ahead_only_and_gone_are_told_apart() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        let remote = with_remote(dir.path());

        // 刚推完：已同步，没有数字，但跟踪分支是存在的
        let refs = list(dir.path()).expect("读引用");
        let main = find(&refs, "main");
        assert_eq!((main.ahead, main.behind, main.upstream_gone), (None, None, false));
        assert!(main.upstream.is_some(), "已同步也要报出跟踪分支名");

        commit(dir.path(), "b.txt", "feat: 只有本地多");
        let refs = list(dir.path()).expect("读引用");
        assert_eq!(
            (find(&refs, "main").ahead, find(&refs, "main").behind),
            (Some(1), Some(0)),
            "git 只写非零的那半，另一半是确定的 0，不能报成未知"
        );

        // 远程分支被删：跟踪分支还在配置里，但对象没了
        git_in(
            remote.path(),
            &["update-ref", "-d", "refs/heads/main"],
        );
        git_in(dir.path(), &["fetch", "-q", "--prune", "origin"]);
        let refs = list(dir.path()).expect("读引用");
        let main = find(&refs, "main");
        assert!(main.upstream_gone, "[gone] 要单独认出来");
        assert_eq!((main.ahead, main.behind), (None, None));
    }

    #[test]
    fn a_branch_without_upstream_has_no_track_at_all() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        git_in(dir.path(), &["branch", "no-up"]);

        let refs = list(dir.path()).expect("读引用");
        let row = find(&refs, "no-up");
        assert!(row.upstream.is_none());
        assert_eq!((row.ahead, row.behind, row.upstream_gone), (None, None, false));
    }

    #[test]
    fn parse_track_covers_every_form_git_emits() {
        // 四种形态全部是实测输出，空串那两种由 upstream 字段本身分开
        assert_eq!(parse_track("[ahead 3]"), (Some((3, 0)), false));
        assert_eq!(parse_track("[behind 2]"), (Some((0, 2)), false));
        assert_eq!(parse_track("[ahead 1, behind 4]"), (Some((1, 4)), false));
        assert_eq!(parse_track("[gone]"), (None, true));
        assert_eq!(parse_track(""), (None, false));
        assert_eq!(parse_track("[]"), (None, false));
        assert_eq!(parse_track("[未来的写法]"), (None, false));
    }

    #[test]
    fn the_state_summary_carries_the_current_branch_track() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        let _remote = with_remote(dir.path());
        commit(dir.path(), "b.txt", "feat: 本地一");

        let refs = list(dir.path()).expect("读引用");
        let state = state(dir.path(), &refs).expect("状态摘要");
        assert_eq!(state.branch.as_deref(), Some("main"));
        assert_eq!(state.upstream.as_deref(), Some("origin/main"));
        assert_eq!((state.ahead, state.behind), (Some(1), Some(0)));
        assert_eq!(state.interrupt, Interrupt::None);
        assert!(state.interrupt_branch.is_none());
    }

    #[test]
    fn detached_head_has_no_branch_but_still_reads_its_markers() {
        let dir = repo();
        let head = commit(dir.path(), "a.txt", "feat: 基线");
        git_in(dir.path(), &["checkout", "-q", "--detach", &head]);

        let refs = list(dir.path()).expect("读引用");
        let state = state(dir.path(), &refs).expect("状态摘要");
        assert!(state.branch.is_none(), "游离 HEAD 不该编出一个分支名");
        assert!(state.upstream.is_none(), "没有当前分支就没有跟踪分支");
        assert_eq!(state.interrupt, Interrupt::None);
    }

    #[test]
    fn a_conflicted_merge_is_recognised() {
        let dir = conflict_fixture();
        let merged = process::run(Some(dir.path()), &["merge", "side"]).expect("merge");
        assert!(!merged.success, "这次合并必须冲突：{}", merged.stdout);

        let info = interrupt(dir.path()).expect("读中断态");
        assert_eq!(info.kind, Interrupt::Merge);
        assert!(info.branch.is_none(), "合并中断时 HEAD 还在原分支上");
    }

    #[test]
    fn a_conflicted_rebase_names_the_branch_being_rebased() {
        let dir = conflict_fixture();
        git_in(dir.path(), &["checkout", "-q", "side"]);
        let rebase = process::run(Some(dir.path()), &["rebase", "main"]).expect("rebase");
        assert!(!rebase.success, "这次变基必须中断");

        let info = interrupt(dir.path()).expect("读中断态");
        assert_eq!(info.kind, Interrupt::Rebase, "变基目录和 CHERRY_PICK_HEAD 同时存在时报变基");
        assert_eq!(
            info.branch.as_deref(),
            Some("side"),
            "变基时 HEAD 是游离的，只有 head-name 认得出是哪个分支"
        );
    }

    #[test]
    fn a_conflicted_cherry_pick_and_revert_are_their_own_states() {
        let dir = conflict_fixture();
        let side = git_in(dir.path(), &["rev-parse", "side"]);

        let picked = process::run(Some(dir.path()), &["cherry-pick", &side]).expect("cherry-pick");
        assert!(!picked.success, "这次摘取必须冲突");
        assert_eq!(
            interrupt(dir.path()).expect("中断态").kind,
            Interrupt::CherryPick
        );
        git_in(dir.path(), &["cherry-pick", "--abort"]);

        let reverted =
            process::run(Some(dir.path()), &["revert", "--no-edit", &side]).expect("revert");
        assert!(!reverted.success, "这次回滚必须冲突");
        assert_eq!(
            interrupt(dir.path()).expect("中断态").kind,
            Interrupt::Revert
        );
    }

    /// linked worktree 的中断态必须按那个工作区的目录去判：主仓库的 `.git/MERGE_HEAD`
    /// 一直是空的，只拼主 git-dir 就永远读不到工作区里正在进行的那次合并（实测）。
    #[test]
    fn an_interrupt_in_a_linked_worktree_is_found_there() {
        let dir = conflict_fixture();
        let worktree = tempfile::tempdir().expect("tempdir worktree");
        let target = worktree.path().to_path_buf();
        git_in(
            dir.path(),
            &[
                "worktree",
                "add",
                "-q",
                target.to_str().expect("utf-8 path"),
                "side",
            ],
        );

        let merged = process::run(Some(&target), &["merge", "main"]).expect("merge");
        assert!(!merged.success, "工作区里的这次合并必须冲突：{}", merged.stdout);

        assert_eq!(
            interrupt(&target).expect("工作区中断态").kind,
            Interrupt::Merge
        );
        assert_eq!(
            interrupt(dir.path()).expect("主仓库不该跟着报中断").kind,
            Interrupt::None
        );
    }

    #[test]
    fn a_clean_repo_reports_no_interrupt() {
        let dir = repo();
        commit(dir.path(), "a.txt", "feat: 基线");
        assert_eq!(interrupt(dir.path()).expect("读中断态").kind, Interrupt::None);
    }

    /// 两条分支在同一行上各写各的，后面的 merge / rebase / cherry-pick 必然冲突
    fn conflict_fixture() -> tempfile::TempDir {
        let dir = repo();
        fs::write(dir.path().join("a.txt"), "base\n").expect("write");
        git_in(dir.path(), &["add", "a.txt"]);
        git_in(dir.path(), &["commit", "-q", "-m", "feat: 基线"]);

        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        fs::write(dir.path().join("a.txt"), "side\n").expect("write");
        git_in(dir.path(), &["add", "a.txt"]);
        git_in(dir.path(), &["commit", "-q", "-m", "feat: 支线改法"]);

        git_in(dir.path(), &["checkout", "-q", "main"]);
        fs::write(dir.path().join("a.txt"), "main\n").expect("write");
        git_in(dir.path(), &["add", "a.txt"]);
        git_in(dir.path(), &["commit", "-q", "-m", "feat: 主干改法"]);
        dir
    }
}
