//! 合并冲突解决器（§7.13）。
//!
//! 状态来源是索引里的三个 stage，不是工作区文件：
//! - stage 1 = 共同祖先（base）
//! - stage 2 = 我方（ours，HEAD）
//! - stage 3 = 对方（theirs）
//!
//! 工作区里那个带 `<<<<<<<` 标记的文件只是"某一方留下的草稿"，不是解法。
//! 界面上的三方视图一律从 `git show :N:<path>` 取，读到的一定是入库时的原始内容。
//!
//! 全部标记完成之后才 `git add`，续跑按中断态分派（§7.13「收尾」）：
//! merge 用 `-c core.editor=true git commit`，rebase/cherry-pick 用 `--continue`。

use std::path::Path;

use serde::Serialize;

use super::process;
use super::refs::Interrupt;
use crate::error::GitError;

/// 单个 stage 的正文。二进制只给大小，不给正文（界面也不该给编辑框）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StageContent {
    pub text: Option<String>,
    pub size: u64,
    pub binary: bool,
}

/// 冲突文件的三方内容。
///
/// 三栏都可能是 `None`，各对应一种冲突：
/// 没有 base = 改删/删改/两边新增；没有 ours 或没有 theirs = 有一方把这个文件删了。
/// 界面按 `ConflictKind` 决定画几栏，不要假设一定有三栏。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Sides {
    /// 共同祖先（stage 1）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<StageContent>,
    /// 我方（stage 2）。我方删了这个文件时是 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ours: Option<StageContent>,
    /// 对方（stage 3）。对方删了这个文件时是 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theirs: Option<StageContent>,
    /// 在场的那几栏里只要有一个是二进制就是 true：界面不给这类文件开逐块合并
    pub binary: bool,
}

/// 一个未合并条目。界面上一个卡片对应一个。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub path: String,
    /// porcelain v1 的两位状态，如 `UU` / `DU` / `AA`
    pub status: String,
    pub kind: ConflictKind,
    /// `kind` 的中文说法。中文字由 Rust 侧一并给出，界面各处不自己拼
    pub label: &'static str,
    /// 能不能逐块合并。
    ///
    /// **界面直接用这一个值，不要自己从 `sides` 反推**：这里是看过索引里三个 stage
    /// 之后得出的结论，界面拿 `sides` 猜则会在改删、子模块这些三栏不全的情形下猜错，
    /// 然后给用户开一个拼不出正确结果的合并区。
    pub three_way: bool,
    /// 只能选一边（与 `three_way` 互为反面，一起给是为了界面不必自己取反）
    pub pick_side_only: bool,
    /// 双改名时对端的路径。同一个卡片里有两个文件名，界面要按对端路径配对展示
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_path: Option<String>,
    /// 工作区里那份带冲突标记的草稿。存疑时用户想看的就是它
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_text: Option<String>,
    pub sides: Sides,
}

/// 冲突类型。六类穷举在 §7.13 的验收里，每一类界面上要说清"为什么只能这么解"。
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ConflictKind {
    /// 内容冲突：两边都改了同一段，三方都在
    Content,
    /// 我方改了、对方删了（`DU`）
    ModifyDelete,
    /// 我方删了、对方改了（`UD`）
    DeleteModify,
    /// 两边都新增（`AA`）
    BothAdded,
    /// 双改名（`AA` 且两侧都有来源路径）：按对端路径配对展示
    RenameRename,
    /// 二进制：只给"选一边"
    Binary,
    /// 子模块指针（mode 160000）。那一端的提交号在本仓库里查不到，别假装能合并
    Submodule,
}

impl ConflictKind {
    /// 给用户看的一句话，决定界面上给哪些按钮（§7.13 的降级穷举）
    pub fn label(self) -> &'static str {
        match self {
            Self::Content => "内容冲突：两边改了同一段，可以逐块取舍",
            Self::ModifyDelete => "改删冲突：我方改了、对方删了这个文件",
            Self::DeleteModify => "删改冲突：我方删了这个文件、对方改了它",
            Self::BothAdded => "两边都新增了同一个文件",
            Self::RenameRename => "双改名：两边把同一个文件改成了两个名字",
            Self::Binary => "二进制冲突：只能选一边，没有逐块取舍的余地",
            Self::Submodule => "子模块冲突：选一边提交号，没有正文可合并",
        }
    }

    /// 能不能逐块合并。只有三方都在且都是文本才行。
    pub fn three_way(self) -> bool {
        matches!(self, Self::Content)
    }

    /// 能不能只靠"选一边"解决。改删、删改、二进制、子模块、双改名都只能这样。
    pub fn pick_side_only(self) -> bool {
        !self.three_way()
    }
}

/// 索引里的一条未合并记录（`git ls-files -u` 的一行）。
#[derive(Clone, Debug)]
struct StageEntry {
    /// 按路径归组用它。双改名会出现两条 path 不同、oid 相同的记录
    path: String,
    mode: String,
    oid: String,
    stage: u8,
}

/// 列出当前仓库全部未合并条目。
///
/// 走 `ls-files -u -z` 而不是从 `status` 的 `UU` 推：只有索引里有 stage 才叫冲突，
/// 界面上的卡片必须与 `git add` 之后消失的东西一一对应。
pub fn list(repo: &Path) -> Result<Vec<Conflict>, GitError> {
    let raw = process::run(Some(repo), &["ls-files", "-u", "-z"])?.expect_success()?;
    let entries = parse_unmerged(&raw)?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    let mut cards: Vec<Conflict> = Vec::new();
    for (path, group) in group_by_path(entries) {
        cards.push(build(repo, path, group)?);
    }
    // 界面按路径排序，"第一个没解决的"才稳定。git 自己的顺序是索引序，不保证可读
    cards.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(cards)
}

/// 还没解决完时返回 `NotClean`——界面据此禁用"提交/摘取"按钮，而不是让 git 报错。
pub fn ensure_all_resolved(repo: &Path) -> Result<(), GitError> {
    let left = list(repo)?;
    if left.is_empty() {
        return Ok(());
    }
    Err(GitError::NotClean {
        detail: format!("还有 {} 个文件没有解决冲突", left.len()),
    })
}

/// 解决方式：直接用某一边的内容，或者用户手改的正文。
#[derive(Debug)]
pub enum Resolution {
    /// 取我方（stage 2）
    Ours,
    /// 取对方（stage 3）
    Theirs,
    /// 手改。正文就是用户看到的，写回去的就是它——不做任何再加工
    Text(String),
}

/// 把一个冲突文件标记成已解决：写正文 → `git add`。
///
/// 走 `git add` 而不是 `update-index --cacheinfo`：用户可能是在编辑区里逐块拼的，
/// 让 git 自己去读工作区那份才是他真正看过的内容。
pub fn resolve(repo: &Path, path: &str, how: Resolution) -> Result<(), GitError> {
    // 路径先校验再碰 git。git 自己也会拒越界路径（报错难看且版本相关），
    // 但不能把防线交给对端：前端的路径原样传过来，校验必须在我们这边先做完。
    let target = absolute(repo, path)?;
    // 子模块指针不能走"写文件 + add"：它在工作区里是个目录（或根本不存在），
    // `:3:sub` 这种取法对 gitlink 也不成立。只能直接改索引里的那一条。
    if let Some(resolved) = resolve_gitlink(repo, path, &how)? {
        return Ok(resolved);
    }
    match how {
        Resolution::Ours => write_worktree(&target, &stage_bytes(repo, path, 2)?)?,
        Resolution::Theirs => write_worktree(&target, &stage_bytes(repo, path, 3)?)?,
        Resolution::Text(text) => write_worktree(&target, text.as_bytes())?,
    }
    // 空正文也要 add：解成"两边都删掉"是一个正当的答案
    process::run(Some(repo), &["add", "-A", "--", path])?.expect_success()?;
    Ok(())
}

/// 子模块冲突的解法：把选中的那个 gitlink 提交号写回索引。
///
/// 返回 `None` 表示"这个路径不是子模块冲突"，走普通的写文件那一条。
fn resolve_gitlink(repo: &Path, path: &str, how: &Resolution) -> Result<Option<()>, GitError> {
    let raw = process::run(Some(repo), &["ls-files", "-u", "-z", "--", path])?.expect_success()?;
    let entries = parse_unmerged(&raw)?;
    if !entries.iter().any(|entry| entry.mode == SUBMODULE_MODE) {
        return Ok(None);
    }

    let chosen = match how {
        Resolution::Ours => 2,
        Resolution::Theirs => 3,
        // 手改子模块就是选一个提交号。界面给的是选中那栏的 oid，不给输入框——
        // 让用户手输 40 位十六进制只会输错
        Resolution::Text(_) => {
            return Err(GitError::NotClean {
                detail: "子模块冲突只能选一边（选一边的提交号），不能手改正文".into(),
            })
        }
    };
    let Some(entry) = entries.iter().find(|entry| entry.stage == chosen) else {
        return Err(GitError::NotClean {
            detail: format!("{path} 没有第 {chosen} 版可选"),
        });
    };
    process::run(
        Some(repo),
        &["update-index", "--add", "--cacheinfo", SUBMODULE_MODE, &entry.oid, path],
    )?
    .expect_success()?;
    Ok(Some(()))
}

/// 改删/删改这类"一方没了"的文件：删掉工作区里的它并 `git add -A`。
///
/// 界面上的"接受删除"就是这一条。走工作区删除而不是索引技巧，
/// 这样解决完之后 `status` 与手搓 git 的结果完全一致。
pub fn accept_deletion(repo: &Path, path: &str) -> Result<(), GitError> {
    let target = absolute(repo, path)?;
    // Windows 上文件可能被别的进程占着，删不掉就说清楚，别把索引先改了留下不一致
    if target.exists() {
        std::fs::remove_file(&target).map_err(|err| GitError::Internal(format!("无法删除 {path}：{err}")))?;
    }
    process::run(Some(repo), &["add", "-A", "--", path])?.expect_success()?;
    Ok(())
}

/// 全部标记完之后续跑。
///
/// 按中断态分派（§7.13「收尾」）：
/// - merge：`-c core.editor=true git commit`（`--continue` 在 merge 上不存在）
/// - rebase / cherry-pick / revert：`<cmd> --continue`
///
/// 一律注入 `core.editor=true`：git 要开编辑器的地方会立刻拿到一个空编辑器返回，
/// 于是它沿用我们已经在暂存区里放好的提交信息。**绝不让编辑器弹到界面上**，
/// 那会让用户在 Tauri 窗口后面找 git 的编辑器窗口。
pub fn continue_operation(
    repo: &Path,
    kind: Interrupt,
    message: Option<&str>,
) -> Result<(), GitError> {
    if kind == Interrupt::None {
        return Err(GitError::NotClean {
            detail: "现在没有进行中的操作，没有东西可以续跑".into(),
        });
    }
    // 续跑之前必须先确认没有遗留冲突，否则 --continue 会中途再停一次
    ensure_all_resolved(repo)?;

    let args: Vec<&str> = match kind {
        Interrupt::Merge => {
            let mut args = vec!["-c", "core.editor=true"];
            if let Some(message) = message {
                args.push("commit");
                args.push("-m");
                args.push(message);
            } else {
                // 没给信息就沿用 MERGE_MSG，`core.editor=true` 让 git 立刻接受它
                args.push("commit");
                args.push("--no-edit");
            }
            args
        }
        Interrupt::Rebase => vec!["rebase", "--continue"],
        Interrupt::CherryPick => vec!["cherry-pick", "--continue"],
        Interrupt::Revert => vec!["revert", "--continue"],
        Interrupt::None => unreachable!("上面已经拒了"),
    };
    process::run_with_env(Some(repo), &args, &[("GIT_EDITOR", "true")])?.expect_success()?;
    Ok(())
}

// ---------------------------------------------------------------- 组装

fn build(repo: &Path, path: String, group: Vec<StageEntry>) -> Result<Conflict, GitError> {
    let stage = |n: u8| group.iter().find(|e| e.stage == n).cloned();
    let (base_entry, ours_entry, theirs_entry) = (stage(1), stage(2), stage(3));

    // 一个冲突条目至少要有两个 stage：一个 stage 是普通待提交文件，不是冲突。
    // 少于两个说明索引被外部改过（另一个进程、坏了的 rebase），报出来而不是画一张空卡片
    if [base_entry.is_some(), ours_entry.is_some(), theirs_entry.is_some()]
        .iter()
        .filter(|present| **present)
        .count()
        < 2
    {
        return Err(GitError::ParseFailure {
            snippet: process::snippet(&format!("{path} 的未合并记录不足两个 stage")),
        });
    }

    // 缺 stage 就是"有一方把它删了"：改删/删改两类冲突的真实形态，不做降级
    let content = |entry: Option<&StageEntry>| -> Result<Option<StageContent>, GitError> {
        entry.map(|e| content_of(repo, &e.oid, &e.mode)).transpose()
    };
    let (base, ours, theirs) = (
        content(base_entry.as_ref())?,
        content(ours_entry.as_ref())?,
        content(theirs_entry.as_ref())?,
    );

    let status = unmerged_status(base.is_some(), ours.is_some(), theirs.is_some());
    let other_path = other_path_of(repo, &path, &ours_entry, &theirs_entry)?;
    // 任一在场的是二进制或子模块指针，就不是能逐块合并的文本冲突
    let any_binary = [&base, &ours, &theirs].into_iter().flatten().any(|c| c.binary);
    let submodule = [base_entry, ours_entry, theirs_entry]
        .into_iter()
        .flatten()
        .any(|e| e.mode == SUBMODULE_MODE);
    let kind = classify(&status, submodule, any_binary, other_path.is_some());

    Ok(Conflict {
        worktree_text: worktree_text(repo, &path)?,
        sides: Sides {
            base,
            ours,
            theirs,
            binary: any_binary,
        },
        path,
        label: kind.label(),
        three_way: kind.three_way(),
        pick_side_only: kind.pick_side_only(),
        status,
        kind,
        other_path,
    })
}

/// 冲突类型判定。判据是 §7.13 穷举的六类。
///
/// 顺序即优先级：子模块先于一切（它压根没有正文），然后按 stage 的组合，
/// 最后才是"有祖先的三方都在"里的文本 / 二进制之分。
fn classify(status: &str, submodule: bool, binary: bool, rename_rename: bool) -> ConflictKind {
    if submodule {
        return ConflictKind::Submodule;
    }
    // git 的 ours = HEAD = 当前分支，theirs = 被合进来的那个分支，
    // 所以 UD（deleted by them）与 DU（deleted by us）不能弄反
    match status {
        // UD = deleted by them：我方改了、对方删了
        "UD" => ConflictKind::ModifyDelete,
        // DU = deleted by us：我方删了、对方改了
        "DU" => ConflictKind::DeleteModify,
        "AA" => {
            if rename_rename {
                ConflictKind::RenameRename
            } else {
                ConflictKind::BothAdded
            }
        }
        // UU：三方都在且都是文本才谈得上逐块合并
        _ if binary => ConflictKind::Binary,
        _ => ConflictKind::Content,
    }
}

/// 从三个 stage 的存在性反推出 porcelain 的两位状态。
///
/// `git ls-files -u` 只说"有哪些 stage"，不说两位状态；两位状态是 status 的语义。
/// 但两者一一对应，所以按 stage 的组合重建即可，不必再跑一次 status：
/// 1+2+3 = `UU`，1+2 = 对方删了（`UD`），1+3 = 我方删了（`DU`），2+3 = 两边都新增（`AA`）。
fn unmerged_status(has_base: bool, has_ours: bool, has_theirs: bool) -> String {
    match (has_base, has_ours, has_theirs) {
        (true, true, true) => "UU",
        (true, true, false) => "UD",
        (true, false, true) => "DU",
        (false, true, true) => "AA",
        // 只有两个 stage 的组合在这里不会出现（那样就不是冲突了），落到 UU 是最接近的读法
        _ => "UU",
    }
    .to_string()
}

/// 双改名时找对端路径。两边都新增（AA）且两个路径指向同一个 blob，就是双改名。
fn other_path_of(
    repo: &Path,
    path: &str,
    ours: &Option<StageEntry>,
    theirs: &Option<StageEntry>,
) -> Result<Option<String>, GitError> {
    let same_blob = ours
        .as_ref()
        .zip(theirs.as_ref())
        .is_some_and(|(ours, theirs)| ours.oid == theirs.oid);
    if !same_blob {
        return Ok(None);
    }
    let raw = process::run(Some(repo), &["ls-files", "-u", "-z", "--", path])?.expect_success()?;
    let entries = parse_unmerged(&raw)?;
    let same_blob = entries
        .iter()
        .find(|e| e.stage == 2)
        .zip(entries.iter().find(|e| e.stage == 3))
        .is_some_and(|(ours, theirs)| ours.oid == theirs.oid);
    if !same_blob {
        return Ok(None);
    }
    // 同一个 blob 挂在两个路径上就是双改名，把不是自己的那个路径作为"对端"
    Ok(entries_paths(repo, path).into_iter().find(|p| p != path))
}

/// 同一个冲突涉及的其它路径。双改名时那就是对端那个路径。
fn entries_paths(repo: &Path, path: &str) -> Vec<String> {
    match process::run(Some(repo), &["ls-files", "-u", "-z", "--", path]) {
        Ok(out) => match out.expect_success() {
            Ok(raw) => parse_unmerged(&raw)
                .unwrap_or_default()
                .into_iter()
                .map(|e| e.path)
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

/// 读一个 stage 的内容。
///
/// 子模块指针（mode 160000）必须**先**短路：它在库里是一个 gitlink，本仓库里查不到
/// 那个 commit 对象，`cat-file -s` 会直接 fatal。所以大小报 0（“不知道”）而不是报错——
/// 界面只需要知道它没有正文。
fn content_of(repo: &Path, oid: &str, mode: &str) -> Result<StageContent, GitError> {
    if mode == SUBMODULE_MODE {
        return Ok(StageContent { text: None, size: 0, binary: true });
    }

    let size = process::run(Some(repo), &["cat-file", "-s", oid])?.expect_success()?;
    let size = size.trim().parse::<u64>().map_err(|_| GitError::ParseFailure {
        snippet: process::snippet(&format!("{oid} 的大小读不出来")),
    })?;

    let bytes = process::run_bytes(Some(repo), &["cat-file", "blob", oid], &[])?.expect_success()?;
    // 文本判定只看前 8 KB：扫完几 MB 只为了找一个 NUL 不值当
    let binary = bytes[..bytes.len().min(8192)].contains(&0);
    Ok(StageContent {
        text: (!binary).then(|| String::from_utf8_lossy(&bytes).into_owned()),
        size,
        binary,
    })
}

/// stage 的原始字节。取"我方/对方"时按字节原样落盘，不经任何解码——
/// 走一次 UTF-8 往返就把二进制文件写坏了。
fn stage_bytes(repo: &Path, path: &str, stage: u8) -> Result<Vec<u8>, GitError> {
    let spec = format!(":{stage}:{path}");
    // 该 stage 不存在就是"这一方把这个文件删了"，取它的界面按钮本来就不该亮。
    // 走到这里说明是绕过界面直接调命令，报出来而不是当成空文件写回去。
    let out = process::run(Some(repo), &["cat-file", "-e", &spec])?;
    if !out.success {
        return Err(GitError::NotClean {
            detail: format!(
                "{path} 没有第 {stage} 版（{}），要接受删除请用「接受删除」",
                if stage == 2 { "我方把它删了" } else { "对方把它删了" }
            ),
        });
    }
    process::run_bytes(Some(repo), &["cat-file", "blob", &spec], &[])?.expect_success()
}

/// 工作区里那份带 `<<<<<<<` 标记的草稿。不存在或读不出（子模块目录、二进制）时是 None。
fn worktree_text(repo: &Path, path: &str) -> Result<Option<String>, GitError> {
    let target = absolute(repo, path)?;
    let Ok(bytes) = std::fs::read(&target) else {
        return Ok(None);
    };
    if bytes.len() > MAX_WORKTREE_BYTES || bytes[..bytes.len().min(8192)].contains(&0) {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// 工作区草稿最多读这么多字节。冲突文件基本都是文本，超过这个量级的多半是生成物。
const MAX_WORKTREE_BYTES: usize = 1024 * 1024;
const SUBMODULE_MODE: &str = "160000";

/// 仓库路径 → 绝对路径，并且**拒绝越出仓库根**的路径。
///
/// 路径是从界面传进来的：`.git` 之类的前缀一旦拼出来就是任意文件写。
/// 界面给的路径本来也只可能是索引里那些，但校验放在 Rust 侧才有效。
fn absolute(repo: &Path, path: &str) -> Result<std::path::PathBuf, GitError> {
    if path.is_empty() || path.contains('\0') {
        return Err(GitError::ParseFailure { snippet: process::snippet(path) });
    }
    let root = repo
        .canonicalize()
        .map_err(|_| GitError::NotARepo)?;
    let joined = root.join(path);
    let normalized = normalize(&joined);
    if !normalized.starts_with(&root) {
        return Err(GitError::ParseFailure {
            snippet: process::snippet(path),
        });
    }
    Ok(normalized)
}

/// 手写一遍 `.` / `..` 的折叠。`canonicalize` 在文件还不存在时（"接受删除"那条路）
/// 会对不上，所以这里只用纯词法的方式，跨盘符和符号链接交回给操作系统。
fn normalize(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 写工作区文件。父目录先建出来：双改名时目标路径所在目录可能还没建。
fn write_worktree(target: &Path, bytes: &[u8]) -> Result<(), GitError> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| GitError::Internal(format!("无法创建 {}：{err}", parent.display())))?;
    }
    std::fs::write(target, bytes).map_err(|err| GitError::Internal(format!("无法写入文件：{err}")))
}

// ---------------------------------------------------------------- 解析

/// `git ls-files -u -z` 的记录：`<mode> SP <oid> SP <stage> TAB <path>`，一条一个 NUL。
///
/// 路径那一段按第一个 TAB 原样取：git 的路径里允许有空格和换行，用空白切会把它切碎。
fn parse_unmerged(raw: &str) -> Result<Vec<StageEntry>, GitError> {
    let mut entries = Vec::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((head, path)) = record.split_once('\t') else {
            return Err(GitError::ParseFailure {
                snippet: process::snippet(record),
            });
        };
        let fields: Vec<&str> = head.split_whitespace().collect();
        if fields.len() != 3 || path.is_empty() {
            return Err(GitError::ParseFailure {
                snippet: process::snippet(record),
            });
        }
        let stage = fields[2].parse::<u8>().map_err(|_| GitError::ParseFailure {
            snippet: process::snippet(record),
        })?;
        entries.push(StageEntry {
            path: path.to_string(),
            mode: fields[0].to_string(),
            oid: fields[1].to_string(),
            stage,
        });
    }
    Ok(entries)
}

/// 按路径把 stage 归组。一个路径只会有一次；双改名是两条不同 path 的记录，
/// 各自成组，再由 `other_path_of` 把它们配成一对。
fn group_by_path(entries: Vec<StageEntry>) -> Vec<(String, Vec<StageEntry>)> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<StageEntry>> =
        std::collections::HashMap::new();
    for entry in entries {
        let bucket = groups.entry(entry.path.clone()).or_insert_with(|| {
            order.push(entry.path.clone());
            Vec::new()
        });
        bucket.push(entry);
    }
    order
        .into_iter()
        .filter_map(|path| groups.get(&path).map(|group| (path, group.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::refs;

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
        git_in(dir.path(), &["config", "core.autocrlf", "false"]);
        dir
    }

    fn commit(dir: &Path, message: &str) {
        git_in(dir, &["add", "-A"]);
        git_in(dir, &["commit", "-q", "-m", message]);
    }

    /// 只提交当前索引，不 `add -A`。
    ///
    /// 子模块用例里外层仓库的 `child/` 是个真目录，`add -A` 会把它当普通文件收进去，
    /// 那个冲突就把测试自己的脚手架给盖住了。外层只改索引里的 gitlink，不能 `add -A`。
    fn commit_index(dir: &Path, message: &str) {
        git_in(dir, &["commit", "-q", "-m", message]);
    }

    /// 跑一条**预期会失败**的 git 命令（造冲突的那些）。退出码非 0 才是对的：
    /// 合并成功了就说明这个用例根本没造出冲突，得当场报错而不是继续断言。
    fn git_conflicting(dir: &Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(!out.success, "git {args:?} 居然成功了，没造出冲突");
    }

    fn write(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, content).expect("write");
    }

    /// 造一个内容冲突：基线之后两条分支各改同一段。
    /// main 是 HEAD，所以它的内容是 ours，`side` 的内容是 theirs。
    fn content_conflict() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = repo();
        let path = dir.path().to_path_buf();
        write(&path, "a.txt", "第一行\n第二行\n第三行\n");
        commit(&path, "feat: 基线");

        git_in(&path, &["checkout", "-q", "-b", "side"]);
        write(&path, "a.txt", "第一行\n并入\n第三行\n");
        commit(&path, "feat: 并入方");

        git_in(&path, &["checkout", "-q", "main"]);
        write(&path, "a.txt", "第一行\n主线\n第三行\n");
        commit(&path, "feat: 主线");

        git_conflicting(&path, &["merge", "side"]);
        (dir, path)
    }

    #[test]
    fn a_content_conflict_carries_three_sides() {
        let (_dir, path) = content_conflict();

        let cards = list(&path).expect("读冲突列表");
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!(card.path, "a.txt");
        assert_eq!(card.status, "UU");
        assert_eq!(card.kind, ConflictKind::Content);
        assert!(card.kind.three_way(), "内容冲突要能逐块合并");

        let sides = &card.sides;
        assert_eq!(sides.base.as_ref().unwrap().text.as_deref(), Some("第一行\n第二行\n第三行\n"));
        assert_eq!(sides.ours.as_ref().unwrap().text.as_deref(), Some("第一行\n主线\n第三行\n"), "ours 是 HEAD，也就是 main");
        assert_eq!(sides.theirs.as_ref().unwrap().text.as_deref(), Some("第一行\n并入\n第三行\n"), "theirs 是被合进来的 side");
        assert!(!sides.binary);

        // 三方视图必须来自 stage，不能是工作区那份带标记的草稿
        let draft = card.worktree_text.as_deref().expect("有草稿");
        assert!(draft.contains("<<<<<<<"), "工作区里留的应该是带标记的版本");
        assert!(!sides.ours.as_ref().unwrap().text.as_deref().unwrap().contains("<<<<<<<"));
    }

    #[test]
    fn taking_one_side_marks_the_file_resolved() {
        let (_dir, path) = content_conflict();

        resolve(&path, "a.txt", Resolution::Theirs).expect("取对方");

        assert!(list(&path).expect("列表").is_empty(), "解决之后索引里不该再有 stage");
        assert_eq!(
            std::fs::read_to_string(path.join("a.txt")).expect("读回"),
            "第一行\n并入\n第三行\n",
            "工作区要落成 stage 3 的原样内容，不能还带着标记"
        );
        // 与手搓 git 等价：索引里正好一条已暂存的修改
        let staged = git_in(&path, &["diff", "--cached", "--name-only"]);
        assert_eq!(staged, "a.txt");
    }

    #[test]
    fn a_hand_edited_result_is_written_back_verbatim() {
        let (_dir, path) = content_conflict();

        resolve(&path, "a.txt", Resolution::Text("第一行\n拼好的\n第三行\n".into()))
            .expect("手改结果");

        assert!(list(&path).expect("列表").is_empty());
        assert_eq!(
            std::fs::read_to_string(path.join("a.txt")).expect("读回"),
            "第一行\n拼好的\n第三行\n",
            "写回的就是用户看到的那份，不做二次加工"
        );
    }

    #[test]
    fn a_modify_delete_conflict_only_offers_the_two_sides() {
        // side 删了它、main 改了它，合进来时是"对方删了这个文件"
        let dir = repo();
        let path = dir.path().to_path_buf();
        write(&path, "gone.txt", "内容\n");
        commit(&path, "feat: 基线");

        git_in(&path, &["checkout", "-q", "-b", "side"]);
        std::fs::remove_file(path.join("gone.txt")).expect("删");
        commit(&path, "feat: 我方删了");

        git_in(&path, &["checkout", "-q", "main"]);
        write(&path, "gone.txt", "对方改的内容\n");
        commit(&path, "feat: 对方改了");

        git_conflicting(&path, &["merge", "side"]);

        let card = &list(&path).expect("列表")[0];
        assert_eq!(card.kind, ConflictKind::ModifyDelete);
        assert!(card.kind.pick_side_only(), "改删只能选一边");
        assert!(!card.kind.three_way(), "没有祖先可当基准，不能逐块合并");
        assert!(card.sides.base.is_some(), "改删仍然有共同祖先，三方里只剩两方");
        assert!(card.sides.theirs.is_none(), "对方删了这个文件，没有第三版");
        assert!(
            card.sides.ours.is_some(),
            "主线改过它，我方那一份还在，取我方是正当的解法"
        );

        accept_deletion(&path, "gone.txt").expect("接受删除");
        assert!(list(&path).expect("列表").is_empty());
        assert!(!path.join("gone.txt").exists(), "接受删除要把工作区里的它也删掉");
    }

    #[test]
    fn a_delete_modify_conflict_can_be_resolved_by_taking_their_side() {
        let dir = repo();
        let path = dir.path().to_path_buf();
        write(&path, "gone.txt", "内容\n");
        commit(&path, "feat: 基线");

        git_in(&path, &["checkout", "-q", "-b", "side"]);
        write(&path, "gone.txt", "对方改的内容\n");
        commit(&path, "feat: 对方改了");

        git_in(&path, &["checkout", "-q", "main"]);
        std::fs::remove_file(path.join("gone.txt")).expect("删");
        commit(&path, "feat: 我方删了");

        git_conflicting(&path, &["merge", "side"]);

        let card = &list(&path).expect("列表")[0];
        assert_eq!(card.kind, ConflictKind::DeleteModify);

        resolve(&path, "gone.txt", Resolution::Theirs).expect("取对方");
        assert!(list(&path).expect("列表").is_empty());
        assert!(path.join("gone.txt").exists(), "取对方要把文件写回来");
    }

    #[test]
    fn a_binary_conflict_only_offers_a_side() {
        let dir = repo();
        let path = dir.path().to_path_buf();
        std::fs::write(path.join("a.bin"), [0u8, 1, 2]).expect("write");
        commit(&path, "feat: 基线");

        git_in(&path, &["checkout", "-q", "-b", "side"]);
        std::fs::write(path.join("a.bin"), [0u8, 9, 9, 9]).expect("write");
        commit(&path, "feat: 并入方");

        git_in(&path, &["checkout", "-q", "main"]);
        std::fs::write(path.join("a.bin"), [0u8, 8, 8, 8, 8]).expect("write");
        commit(&path, "feat: 主线");

        git_conflicting(&path, &["merge", "side"]);

        let card = &list(&path).expect("列表")[0];
        assert_eq!(card.kind, ConflictKind::Binary);
        assert!(card.sides.binary);
        assert!(card.sides.ours.as_ref().unwrap().text.is_none(), "二进制不给正文");
        assert!(card.worktree_text.is_none(), "二进制草稿也不给");

        // 取一边必须按字节原样落盘：走一次 UTF-8 往返就把文件写坏了。theirs 是 side 那份
        resolve(&path, "a.bin", Resolution::Theirs).expect("取对方");
        assert_eq!(
            std::fs::read(path.join("a.bin")).expect("读回"),
            vec![0u8, 9, 9, 9]
        );
    }

    #[test]
    fn a_submodule_conflict_is_its_own_kind_with_no_text() {
        let dir = repo();
        let path = dir.path().to_path_buf();
        let child = path.join("child");
        std::fs::create_dir_all(&child).expect("mkdir");
        git_in(&child, &["init", "-q", "-b", "main", "."]);
        git_in(&child, &["config", "user.name", "测试者"]);
        git_in(&child, &["config", "user.email", "t@example.com"]);
        write(&child, "b.txt", "2\n");
        commit(&child, "feat: 子仓库基线");
        let child_base = git_in(&child, &["rev-parse", "HEAD"]);
        git_in(
            &path,
            &["update-index", "--add", "--cacheinfo", "160000", &child_base, "sub"],
        );
        commit_index(&path, "feat: 加子模块");

        // 外层仓库：main 上立基线，分出 side 推一版 gitlink，回到 main 再推一版。
        // 两次 update-index 都要 --add：mode 相同但 oid 变了，git 也不认作已存在的条目
        git_in(&path, &["checkout", "-q", "-b", "side"]);
        git_in(&child, &["checkout", "-q", "-b", "side"]);
        write(&child, "b.txt", "并入\n");
        commit(&child, "feat: 并入方推了");
        git_in(
            &path,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000",
                &git_in(&child, &["rev-parse", "HEAD"]),
                "sub",
            ],
        );
        commit_index(&path, "feat: 并入方推了");

        git_in(&child, &["checkout", "-q", "main"]);
        write(&child, "b.txt", "主线\n");
        commit(&child, "feat: 主线推了");
        git_in(&path, &["checkout", "-q", "main"]);
        git_in(
            &path,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000",
                &git_in(&child, &["rev-parse", "HEAD"]),
                "sub",
            ],
        );
        commit_index(&path, "feat: 主线推了");
        git_conflicting(&path, &["merge", "side"]);

        let cards = list(&path).expect("读冲突列表");
        assert_eq!(cards.len(), 1, "只该有子模块这一个冲突：{cards:?}");
        let card = &cards[0];
        assert_eq!(card.path, "sub");
        assert_eq!(card.kind, ConflictKind::Submodule);
        assert!(card.sides.ours.as_ref().unwrap().text.is_none(), "子模块指针没有正文");
        assert!(card.sides.theirs.as_ref().unwrap().text.is_none());
        assert!(card.kind.pick_side_only());

        // 选一边就是选一个 gitlink 提交号，不需要正文
        resolve(&path, "sub", Resolution::Theirs).expect("取对方");
        assert!(list(&path).expect("列表").is_empty());
    }

    #[test]
    fn the_merger_flags_agree_with_the_kind_they_come_from() {
        // 这两个标志是给界面的：它靠它们决定开不开逐块合并。自己从 `sides` 反推会在
        // 改删、子模块这类三栏不全的情形下猜错，然后开出一个拼不出正确结果的合并区。
        // 所以标志与 `kind` 的关系在这里钉住，不靠调用方自觉。
        for kind in [
            ConflictKind::Content,
            ConflictKind::ModifyDelete,
            ConflictKind::DeleteModify,
            ConflictKind::BothAdded,
            ConflictKind::RenameRename,
            ConflictKind::Binary,
            ConflictKind::Submodule,
        ] {
            assert!(kind.three_way() != kind.pick_side_only(), "{kind:?}: 两个标志必须互为反面");
            assert!(!kind.label().is_empty(), "{kind:?}: 每一种都要有中文说法");
        }
        // 只有内容冲突才谈得上逐块合并，这是 §7.13 降级穷举的核心一条
        assert!(ConflictKind::Content.three_way());
        for kind in [
            ConflictKind::ModifyDelete,
            ConflictKind::DeleteModify,
            ConflictKind::BothAdded,
            ConflictKind::RenameRename,
            ConflictKind::Binary,
            ConflictKind::Submodule,
        ] {
            assert!(kind.pick_side_only(), "{kind:?} 只能选一边");
        }
    }

    #[test]
    fn an_untouched_repository_has_no_conflicts() {
        let dir = repo();
        write(dir.path(), "a.txt", "1\n");
        commit(dir.path(), "feat: 基线");

        assert!(list(dir.path()).expect("列表").is_empty());
        assert!(ensure_all_resolved(dir.path()).is_ok());
    }

    #[test]
    fn unresolved_files_block_the_continuation() {
        let (_dir, path) = content_conflict();

        let kind = refs::interrupt(&path).expect("中断态").kind;
        assert_eq!(kind, Interrupt::Merge);

        let err = continue_operation(&path, kind, None).expect_err("没解决完不许续跑");
        assert!(matches!(err, GitError::NotClean { .. }), "{err:?}");
        assert_eq!(
            refs::interrupt(&path).expect("中断态").kind,
            Interrupt::Merge,
            "失败的续跑不能把中断态吃掉"
        );
    }

    #[test]
    fn resolving_everything_lets_the_merge_finish() {
        let (_dir, path) = content_conflict();
        let before = git_in(&path, &["rev-parse", "HEAD"]);

        resolve(&path, "a.txt", Resolution::Ours).expect("取我方");
        continue_operation(&path, Interrupt::Merge, None).expect("续跑");

        assert_eq!(
            refs::interrupt(&path).expect("中断态").kind,
            Interrupt::None,
            "merge 应该收尾了"
        );
        let parents = git_in(&path, &["rev-list", "--parents", "-n", "1", "HEAD"]);
        assert_eq!(
            parents.split_whitespace().count(),
            3,
            "结果要是一个两父的合并提交：{parents}"
        );
        assert_ne!(git_in(&path, &["rev-parse", "HEAD"]), before);
        assert_eq!(
            std::fs::read_to_string(path.join("a.txt")).expect("读回"),
            "第一行\n主线\n第三行\n",
            "续跑之后工作区要停在合并结果上"
        );
    }

    #[test]
    fn a_continuation_never_opens_an_editor() {
        // `core.editor=true` 与 GIT_EDITOR 一起注入，merge 的续跑不该等着人来敲键
        let (_dir, path) = content_conflict();
        std::fs::write(
            path.join(".git").join("MERGE_MSG"),
            "feat: 合起来\n",
        )
        .expect("写 MERGE_MSG");

        resolve(&path, "a.txt", Resolution::Ours).expect("取我方");
        // 不给超时：编辑器真弹出来的话这个用例会挂住，而不是安静地失败
        continue_operation(&path, Interrupt::Merge, None).expect("续跑");
        assert!(git_in(&path, &["log", "-1", "--pretty=%s"]).contains("合起来"));
    }

    #[test]
    fn a_cherry_pick_continuation_keeps_its_own_command() {
        let dir = repo();
        let path = dir.path().to_path_buf();
        write(&path, "a.txt", "第一行\n第二行\n");
        commit(&path, "feat: 基线");
        git_in(&path, &["checkout", "-q", "-b", "side"]);
        write(&path, "a.txt", "第一行\n被摘的\n第二行\n");
        commit(&path, "feat: 被摘的提交");
        let picked = git_in(&path, &["rev-parse", "HEAD"]);

        git_in(&path, &["checkout", "-q", "main"]);
        write(&path, "a.txt", "第一行\n主线的\n第二行\n");
        commit(&path, "feat: 主线的提交");

        let out = process::run(Some(&path), &["cherry-pick", &picked]).expect("spawn");
        assert!(!out.success, "这里本来就要冲突");
        assert_eq!(
            refs::interrupt(&path).expect("中断态").kind,
            Interrupt::CherryPick
        );

        // 取 theirs（被摘的那个提交的内容）。取 ours 会让这次摘取变成空提交，
        // --continue 会报 "now empty" —— 那不是解决器的 bug，是真的没东西可摘
        resolve(&path, "a.txt", Resolution::Theirs).expect("解决");
        assert_eq!(
            std::fs::read_to_string(path.join("a.txt")).expect("读回"),
            "第一行\n被摘的\n第二行\n"
        );
        continue_operation(&path, Interrupt::CherryPick, None).expect("续跑");
        assert_eq!(refs::interrupt(&path).expect("中断态").kind, Interrupt::None);
        assert_eq!(git_in(&path, &["log", "-1", "--pretty=%s"]), "feat: 被摘的提交");
    }

    #[test]
    fn a_path_escaping_the_repository_is_refused() {
        let dir = repo();
        write(dir.path(), "a.txt", "1\n");
        commit(dir.path(), "feat: 基线");

        for evil in ["../outside.txt", "a/../../outside.txt", "", "a\0b"] {
            let err = resolve(dir.path(), evil, Resolution::Ours)
                .expect_err("越界的路径必须被拒");
            assert!(
                matches!(err, GitError::ParseFailure { .. }),
                "{evil:?} 不该被放行：{err:?}"
            );
        }
        assert!(!dir.path().parent().unwrap().join("outside.txt").exists());
    }

    #[test]
    fn a_malformed_unmerged_record_is_a_parse_failure() {
        assert!(matches!(
            parse_unmerged("这不是记录").expect_err("坏记录"),
            GitError::ParseFailure { .. }
        ));
        assert!(matches!(
            parse_unmerged("100644 abc 1 没有制表符\0").expect_err("没有 TAB"),
            GitError::ParseFailure { .. }
        ));
        assert!(matches!(
            parse_unmerged("100644 abc x\t文件.txt\0").expect_err("stage 不是数字"),
            GitError::ParseFailure { .. }
        ));
    }

    #[test]
    fn a_path_with_spaces_survives_the_round_trip() {
        let (_dir, path) = {
            let dir = repo();
            let path = dir.path().to_path_buf();
            write(&path, "有 空格.txt", "第一行\n第二行\n");
            commit(&path, "feat: 基线");
            git_in(&path, &["checkout", "-q", "-b", "side"]);
            write(&path, "有 空格.txt", "第一行\n并入\n");
            commit(&path, "feat: 并入方");
            git_in(&path, &["checkout", "-q", "main"]);
            write(&path, "有 空格.txt", "第一行\n主线\n");
            commit(&path, "feat: 主线");
            git_conflicting(&path, &["merge", "side"]);
            (dir, path)
        };

        let card = &list(&path).expect("列表")[0];
        assert_eq!(card.path, "有 空格.txt", "路径按第一个 TAB 原样取，不被空格切碎");
        resolve(&path, "有 空格.txt", Resolution::Theirs).expect("解决");
        assert!(list(&path).expect("列表").is_empty());
    }
}
