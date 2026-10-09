use std::path::Path;

use serde::Serialize;

use super::process;
use crate::error::GitError;

/// 单个文件的正文，一次最多回这么多字节。超出只给前缀并标 `truncated`。
const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// 跨修订文件树的一个条目（§7.6）。
///
/// 刻意返回**扁平列表**而不是嵌套结构：树上要做的是筛、缩进、跳，
/// 而嵌套结构在 Rust 侧要额外处理中间层目录是空的情况，前端还得再拆一次。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    /// 树里的 mode（如 `100644`）。可执行位变了用户要能看出来，所以留着
    pub mode: String,
    /// blob / commit（子模块指针）。tree 不会出现——用了 `-r` 就已经展开了
    pub kind: EntryKind,
    pub oid: String,
    /// blob 字节数。子模块指针与符号链接在这一版不给（`-l` 只对 blob 报大小）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Blob,
    /// 子模块指针。那一端的提交号在本仓库里查不到，所以界面上不给它开 blame
    Commit,
}

/// 某个修订里的文件正文（§7.6「树上可跳该修订的 blob 内容」）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub path: String,
    /// 正文。二进制、超阈值时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub size: u64,
    /// 内容不是 UTF-8 文本（也可能是二进制）：界面就只给大小，不给正文
    pub binary: bool,
    /// 文本被字节闸门截断过
    pub truncated: bool,
}

/// 一个修订的文件树。
///
/// `ls-tree -r -l -z` 的记录形态（本机 git 2.54 实测，很久没变过）：
/// `<mode> SP <type> SP <object> SP <size|-> TAB <path>`，一条一个 NUL。
/// 刻意用这一种传统形态而不是 `--format=`：后者是 git 2.36 才有的，
/// 换来的好处只是少切一次空格，而少一次切分不值得把下限抬上去。
///
/// **头字段按空白切**（`-l` 会把大小右对齐补空格，实测 `blob <oid>      16`），
/// 但路径那一段按第一个 TAB 原样取：git 的路径里允许有空格和换行，用空白切路径会把它切碎。
pub fn list(repo: &Path, rev: &str) -> Result<Vec<Entry>, GitError> {
    let stdout = process::run(Some(repo), &["ls-tree", "-r", "-l", "-z", rev])?.expect_success()?;
    parse(&stdout)
}

fn parse(stdout: &str) -> Result<Vec<Entry>, GitError> {
    let mut entries = Vec::new();
    for record in stdout.split('\0').filter(|record| !record.is_empty()) {
        let Some((head, path)) = record.split_once('\t') else {
            return Err(failure(record));
        };
        let fields: Vec<&str> = head.split_whitespace().collect();
        if fields.len() != 4 {
            return Err(failure(record));
        }
        let (mode, kind, oid, size) = (fields[0], fields[1], fields[2], fields[3]);
        if mode.is_empty() || oid.is_empty() || path.is_empty() {
            return Err(failure(record));
        }
        entries.push(Entry {
            path: path.to_string(),
            mode: mode.to_string(),
            kind: match kind {
                "commit" => EntryKind::Commit,
                // 符号链接在 ls-tree 里也是 blob；正文当文本看就是那串目标路径
                _ => EntryKind::Blob,
            },
            oid: oid.to_string(),
            size: size.parse::<u64>().ok(),
        });
    }

    // 同一目录里的兄弟排在一起，界面上按这个顺序画才像棵树
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// 读某个修订里一个文件的正文。子模块指针没有正文，界面也不该给这个入口。
pub fn content(repo: &Path, rev: &str, path: &str) -> Result<Content, GitError> {
    let object = format!("{rev}:{path}");
    let size = object_size(repo, &object)?;
    if size > MAX_TEXT_BYTES as u64 {
        // 超阈值也回前缀：界面上直接给前几百 KB 比什么都不给有用，
        // 但必须说清截断了，否则用户会以为文件就这么多
        let head = process::run_bytes(Some(repo), &["cat-file", "blob", &object], &[])?
            .expect_success()?;
        return Ok(Content {
            path: path.to_string(),
            text: Some(loose_text(&head)),
            size,
            binary: false,
            truncated: true,
        });
    }

    let bytes =
        process::run_bytes(Some(repo), &["cat-file", "blob", &object], &[])?.expect_success()?;
    // 文本判定看前 8 KB：完整扫一遍几 MB 的文件只为了找 NUL 不值当，
    // 而没有 NUL 的二进制（PSD 之类）本来就超出这个视图的能力范围
    let window = &bytes[..bytes.len().min(8192)];
    let binary = window.contains(&0);

    Ok(Content {
        path: path.to_string(),
        text: (!binary).then(|| loose_text(&bytes)),
        size,
        binary,
        truncated: false,
    })
}

/// 二进制判定与正文都用同一套宽松解码：坏字节换成 U+FFFD，
/// 界面不会因为一个坏字节整页报错。
fn loose_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn object_size(repo: &Path, object: &str) -> Result<u64, GitError> {
    let stdout = process::run(Some(repo), &["cat-file", "-s", object])?.expect_success()?;
    stdout
        .trim()
        .parse::<u64>()
        .map_err(|_| GitError::ParseFailure {
            snippet: process::snippet(&stdout),
        })
}

fn failure(record: &str) -> GitError {
    GitError::ParseFailure {
        snippet: process::snippet(record),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // 关掉 autocrlf：本机全局开着它，不关的话写进去的 CRLF 会在入库时被折成 LF，
        // 那些专门验 CRLF 的用例就变成验别的机器的设定了
        git_in(dir.path(), &["config", "core.autocrlf", "false"]);
        dir
    }

    #[test]
    fn a_tree_lists_files_with_mode_and_size() {
        let dir = repo();
        let repo_path = dir.path();
        std::fs::create_dir_all(repo_path.join("src")).expect("mkdir");
        std::fs::write(repo_path.join("src/主程序.rs"), "fn main() {}\n").expect("write");
        std::fs::write(repo_path.join("README.md"), "中文也要在\n").expect("write");
        git_in(repo_path, &["add", "-A"]);
        git_in(repo_path, &["commit", "-q", "-m", "feat: 初次"]);

        let entries = list(repo_path, "HEAD").expect("读文件树");
        let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["README.md", "src/主程序.rs"],
            "扁平列表按路径排序，中文路径原样保留"
        );
        assert_eq!(entries[1].kind, EntryKind::Blob);
        assert_eq!(entries[1].mode, "100644");
        assert_eq!(entries[1].size, Some(13), "大小按字节算：三个汉字加换行");
        assert_eq!(entries[0].oid.len(), 40);
    }

    #[test]
    fn a_submodule_entry_is_marked_and_has_no_size() {
        let dir = repo();
        let repo_path = dir.path();
        std::fs::write(repo_path.join("a.txt"), "1\n").expect("write");
        git_in(repo_path, &["add", "-A"]);
        git_in(repo_path, &["commit", "-q", "-m", "feat: 基线"]);

        // 手搓一个 gitlink：submodule add 要联网，这里直接建对象再 update-index
        let child = repo_path.join("child");
        std::fs::create_dir_all(&child).expect("mkdir");
        git_in(&child, &["init", "-q", "-b", "main", "."]);
        git_in(&child, &["config", "user.name", "测试者"]);
        git_in(&child, &["config", "user.email", "t@example.com"]);
        std::fs::write(child.join("b.txt"), "2\n").expect("write");
        git_in(&child, &["add", "-A"]);
        git_in(&child, &["commit", "-q", "-m", "feat: 子仓库"]);
        let child_head = git_in(&child, &["rev-parse", "HEAD"]);
        git_in(
            repo_path,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000",
                &child_head,
                "sub",
            ],
        );
        git_in(repo_path, &["commit", "-q", "-m", "feat: 加子模块"]);

        let entries = list(repo_path, "HEAD").expect("读文件树");
        let sub = entries
            .iter()
            .find(|entry| entry.path == "sub")
            .expect("子模块指针要在树里");
        assert_eq!(sub.kind, EntryKind::Commit);
        assert_eq!(sub.mode, "160000");
        assert_eq!(sub.size, None, "gitlink 没有 blob 大小");
    }

    #[test]
    fn content_returns_text_and_marks_binaries() {
        let dir = repo();
        let repo_path = dir.path();
        std::fs::write(repo_path.join("a.txt"), "第一行\r\n第二行\n").expect("write");
        std::fs::write(repo_path.join("b.bin"), [0u8, 1, 2, 3, 0]).expect("write");
        git_in(repo_path, &["add", "-A"]);
        git_in(repo_path, &["commit", "-q", "-m", "feat: 两种内容"]);

        let text = content(repo_path, "HEAD", "a.txt").expect("读正文");
        assert!(!text.binary);
        assert_eq!(
            text.text.as_deref(),
            Some("第一行\r\n第二行\n"),
            "CRLF 原样"
        );
        assert!(!text.truncated);
        assert_eq!(text.size, 21, "三个汉字一行 + CRLF + 三个汉字 + 换行");

        let binary = content(repo_path, "HEAD", "b.bin").expect("读正文");
        assert!(binary.binary);
        assert_eq!(binary.text, None, "二进制不给正文");
    }

    #[test]
    fn a_huge_file_is_returned_as_a_truncated_prefix() {
        let dir = repo();
        let repo_path = dir.path();
        let many: String = "行\n".repeat(MAX_TEXT_BYTES / 3 + 100);
        std::fs::write(repo_path.join("big.txt"), &many).expect("write");
        git_in(repo_path, &["add", "-A"]);
        git_in(repo_path, &["commit", "-q", "-m", "feat: 大文件"]);

        let got = content(repo_path, "HEAD", "big.txt").expect("读正文");
        assert!(got.truncated, "超阈值要截断并说出来");
        assert!(got.size > MAX_TEXT_BYTES as u64);
        assert!(got.text.expect("有前缀").starts_with("行\n"));
    }

    #[test]
    fn a_malformed_record_is_reported_as_parse_failure() {
        let err = parse("这不是 ls-tree 的记录").expect_err("坏记录必须报错");
        assert!(matches!(err, GitError::ParseFailure { .. }), "{err:?}");

        let err = parse("100644 blob abc -\0").expect_err("少一段字段必须报错");
        assert!(matches!(err, GitError::ParseFailure { .. }), "{err:?}");

        let err = parse("100644 blob abc - 没有制表符\0").expect_err("没有 TAB 分隔必须报错");
        assert!(matches!(err, GitError::ParseFailure { .. }), "{err:?}");
    }

    #[test]
    fn a_path_with_spaces_stays_one_entry() {
        // 路径那一段按第一个 TAB 原样取，用空白切会把它切碎
        let entry = parse("100644 blob abc123 6\t有 空格 的文件.txt\0")
            .expect("该解析成功")
            .remove(0);
        assert_eq!(entry.path, "有 空格 的文件.txt");
        assert_eq!(entry.size, Some(6));
    }
}
