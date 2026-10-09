use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// 一个文件在这次提交里的改动类型。字母来自 `--raw` 的状态位，与 `--name-status` 同源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeStatus {
    Add,
    Modify,
    Delete,
    Rename,
    Copy,
    /// 类型变更（普通文件 ↔ 符号链接 ↔ 子模块那类）
    TypeChange,
}

/// `--raw` 的一条记录，还没和行数、大小合起来。
struct Raw {
    status: ChangeStatus,
    score: Option<usize>,
    old_mode: String,
    new_mode: String,
    old_blob: Option<String>,
    new_blob: Option<String>,
    /// rename / copy 才有：来源路径
    old_path: Option<String>,
    path: String,
}

impl Raw {
    /// 这条记录在 `-z` 的 numstat 里是不是多占两个路径 token（改名/复制）
    fn renamed(&self) -> bool {
        self.old_path.is_some()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// 展示与点开 diff 用的路径：rename 是**新**路径
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    /// rename / copy 的相似度（`R100` 的那个 100）
    pub score: Option<usize>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    /// 只改权限没改内容：两个 blob 相同、模式不同
    pub mode_only: bool,
    /// 类型变更：mode 是 160000 或 120000，或两侧模式类别不同
    pub gitlink: bool,
    /// 子模块指针的两端提交号（§7.4 的"显示 old/new sha"）。只有 gitlink 才填，
    /// 普通文件的 blob 号对界面没有意义，不该跟着每个文件走一遍 IPC
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    pub added: Option<usize>,
    pub deleted: Option<usize>,
    /// numstat 给 `-`：二进制文件没有行数差
    pub binary: bool,
    /// 二进制只给大小差（§7.4）。取不到时为 None，界面退化成"二进制文件"
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub sha: String,
    pub parents: Vec<String>,
    /// 合并提交：改动文件是**对第一父**的差集，界面必须标出来（§7.4）
    pub merge: bool,
    pub changes: Vec<Change>,
}

/// 读一条提交的改动文件清单 + 行数差。
///
/// browse（treeless）仓库允许读：`--raw` 只要 commit/tree 对象，界面点开某个文件才会
/// 触发按需取 blob（§7.5）。
pub fn read(repo: &Path, sha: &str) -> Result<Detail, GitError> {
    let parents = parents(repo, sha)?;
    let merge = parents.len() > 1;

    let raw_args = record_args(sha, merge, "--raw");
    let raw = process::run(Some(repo), &process::strs(&raw_args))?.expect_success()?;
    let records = parse_raw(&raw)?;

    let count_args = record_args(sha, merge, "--numstat");
    let numstat = process::run(Some(repo), &process::strs(&count_args))?.expect_success()?;
    let counts = counts_for(&records, &numstat)?;

    let mut changes = Vec::with_capacity(records.len());
    // 与 changes 同序：二进制那一格记下要向 cat-file 查的两个 sha，取不到的一侧是 None。
    // gitlink 排在这里外面——子模块那个提交对象在本仓库里通常根本没有，查了也是 missing。
    let mut blobs: Vec<(Option<String>, Option<String>)> = Vec::with_capacity(records.len());
    for (record, (added, deleted)) in records.into_iter().zip(counts) {
        let gitlink = record.old_mode == GITLINK_MODE || record.new_mode == GITLINK_MODE;
        let binary = added.is_none() && deleted.is_none();
        // 先算再搬：下面 `Some(record.old_mode)` 那两行会把模式串搬进 Change 里，
        // 搬完再比就是 use-after-move
        let mode_only =
            record.old_blob == record.new_blob && record.old_mode != record.new_mode && !gitlink;
        // 子模块那两端是提交号，界面要照着 §7.4 摆出来给人对比；普通文件的 blob 号不填
        let oids = if gitlink {
            (record.old_blob.clone(), record.new_blob.clone())
        } else {
            (None, None)
        };
        blobs.push(if binary && !gitlink {
            (record.old_blob.clone(), record.new_blob.clone())
        } else {
            (None, None)
        });
        changes.push(Change {
            path: record.path,
            old_path: record.old_path,
            status: record.status,
            score: record.score,
            old_mode: Some(record.old_mode),
            new_mode: Some(record.new_mode),
            mode_only,
            gitlink,
            old_oid: oids.0,
            new_oid: oids.1,
            added,
            deleted,
            binary,
            old_size: None,
            new_size: None,
        });
    }

    fill_sizes(repo, &mut changes, &blobs)?;

    Ok(Detail {
        sha: sha.to_string(),
        parents,
        merge,
        changes,
    })
}

/// 子模块指针在 raw 里的 mode。看到它就知道那两个 sha 字段是子模块的提交号，不是 blob。
const GITLINK_MODE: &str = "160000";

/// show 处理普通提交和根提交（根提交本来就没有父，`show` 直接给全量新增）；
/// 合并提交 `show` 什么都不输出，所以显式对第一父做差。
fn record_args(sha: &str, merge: bool, format: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        if merge { "diff" } else { "show" }.to_string(),
        format.to_string(),
        "-z".to_string(),
        // 状态里要带 R/C 的相似度分数（§7.4）。只在本已改动的文件之间找副本，
        // 不加 --find-copies-harder：那会把整个树拖进来扫一遍。
        "--find-copies".to_string(),
        // raw 默认把对象号缩成 7 位，而二进制的大小要拿去喂 cat-file、gitlink 的 sha 要展示，
        // 两个都得是完整 40 位。
        "--no-abbrev".to_string(),
    ];
    if merge {
        args.push(format!("{sha}^1"));
        args.push(sha.to_string());
    } else {
        args.push("--format=".to_string());
        args.push(sha.to_string());
    }
    args
}

/// 提交本身的父列表。`rev-list --parents` 一行给"自己 父1 父2 …"，
/// 一次调用同时确认"这条存在"和"它有几个父"。
///
/// diff 那一层也要它来判断合并提交（合并的 `show` 什么都不输出），所以给到 crate 内。
pub(crate) fn parents(repo: &Path, sha: &str) -> Result<Vec<String>, GitError> {
    let stdout =
        process::run(Some(repo), &["rev-list", "--parents", "-n", "1", sha])?.expect_success()?;
    let line = stdout.lines().next().unwrap_or_default();
    let mut fields = line.split_whitespace();
    // 第一个字段是提交自己，剩下的才是父
    fields.next();
    Ok(fields.map(str::to_string).collect())
}

fn parse_raw(stdout: &str) -> Result<Vec<Raw>, GitError> {
    let tokens = z_tokens(stdout);
    let mut records: Vec<Raw> = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let token = tokens[index];
        let Some(head) = token.strip_prefix(':') else {
            return Err(parse_failure(&tokens, index, "raw 记录不以 : 开头"));
        };
        // 两种形态都要接：`:… M\0path\0`（路径自己一个 token）和 `:… M\tpath\0`（同一个 token）
        let (head, inline) = match head.split_once('\t') {
            Some((head, path)) => (head, Some(path)),
            None => (head, None),
        };
        let fields: Vec<&str> = head.split_whitespace().collect();
        let [old_mode, new_mode, old_blob, new_blob, state] = fields.as_slice() else {
            return Err(parse_failure(&tokens, index, "raw 头部字段数不对"));
        };
        let (status, score) = match_status(state)?;

        index += 1;
        // 路径：内联形态先吃掉 token 里的那一段，剩下的从后续 token 取
        let mut paths: Vec<&str> = Vec::new();
        if let Some(path) = inline {
            if !path.is_empty() {
                paths.push(path);
            }
        }
        let want = if matches!(status, ChangeStatus::Rename | ChangeStatus::Copy) {
            2
        } else {
            1
        };
        while paths.len() < want {
            if index >= tokens.len() {
                return Err(parse_failure(&tokens, index, "raw 记录缺路径字段"));
            }
            paths.push(tokens[index]);
            index += 1;
        }

        let path = paths[want - 1];
        records.push(Raw {
            status,
            score,
            old_mode: (*old_mode).to_string(),
            new_mode: (*new_mode).to_string(),
            old_blob: full_sha(old_blob),
            new_blob: full_sha(new_blob),
            old_path: if want == 2 {
                Some(paths[0].to_string())
            } else {
                None
            },
            path: path.to_string(),
        });
    }

    Ok(records)
}

fn match_status(state: &str) -> Result<(ChangeStatus, Option<usize>), GitError> {
    if let Some(rest) = state.strip_prefix('R') {
        return Ok((ChangeStatus::Rename, Some(percent(rest)?)));
    }
    if let Some(rest) = state.strip_prefix('C') {
        return Ok((ChangeStatus::Copy, Some(percent(rest)?)));
    }
    let status = match state {
        "A" => ChangeStatus::Add,
        "M" => ChangeStatus::Modify,
        "D" => ChangeStatus::Delete,
        "T" => ChangeStatus::TypeChange,
        other => {
            return Err(GitError::ParseFailure {
                snippet: format!("未知的 raw 状态：{other}"),
            })
        }
    };
    Ok((status, None))
}

fn percent(raw: &str) -> Result<usize, GitError> {
    raw.parse::<usize>().map_err(|_| GitError::ParseFailure {
        snippet: format!("相似度不是数字：{raw}"),
    })
}

/// raw 的对象号：新增侧是 40 个 0，删除侧同理，那不是能查的 sha。
/// 位数不对说明 `--no-abbrev` 没生效（缩写的号既查不到也用不了），当成没有处理。
fn full_sha(raw: &str) -> Option<String> {
    if raw.len() != 40 || raw.chars().all(|c| c == '0') {
        return None;
    }
    Some(raw.to_string())
}

/// numstat 的行数差，**按位置对齐**到 raw 的记录上：两边来自同一套 diff 引擎、同一条顺序，
/// 所以第 i 条就是第 i 个文件，不需要拿路径去猜。
///
/// 要接的是两种字段形态（本机 git 2.54 实测）：
/// - `-z`：非 rename/copy 记录是 `增\t删\t路径` 一个 token；**改名记录占三个 token**——
///   `增\t删\t` 后面跟一个 NUL，再是源路径、目标路径各一个。这就是改名那条"路径字段比别的
///   记录多"的来源，只按"每条记录一个 token"数必然对不上（这条曾把带改名的提交打成
///   ParseFailure：文件清单整条读不出来）；
/// - 非 `-z`：每条记录一个 token，改名把两个路径写成 `old => new`。
///
/// 两种都由"token 总数"一次定死，对不上就带原文报错而不是含糊兜底。
/// 一条记录的行数差：两侧各一个数字，`-`（二进制）记 None
type Counts = Vec<(Option<usize>, Option<usize>)>;

fn counts_for(records: &[Raw], stdout: &str) -> Result<Counts, GitError> {
    let tokens = z_tokens(stdout);
    let renamed = records.iter().filter(|record| record.renamed()).count();
    let nul_shape = records.len() + 2 * renamed == tokens.len();
    let inline_shape = records.len() == tokens.len();

    if !nul_shape && !inline_shape {
        return Err(parse_failure(
            &tokens,
            0,
            &format!(
                "numstat 的字段数对不上：{} 条记录（其中 {} 条改名）、{} 个 token",
                records.len(),
                renamed,
                tokens.len()
            ),
        ));
    }

    let mut out = Vec::with_capacity(records.len());
    let mut index = 0;
    for record in records {
        let (added, deleted, tail) = split_counts(&tokens, index)?;
        index += 1;
        if nul_shape {
            // 改名那条的源路径与目标路径各占一个 token
            index += usize::from(record.renamed()) * 2;
        }
        // 非改名的记录顺手把路径对一下：名字都串了说明我们对 git 输出形态的理解错了，宁可报错。
        // 改名的路径两种形态都不一样（`old => new` 或分成两个 token），不在这里对
        if !record.renamed() {
            if let Some(tail) = tail {
                if tail != record.path {
                    return Err(parse_failure(
                        &tokens,
                        index,
                        &format!("numstat 路径与 raw 不一致：{tail} ≠ {}", record.path),
                    ));
                }
            }
        }
        out.push((added, deleted));
    }
    Ok(out)
}

/// `1\t1\tpath` / `-\t-\0`：前两段是行数，第三段（如果有）是路径。
/// 一次拆出：增、删，以及同一个 token 里剩下的第三段（内联形态下的路径）
type Counts3<'a> = (Option<usize>, Option<usize>, Option<&'a str>);

fn split_counts<'a>(tokens: &'a [&'a str], index: usize) -> Result<Counts3<'a>, GitError> {
    let token = *tokens
        .get(index)
        .ok_or_else(|| parse_failure(tokens, index, "numstat 提前结束"))?;
    let mut fields = token.splitn(3, '\t');
    let added = count(fields.next().unwrap_or_default())?;
    let deleted = count(fields.next().unwrap_or_default())?;
    Ok((added, deleted, fields.next()))
}

fn count(raw: &str) -> Result<Option<usize>, GitError> {
    if raw == "-" {
        return Ok(None);
    }
    raw.parse::<usize>()
        .map(Some)
        .map_err(|_| parse_failure(&[raw], 0, &format!("numstat 的行数不是数字：{raw}")))
}

/// `-z` 输出的记录以 NUL 结尾，所以整串末尾多出一个空 token。
fn z_tokens(stdout: &str) -> Vec<&str> {
    let mut tokens: Vec<&str> = stdout.split('\0').collect();
    if tokens.last().is_some_and(|token| token.is_empty()) {
        tokens.pop();
    }
    tokens
}

/// 二进制文件的大小差：一次 `cat-file --batch-check` 把所有要问的 sha 一起喂进去。
/// 取不到就当没有（界面退化成"二进制文件"），不让一个装饰性数字毁掉整份清单。
fn fill_sizes(
    repo: &Path,
    changes: &mut [Change],
    blobs: &[(Option<String>, Option<String>)],
) -> Result<(), GitError> {
    let asked: Vec<&str> = blobs
        .iter()
        .flat_map(|(old, new)| old.iter().chain(new))
        .map(|sha| sha.as_str())
        .collect();
    if asked.is_empty() {
        return Ok(());
    }
    let stdin: String = asked.iter().map(|sha| format!("{sha}\n")).collect();
    let Ok(out) = process::run_bytes(Some(repo), &["cat-file", "--batch-check"], stdin.as_bytes())
    else {
        return Ok(());
    };
    if !out.success {
        return Ok(());
    }

    let sizes = parse_batch_check(&out.stdout);
    // 按格对齐，不靠"问的顺序"猜：两侧都问到，每个文件恰好两个
    for (change, (old, new)) in changes.iter_mut().zip(blobs) {
        change.old_size = old.as_deref().and_then(|sha| sizes.get(sha).copied());
        change.new_size = new.as_deref().and_then(|sha| sizes.get(sha).copied());
    }
    Ok(())
}

/// 一行一个 `<sha> <type> <size>`；不存在的对象是 `<输入> missing`，直接跳过。
fn parse_batch_check(stdout: &[u8]) -> HashMap<String, u64> {
    let mut sizes = HashMap::new();
    for line in String::from_utf8_lossy(stdout).lines() {
        let mut fields = line.split(' ');
        let (Some(sha), Some(typed)) = (fields.next(), fields.next()) else {
            continue;
        };
        if typed == "missing" {
            continue;
        }
        if let Some(size) = fields.next().and_then(|raw| raw.parse::<u64>().ok()) {
            sizes.insert(sha.to_string(), size);
        }
    }
    sizes
}

fn parse_failure(tokens: &[&str], index: usize, why: &str) -> GitError {
    let around: Vec<&str> = tokens.iter().skip(index).take(4).copied().collect();
    GitError::ParseFailure {
        snippet: format!("{why}｜从第 {index} 个字段起：{around:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path as StdPath;

    fn git_in(dir: &StdPath, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} failed: {}", out.stderr);
        out.stdout
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "."]);
        git_in(dir.path(), &["config", "user.name", "测试者"]);
        git_in(dir.path(), &["config", "user.email", "t@example.com"]);
        dir
    }

    fn commit(dir: &StdPath, msg: &str) -> String {
        git_in(dir, &["commit", "-q", "-m", msg]);
        git_in(dir, &["rev-parse", "HEAD"]).trim().to_string()
    }

    fn write(dir: &StdPath, name: &str, body: &str) {
        fs::write(dir.join(name), body).expect("write");
    }

    /// 一条提交同时含新增、修改、删除、改名，还夹一个中文名和带空格的名字
    #[test]
    fn add_modify_delete_and_rename_carry_status_and_counts() {
        let dir = repo();
        write(dir.path(), "keep.txt", "one\ntwo\nthree\n");
        write(dir.path(), "drop.txt", "x\n");
        write(dir.path(), "old name.txt", "alpha\n");
        write(dir.path(), "中文.txt", "第一行\n");
        git_in(dir.path(), &["add", "-A"]);
        commit(dir.path(), "chore: 铺底");

        write(dir.path(), "keep.txt", "one\nTWO CHANGED\nthree\nextra\n");
        fs::remove_file(dir.path().join("drop.txt")).expect("rm");
        fs::rename(
            dir.path().join("old name.txt"),
            dir.path().join("new name.txt"),
        )
        .expect("rename");
        write(dir.path(), "new name.txt", "alpha\nbeta\n");
        write(dir.path(), "中文.txt", "第一行\n第二行\n");
        git_in(dir.path(), &["add", "-A"]);
        let sha = commit(dir.path(), "feat: 一次改四类");

        let detail = read(dir.path(), &sha).expect("detail");
        assert_eq!(detail.parents.len(), 1);
        assert!(!detail.merge);
        let by: HashMap<String, &Change> =
            detail.changes.iter().map(|c| (c.path.clone(), c)).collect();

        let keep = &by["keep.txt"];
        assert!(matches!(keep.status, ChangeStatus::Modify));
        assert_eq!((keep.added, keep.deleted), (Some(2), Some(1)));
        assert!(!keep.binary);

        let drop = &by["drop.txt"];
        assert!(matches!(drop.status, ChangeStatus::Delete));
        assert_eq!(drop.new_mode.as_deref(), Some("000000"));

        let renamed = &by["new name.txt"];
        assert!(matches!(renamed.status, ChangeStatus::Rename));
        assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
        // 改名又改了内容：分数在 1..100 之间，100 才是纯改名
        let score = renamed.score.expect("rename 要带相似度");
        assert!((1..=100).contains(&score), "相似度越界：{score}");

        let chinese = &by["中文.txt"];
        assert_eq!(
            (chinese.added, chinese.deleted),
            (Some(1), Some(0)),
            "中文路径也要能对上号：{chinese:?}"
        );
    }

    /// 只改权限不改内容：raw 给 M、numstat 给 0/0，两个 blob 相同
    #[test]
    fn a_permission_only_change_is_flagged_and_has_no_line_counts() {
        let dir = repo();
        write(dir.path(), "run.sh", "echo hi\n");
        git_in(dir.path(), &["add", "-A"]);
        commit(dir.path(), "chore: 铺底");

        git_in(dir.path(), &["update-index", "--chmod=+x", "run.sh"]);
        let sha = commit(dir.path(), "chore: 加可执行位");

        let detail = read(dir.path(), &sha).expect("detail");
        let change = &detail.changes[0];
        assert_eq!(change.path, "run.sh");
        assert!(matches!(change.status, ChangeStatus::Modify));
        assert!(change.mode_only, "只改权限必须标出来：{:?}", change);
        assert_eq!((change.added, change.deleted), (Some(0), Some(0)));
        assert_eq!(change.old_mode.as_deref(), Some("100644"));
        assert_eq!(change.new_mode.as_deref(), Some("100755"));
    }

    /// 二进制：numstat 给 `-\t-`，没有行数差，只能给大小差
    #[test]
    fn binary_files_report_sizes_instead_of_lines() {
        let dir = repo();
        fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 3, 0, 255]).expect("write");
        git_in(dir.path(), &["add", "-A"]);
        commit(dir.path(), "chore: 放一个二进制");

        fs::write(dir.path().join("blob.bin"), [0u8; 9]).expect("write");
        git_in(dir.path(), &["add", "-A"]);
        let sha = commit(dir.path(), "fix: 换一坨");

        let detail = read(dir.path(), &sha).expect("detail");
        let change = &detail.changes[0];
        assert!(change.binary, "含 NUL 的内容 git 必判二进制：{:?}", change);
        assert_eq!((change.added, change.deleted), (None, None));
        assert_eq!(change.old_size, Some(6), "上一版 6 字节");
        assert_eq!(change.new_size, Some(9), "这一版 9 字节");
    }

    /// 子模块指针：绕开 submodule 的机器，直接往索引里塞一条 gitlink
    #[test]
    fn a_submodule_pointer_shows_both_commits() {
        let other = repo();
        write(other.path(), "a.txt", "one\n");
        git_in(other.path(), &["add", "-A"]);
        commit(other.path(), "chore: 子仓库一条");
        let first = git_in(other.path(), &["rev-parse", "HEAD"])
            .trim()
            .to_string();

        let dir = repo();
        write(dir.path(), "keep.txt", "one\n");
        git_in(dir.path(), &["add", "-A"]);
        commit(dir.path(), "chore: 铺底");

        // 先让 vendor 作为一个 gitlink 落进历史，下一跳才有"两侧都有值"的比较
        git_in(
            dir.path(),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{first},vendor"),
            ],
        );
        commit(dir.path(), "chore: 挂上子模块");

        write(other.path(), "a.txt", "one\ntwo\n");
        git_in(other.path(), &["add", "-A"]);
        let moved = commit(other.path(), "chore: 子仓库第二条");

        git_in(
            dir.path(),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{moved},vendor"),
            ],
        );
        let sha = commit(dir.path(), "chore: 抬一下子模块指针");

        let detail = read(dir.path(), &sha).expect("detail");
        let change = &detail.changes[0];
        assert_eq!(change.path, "vendor");
        assert!(change.gitlink, "160000 要认成子模块指针：{:?}", change);
        assert_eq!(change.old_mode.as_deref(), Some(GITLINK_MODE));
        assert_eq!(change.new_mode.as_deref(), Some(GITLINK_MODE));
        // 界面要摆出"从哪个子模块提交抬到哪个"，所以两个提交号都得是全 40 位
        assert_eq!(
            change.old_oid.as_deref(),
            Some(first.as_str()),
            "{:?}",
            change
        );
        assert_eq!(change.new_oid.as_deref(), Some(moved.as_str()));
        // 子模块那个提交对象不在本仓库里，大小查不到是预期的，界面退化成"指针变更"
        assert_eq!(change.old_size, None);
        assert_eq!(change.new_size, None);
    }

    /// 合并提交：`show` 本来什么都不输出，必须显式对第一父做差，界面再明确标注
    #[test]
    fn a_merge_lists_files_against_the_first_parent() {
        let dir = repo();
        write(dir.path(), "base.txt", "one\n");
        git_in(dir.path(), &["add", "-A"]);
        commit(dir.path(), "chore: 基线");
        // 主干的分支名由 init.defaultBranch 决定（master / main 都常见），一律现取
        let trunk = git_in(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"])
            .trim()
            .to_string();

        git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
        write(dir.path(), "side.txt", "s\n");
        git_in(dir.path(), &["add", "-A"]);
        commit(dir.path(), "feat: 支线一条");

        git_in(dir.path(), &["checkout", "-q", &trunk]);
        write(dir.path(), "main.txt", "m\n");
        git_in(dir.path(), &["add", "-A"]);
        commit(dir.path(), "feat: 主干一条");

        git_in(
            dir.path(),
            &["merge", "--no-ff", "-q", "-m", "chore: 合并支线", "side"],
        );
        let sha = git_in(dir.path(), &["rev-parse", "HEAD"])
            .trim()
            .to_string();

        let detail = read(dir.path(), &sha).expect("合并提交的 detail");
        assert_eq!(detail.parents.len(), 2);
        assert!(detail.merge);
        // 对第一父（主干那条）的差集只有支线带进来的文件
        let paths: Vec<&str> = detail.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["side.txt"],
            "合并的清单该只对第一父：{detail:?}"
        );
        assert!(matches!(detail.changes[0].status, ChangeStatus::Add));
        assert_eq!(detail.changes[0].added, Some(1));
    }

    #[test]
    fn an_unknown_raw_status_is_a_parse_failure_not_a_default() {
        let err = match_status("Q100").expect_err("未知状态必须报错");
        assert!(format!("{err:?}").contains("Q100"));
    }

    /// 两种 numstat 字段形态都要接住，且对不上时必须带着 token 数报错
    #[test]
    fn counts_align_in_both_nul_field_shapes() {
        let records = vec![
            Raw {
                status: ChangeStatus::Modify,
                score: None,
                old_mode: "100644".into(),
                new_mode: "100644".into(),
                old_blob: Some("a".repeat(40)),
                new_blob: Some("b".repeat(40)),
                old_path: None,
                path: "keep.txt".into(),
            },
            Raw {
                status: ChangeStatus::Rename,
                score: Some(100),
                old_mode: "100644".into(),
                new_mode: "100644".into(),
                old_blob: Some("b".repeat(40)),
                new_blob: Some("c".repeat(40)),
                old_path: Some("old.txt".into()),
                path: "new.txt".into(),
            },
        ];

        // 形态一：`-z`（实测）。非改名记录是一个 token，改名那条占三个：计数、源、目标
        let nul = "2\t1\tkeep.txt\x000\t0\t\x00old.txt\x00new.txt\x00";
        let got = counts_for(&records, nul).expect("-z 的形态");
        assert_eq!(got, vec![(Some(2), Some(1)), (Some(0), Some(0))]);

        // 形态二：路径挂在同一个 token 的第三段
        let inline = "2\t1\tkeep.txt\x000\t0\told.txt => new.txt\x00";
        let got = counts_for(&records, inline).expect("内联的形态");
        assert_eq!(got, vec![(Some(2), Some(1)), (Some(0), Some(0))]);

        // 对不上：token 数既不匹配 `-z` 形态也不匹配内联形态
        let err = counts_for(
            &records,
            "2\t1\tkeep.txt\x000\t0\t\x00old.txt\x00new.txt\x00extra\x00",
        )
        .expect_err("该报错");
        let text = format!("{err:?}");
        assert!(text.contains("字段数对不上"), "报错要说清为什么：{text}");
    }
}
