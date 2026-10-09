use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// 超大文件只 blame 开头这么多行（§7.6）。
///
/// blame 的成本跟着行数走：10 万行的文件一次要回几 MB 文本、还要为每一行算出归属，
/// 而人翻这种文件时看的永远是开头。降到"只 blame 可见区"，并在界面上说清被截断了。
const MAX_LINES: usize = 2000;
/// 判"超大"用的字节闸门。500KB 大约是一万行上下，取它是为了不用先把整个 blob 读出来数行。
const MAX_BYTES: u64 = 500 * 1024;

/// blame 的一行归属。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlameLine {
    /// 文件里的行号，从 1 起
    pub line: usize,
    pub sha: String,
    pub author: String,
    /// 作者时间，Unix 秒
    pub time: i64,
    /// 该行原文（去掉行尾换行，CR 保留——CRLF 文件里回车就是内容的一部分）
    pub text: String,
    /// 边界提交：这一行的归属早于我们 blame 的那个修订（通常是仓库导入前的历史）。
    /// 这类行不该拿去跳提交详情，它们的 sha 在本仓库里可能根本不存在。
    pub boundary: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Blame {
    pub path: String,
    pub lines: Vec<BlameLine>,
    /// 文件被闸门截断过：只 blame 了开头 MAX_LINES 行
    pub truncated: bool,
    /// 这次 blame 用了哪些忽略修订（`.blame-ignore-revs` + 界面勾选的），
    /// 界面上要列出来：忽略之后行的归属会变，用户得知道这张表不是原始归属
    pub ignored: Vec<String>,
}

/// 逐行归属（§7.6）。
///
/// 走 `git blame --porcelain`：这是 git 唯一给出结构化归属的输出格式，
/// `^` 前缀、边界提交、跨文件搬移都在 header 里显式带着，不用从人读的格式里猜。
///
/// `ignore_revs` 由界面传入（用户勾选的修订），`.blame-ignore-revs` 自动生效——
/// 两者都走 git 自己的忽略机制，所以"忽略之后"的重算逻辑不用我们再实现一遍。
pub fn read(repo: &Path, rev: &str, path: &str, ignore_revs: &[String]) -> Result<Blame, GitError> {
    let object = format!("{rev}:{path}");

    let size = object_size(repo, &object)?;
    let truncated = size > MAX_BYTES;
    let window = format!("1,{MAX_LINES}");

    let mut args = vec![
        "blame".to_string(),
        "--porcelain".to_string(),
        "--root".to_string(),
        rev.to_string(),
    ];
    if truncated {
        // 只 blame 开头一段。--root 与 -L 同存时 git 给的是根提交那一段，
        // 归属仍由 git 自己算，我们不碰
        args.push("-L".to_string());
        args.push(window);
    }
    for ignored in ignore_revs {
        args.push("--ignore-rev".to_string());
        args.push(ignored.clone());
    }
    // `.blame-ignore-revs` 是仓库自己维护的清单，有就用，没有就当没有
    if let Some(file) = ignore_file(repo) {
        args.push("--ignore-revs-file".to_string());
        args.push(file);
    }
    args.push("--".to_string());
    // blame 不接 pathspec 魔法（实测 `:(literal)` 会被当成文件名的一部分），
    // 而它本来就把这个参数当**字面文件名**匹配，所以原样传下去正好
    args.push(path.to_string());
    let args = process::strs(&args);

    let stdout = process::run(Some(repo), &args)?.expect_success()?;
    let lines = parse(&stdout)?;

    Ok(Blame {
        path: path.to_string(),
        lines,
        truncated,
        ignored: ignored_list(repo, rev, ignore_revs),
    })
}

const IGNORE_FILE: &str = ".blame-ignore-revs";

/// 清单文件的正文。工作区优先：刚提交到 HEAD 的那份就在工作区，
/// 而本地新建的那份还没进 HEAD——两者用户都希望它生效。
fn ignore_content(repo: &Path, rev: &str) -> String {
    for name in IGNORE_FILES {
        if let Ok(text) = std::fs::read_to_string(repo.join(name)) {
            return text;
        }
    }
    process::run(Some(repo), &["show", &format!("{rev}:{IGNORE_FILE}")])
        .ok()
        .filter(|out| out.success)
        .map(|out| out.stdout)
        .unwrap_or_default()
}

/// `.blame-ignore-revs` 在这个仓库里存在吗？不存在就不传 `--ignore-revs-file`——
/// 传一个 git 读不到的文件路径是硬错误，而"没有这个文件"是完全正常的仓库状态。
///
/// 找三个位置，顺序即优先级：`blame.ignoreRevsFile` 配置（git 自己的约定）、
/// 仓库根的 `.git-blame-ignore-revs`（git 的默认名）、GitHub 生态常用的
/// `.blame-ignore-revs`。只认工作区那一侧的路径：git 读的是磁盘。
fn ignore_file(repo: &Path) -> Option<String> {
    if let Some(configured) = process::run(Some(repo), &["config", "--get", "blame.ignoreRevsFile"])
        .ok()
        .filter(|out| out.success)
    {
        let configured = configured.stdout.trim();
        if !configured.is_empty() && repo.join(configured).is_file() {
            return Some(configured.to_string());
        }
    }
    IGNORE_FILES
        .iter()
        .find(|name| repo.join(name).is_file())
        .map(|name| (*name).to_string())
}

/// git 自己会用到的两个文件名，以及 GitHub 生态那个。第一个是 git 的默认名。
const IGNORE_FILES: [&str; 2] = [".git-blame-ignore-revs", ".blame-ignore-revs"];

/// 界面上要显示的"这次忽略了谁"：配置文件里的一行一个 sha，加上界面勾的。
fn ignored_list(repo: &Path, rev: &str, extra: &[String]) -> Vec<String> {
    let mut ignored: Vec<String> = extra.to_vec();
    for line in ignore_content(repo, rev).lines() {
        // 一行一个 sha，允许 `#` 注释和空行；非十六进制的行（除注释）一律不认，
        // 免得把一段说明文字当成修订号传给 git
        let token = line.split('#').next().unwrap_or("").trim();
        if token.len() >= 7 && token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let full = token.to_string();
            if !ignored.contains(&full) {
                ignored.push(full);
            }
        }
    }
    ignored
}

/// blob 字节数。取不到（路径在这一修订里不存在、或是目录）就让 git 自己报错去。
fn object_size(repo: &Path, object: &str) -> Result<u64, GitError> {
    let stdout = process::run(Some(repo), &["cat-file", "-s", object])?.expect_success()?;
    stdout
        .trim()
        .parse::<u64>()
        .map_err(|_| GitError::ParseFailure {
            snippet: process::snippet(&stdout),
        })
}

const BOUNDARY_SHA: &str = "0000000000000000000000000000000000000000";

/// `--porcelain` 的结构（本机 git 2.54 实测）：
///
/// ```text
/// <sha> <原行号> <最终行号> <本组行数>
/// author 张三            ┐
/// author-mail <z@e.com>  │ 组头之后的元信息，逐行 key value
/// author-time 1700000000 │
/// boundary               ┘（只在这一组首次出现）
/// previous <sha> <路径>
/// filename a.txt
/// \t这一行的原文        ← 以 TAB 开头的行才是内容行，之前的一律当元信息读
/// \t……
/// ```
///
/// 一次提交可以占一组多行，所以组头行数一到就要把该组补齐；组里的后续行不再重复元信息。
/// 少了组头的行数就永远补不齐，所以组头不合法直接带原文报 `ParseFailure`。
fn parse(stdout: &str) -> Result<Vec<BlameLine>, GitError> {
    let mut lines = Vec::new();
    let mut group: Option<Group> = None;

    for line in stdout.split('\n') {
        // 内容行先判：CRLF 文件里行尾那个 \r 属于内容（CRLF 改动只有看得见才叫改过），
        // 所以它不能被当行尾噪声剥掉；组头与元信息行不带 \r，剥掉只是为了容错
        if let Some(text) = line.strip_prefix('\t') {
            // 内容行：靠组头的行数决定一组有几条，缺了组头就永远补不齐，所以直接报错
            let Some(open) = group.as_mut() else {
                return Err(failure(line));
            };
            lines.push(BlameLine {
                line: open.next_line,
                sha: open.sha.clone(),
                author: open.author.clone(),
                time: open.time,
                text: text.to_string(),
                boundary: open.boundary,
            });
            open.next_line += 1;
            open.remaining -= 1;
            if open.remaining == 0 {
                group = None;
            }
            continue;
        }

        let raw = line.strip_suffix('\r').unwrap_or(line);
        if raw.is_empty() {
            continue;
        }

        if let Some(head) = group_head(raw) {
            // 上一组没走完就来了新组头：porcelain 不会这么写，出现即数据不符预期
            if group.is_some() {
                return Err(failure(raw));
            }
            group = Some(Group {
                boundary: head.sha == BOUNDARY_SHA,
                sha: head.sha,
                next_line: head.final_line,
                remaining: head.count,
                ..Group::default()
            });
            continue;
        }

        // 元信息行：只认这三个 key，其余（previous / filename / committer-*）对界面没用
        let Some(open) = group.as_mut() else {
            return Err(failure(raw));
        };
        let Some((key, value)) = raw.split_once(' ') else {
            return Err(failure(raw));
        };
        match key {
            "author" => open.author = value.to_string(),
            "author-time" => open.time = value.parse::<i64>().unwrap_or(0),
            "boundary" => open.boundary = true,
            _ => {}
        }
    }

    Ok(lines)
}

/// 一组头。本组往下的行数为一，组内后续行不带元信息。
#[derive(Default)]
struct Group {
    sha: String,
    author: String,
    time: i64,
    boundary: bool,
    /// 组内下一条要落到的行号
    next_line: usize,
    /// 组内还剩几行没读
    remaining: usize,
}

struct GroupHead {
    sha: String,
    final_line: usize,
    count: usize,
}

/// 认组头：`<40位sha> <原行号> <最终行号> <组内行数>`，四段，不多不少。
/// 原行号读不到就退化成 0——它只对 `git blame --incremental` 的消费方有用。
fn group_head(raw: &str) -> Option<GroupHead> {
    let mut fields = raw.split(' ');
    let sha = fields.next()?;
    let _original = fields.next()?;
    let final_line = fields.next()?.parse::<usize>().ok()?;
    let count = fields.next()?.parse::<usize>().ok()?;
    if fields.next().is_some() {
        return None;
    }
    if !is_sha(sha) || count == 0 {
        return None;
    }
    Some(GroupHead {
        sha: sha.to_string(),
        final_line,
        count,
    })
}

fn is_sha(token: &str) -> bool {
    token.len() == 40 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn failure(raw: &str) -> GitError {
    GitError::ParseFailure {
        snippet: process::snippet(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        out.stdout.trim_end().to_string()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        git_in(dir.path(), &["config", "user.name", "张三"]);
        git_in(dir.path(), &["config", "user.email", "z@example.com"]);
        // 关掉 autocrlf：本机全局开着它，不关的话写进去的 CRLF 会在入库时被折成 LF，
        // 那个专门验 CRLF 的用例就变成在验别的机器的设定
        git_in(dir.path(), &["config", "core.autocrlf", "false"]);
        dir
    }

    fn commit(dir: &Path, file: &str, content: &str, msg: &str) -> String {
        fs::write(dir.join(file), content).expect("write");
        git_in(dir, &["add", file]);
        git_in(dir, &["commit", "-q", "-m", msg]);
        git_in(dir, &["rev-parse", "HEAD"])
    }

    /// 前 20 行必须与终端 `git blame -L 1,20` 逐行同 sha（§7.6 验收）
    #[test]
    fn the_first_twenty_lines_match_git_blame() {
        let dir = repo();
        let repo_path = dir.path();
        commit(repo_path, "a.txt", "第一行\n第二行\n", "feat: 初次");
        fs::write(repo_path.join("a.txt"), "第一行\n第二行改了\n").expect("write");
        git_in(repo_path, &["commit", "-qam", "fix: 改第二行"]);

        let got = read(repo_path, "HEAD", "a.txt", &[]).expect("blame ok");
        assert_eq!(got.lines.len(), 2);
        assert!(!got.truncated);
        assert_eq!(got.lines[0].text, "第一行");
        assert_eq!(got.lines[1].text, "第二行改了");
        assert_eq!(got.lines[0].author, "张三");
        assert!(got.lines.iter().all(|line| !line.boundary));
        assert!(got.ignored.is_empty());

        // 与 git 自己那份对一遍：逐行 sha 相同
        let raw = git_in(
            repo_path,
            &["blame", "--porcelain", "-L", "1,2", "HEAD", "--", "a.txt"],
        );
        let shas = porcelain_shas(&raw);
        assert_eq!(
            got.lines.iter().map(|l| l.sha.clone()).collect::<Vec<_>>(),
            shas,
            "我们解析出来的归属要逐行等于 git blame 的输出"
        );
    }

    fn porcelain_shas(raw: &str) -> Vec<String> {
        raw.lines()
            .filter_map(|line| {
                let head = line.split(' ').next()?;
                is_sha(head).then(|| head.to_string())
            })
            .collect()
    }

    /// 一条提交改了多行：porcelain 只给一组组头，组内后续行靠组头数补齐
    #[test]
    fn a_group_of_several_lines_is_expanded() {
        let dir = repo();
        let repo_path = dir.path();
        commit(repo_path, "a.txt", "1\n2\n3\n", "feat: 初次");
        fs::write(repo_path.join("a.txt"), "1\n改了\n改了\n改了\n").expect("write");
        git_in(repo_path, &["commit", "-qam", "feat: 三行一起改"]);

        let got = read(repo_path, "HEAD", "a.txt", &[]).expect("blame ok");
        assert_eq!(got.lines.len(), 4);
        assert_eq!(got.lines[0].text, "1");
        assert_eq!(
            got.lines[1..]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["改了", "改了", "改了"]
        );
        assert!(
            got.lines[1..]
                .iter()
                .all(|line| line.sha == got.lines[1].sha),
            "同一次提交改的连续几行 sha 相同"
        );
        assert_eq!(
            got.lines.iter().map(|line| line.line).collect::<Vec<_>>(),
            vec![1, 2, 3, 4],
            "行号必须顺着往下走"
        );
    }

    /// 忽略修订之后，那几行的归属要退到上一次动它们的提交上。
    ///
    /// 用例挑的是"只改空白"的那种提交（实测 git 2.54）：这类改动被忽略之后，
    /// 行内容在父提交里逐字相同，git 能干净地退回去。反过来，一次提交**引入的新内容**
    /// 忽略不掉——父提交里根本没有这一行，git 只能仍然记在被忽略的那次提交上。
    #[test]
    fn ignored_revisions_move_the_ownership_back() {
        let dir = repo();
        let repo_path = dir.path();
        let base = commit(repo_path, "a.txt", "one\ntwo\n", "feat: 初次");
        fs::write(repo_path.join("a.txt"), "one \ntwo\n").expect("write");
        git_in(repo_path, &["commit", "-qam", "style: 加个空格"]);
        let style = git_in(repo_path, &["rev-parse", "HEAD"]);
        fs::write(repo_path.join("a.txt"), "one \ntwo\nthree\n").expect("write");
        git_in(repo_path, &["commit", "-qam", "feat: 补第三行"]);

        let plain = read(repo_path, "HEAD", "a.txt", &[]).expect("blame");
        assert_eq!(plain.lines[0].sha, style, "不改内容时这一行记在改空白那次");

        let ignored =
            read(repo_path, "HEAD", "a.txt", std::slice::from_ref(&style)).expect("blame");
        assert_eq!(ignored.lines[0].sha, base, "忽略掉那次，归属该退回上一次");
        assert_eq!(ignored.ignored, vec![style]);
    }

    /// `.blame-ignore-revs` 由仓库自己维护：自动生效，界面上要列出来
    #[test]
    fn a_blame_ignore_revs_file_is_picked_up_automatically() {
        let dir = repo();
        let repo_path = dir.path();
        let base = commit(repo_path, "a.txt", "one\ntwo\n", "feat: 初次");
        fs::write(repo_path.join("a.txt"), "one \ntwo\n").expect("write");
        git_in(repo_path, &["commit", "-qam", "style: 加个空格"]);
        let style = git_in(repo_path, &["rev-parse", "HEAD"]);
        fs::write(repo_path.join("a.txt"), "one \ntwo\nthree\n").expect("write");
        git_in(repo_path, &["commit", "-qam", "feat: 补第三行"]);
        fs::write(
            repo_path.join(".blame-ignore-revs"),
            format!("# 只是改格式\n{style}\n"),
        )
        .expect("write");
        git_in(repo_path, &["add", "-A"]);
        git_in(repo_path, &["commit", "-q", "-m", "chore: 加忽略清单"]);

        let got = read(repo_path, "HEAD", "a.txt", &[]).expect("blame");
        assert_eq!(got.lines[0].sha, base);
        assert!(
            got.ignored.contains(&style),
            "界面上要能告诉用户忽略了哪些修订：{:?}",
            got.ignored
        );
    }

    /// 超大文件降级为"只 blame 开头"，且这个降级要能被界面说出口
    #[test]
    fn a_huge_file_is_truncated_to_the_visible_window() {
        let dir = repo();
        let repo_path = dir.path();
        // 字节闸门在 500KB，所以行得写得够长才能越过去
        let filler = "x".repeat(400);
        let many: String = (1..=(MAX_LINES + 500))
            .map(|i| format!("行{i}{filler}\n"))
            .collect();
        commit(repo_path, "big.txt", &many, "feat: 大文件");

        let got = read(repo_path, "HEAD", "big.txt", &[]).expect("blame");
        assert!(got.truncated, "超过字节闸门就该降级");
        assert_eq!(got.lines.len(), MAX_LINES);
    }

    /// CRLF 文件里的回车是内容的一部分，不能在解析时吃掉
    #[test]
    fn carriage_returns_survive() {
        let dir = repo();
        let repo_path = dir.path();
        commit(repo_path, "a.txt", "第一行\r\n", "feat: crlf");

        let got = read(repo_path, "HEAD", "a.txt", &[]).expect("blame");
        assert_eq!(
            got.lines[0].text, "第一行\r",
            "行尾的 \\r 属于内容：CRLF 改动只有看得见才叫改过"
        );
    }

    #[test]
    fn parse_rejects_a_group_without_a_usable_header() {
        let err = parse("这不是组头也不是元信息\n").expect_err("坏行必须报错");
        assert!(matches!(err, GitError::ParseFailure { .. }), "{err:?}");
    }

    #[test]
    fn parse_rejects_a_content_line_before_any_group() {
        let err = parse("\t孤零零的内容\n").expect_err("没有组头的内容行必须报错");
        assert!(matches!(err, GitError::ParseFailure { .. }), "{err:?}");
    }
}
