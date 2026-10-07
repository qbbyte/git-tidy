use std::path::Path;

use serde::Serialize;

use super::detail;
use super::process;
use crate::error::GitError;

/// 单个文件一次最多给这么多行原文。超过就只回统计（§7.5 的降级），
/// 初值按文档来，实测后再定终值。
pub const MAX_DIFF_LINES: usize = 2000;
/// 原始字节的上限。行数少但单行极长的压缩文件同样会撑爆 IPC，两个闸都得有。
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
/// 图片一边最大这么大才内联成 data URL，再大就只报"二进制"。
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const CONTEXT_LINES: &str = "--unified=3";

/// 界面该画哪一种。降级路径全在这里枚举，前端不再自己判长度。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Render {
    /// 有 hunk，正常渲染
    Text,
    /// 没有差异（含"忽略空白后就没了"）
    Empty,
    Binary,
    Image,
    /// 超过阈值：只给统计，原文不回传
    TooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LineKind {
    Context,
    Add,
    Delete,
    /// `\ No newline at end of file`：不占行号
    Meta,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub kind: LineKind,
    /// 旧文件里的行号；新增行没有旧行号
    pub old_no: Option<usize>,
    /// 新文件里的行号；删除行没有新行号
    pub new_no: Option<usize>,
    /// 去掉行首标记符之后的内容。**行尾的 `\r` 原样留着**：CRLF 改动只有看得见才叫改过
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    /// `@@` 后面的段落名（函数上下文），git 给什么就显示什么
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blob {
    pub mime: String,
    /// 原始字节的 base64，前端拼成 data URL
    pub base64: String,
    pub bytes: u64,
}

/// 手写而不是 derive：blob 的 base64 最大 5 MB 多，出错时把它整串打进断言消息里，
/// 真正要看的那几行就被埋了。
impl std::fmt::Debug for Blob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blob")
            .field("mime", &self.mime)
            .field("bytes", &self.bytes)
            .field("base64_len", &self.base64.len())
            .finish()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Images {
    /// 纯新增（或删除）时对应的一侧是 None
    pub old: Option<Blob>,
    pub new: Option<Blob>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub path: String,
    pub render: Render,
    /// TooLarge 时是空的：原文不回传，否则一次 IPC 塞进 2 MB 文本
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub deleted: usize,
    pub line_count: usize,
    pub byte_count: usize,
    /// 二进制 / 降级时给界面看的大小差
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
    pub images: Option<Images>,
}

/// 读某个文件在"这条提交相对它的第一个父"之间的差异。
///
/// `old_path` 是改名前的路径：只在改名时给，两个路径一起当 pathspec 传下去，
/// git 才会把改名前后配成同一段 diff（只给新路径时它可能配不成，退化成"一删一增"两段）。
///
/// browse（treeless）仓库允许读：diff 会按需向远程取 blob，那是读，不是写。
pub fn read(
    repo: &Path,
    sha: &str,
    path: &str,
    old_path: Option<&str>,
    ignore_white_space: bool,
) -> Result<Diff, GitError> {
    let parents = detail::parents(repo, sha)?;
    let mut args: Vec<String> = vec![
        if parents.len() > 1 { "diff" } else { "show" }.to_string(),
        // 三个都必须是"关"：颜色转义会混进原文，外部 diff 工具和 textconv（比如把
        // docx 转成文本的那个）给出的都不是这个文件的内容
        "--no-color".to_string(),
        "--no-textconv".to_string(),
        "--no-ext-diff".to_string(),
        // 改名要配成一段，才拿得到成对的 -/+ 行
        "--find-renames".to_string(),
        CONTEXT_LINES.to_string(),
    ];
    if ignore_white_space {
        args.push("--ignore-all-space".to_string());
    }
    if parents.len() > 1 {
        args.push(format!("{sha}^1"));
        args.push(sha.to_string());
    } else {
        args.push("--format=".to_string());
        args.push(sha.to_string());
    }
    args.push("--".to_string());
    // `:(literal)`：文件名里带 `*`、`[`、`?` 的仓库真存在（比如截图导出），当通配传下去
    // 就会比到别的文件上——画错文件比画不出来严重得多
    args.push(format!(":(literal){path}"));
    if let Some(old_path) = old_path {
        if old_path != path {
            args.push(format!(":(literal){old_path}"));
        }
    }
    let out = process::run_bytes(Some(repo), &process::strs(&args), &[])?.expect_success()?;
    assemble(repo, path, &out)
}

/// 解析 + 阈值判定 + 图片取字节，全在一趟里做完。
fn assemble(repo: &Path, path: &str, stdout: &[u8]) -> Result<Diff, GitError> {
    let text = String::from_utf8_lossy(stdout).into_owned();
    let parsed = parse(&text)?;
    let byte_count = stdout.len();

    let mut diff = Diff {
        path: path.to_string(),
        render: parsed.render,
        hunks: parsed.hunks,
        added: parsed.added,
        deleted: parsed.deleted,
        line_count: parsed.line_count,
        byte_count,
        old_size: None,
        new_size: None,
        images: None,
    };

    // 阈值闸在解析之后：统计还得给，那正是降级之后唯一能显示的东西
    if diff.render == Render::Text && (diff.line_count > MAX_DIFF_LINES || byte_count > MAX_DIFF_BYTES)
    {
        diff.render = Render::TooLarge;
        diff.hunks = Vec::new();
    }

    if diff.render == Render::Binary {
        match upgrade_to_images(repo, path, parsed.blobs) {
            Some(images) => {
                diff.render = Render::Image;
                diff.old_size = images.old.as_ref().map(|blob| blob.bytes);
                diff.new_size = images.new.as_ref().map(|blob| blob.bytes);
                diff.images = Some(images);
            }
            // 取不到字节就停在二进制那一档，界面照样能给"这文件不能按行比"
            None => {
                let (old, new) = (real_blob(parsed.blobs.0), real_blob(parsed.blobs.1));
                let asked: Vec<&str> = old.into_iter().chain(new).collect();
                if !asked.is_empty() {
                    // `--batch-check` 按 stdin 逐行应答，回来的大小与问进去的顺序一致，
                    // 而号是缩写的，不能拿输出里的完整号反查
                    let sizes = sizes_of(repo, &asked).unwrap_or_default();
                    let slot = |index: usize| sizes.get(index).copied().flatten();
                    diff.old_size = if old.is_some() { slot(0) } else { None };
                    diff.new_size = match (old, new) {
                        (Some(_), Some(_)) => slot(1),
                        (None, Some(_)) => slot(0),
                        _ => None,
                    };
                }
            }
        }
    }

    Ok(diff)
}

/// 能画成图片的扩展名。**扩展名 + 内容头两道都要过**（§7.5）：
/// 一个改名叫 `.png` 的压缩包按图片画出来只会是一张坏图。
const IMAGE_SUFFIXES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("bmp", "image/bmp"),
    ("webp", "image/webp"),
    ("pdf", "application/pdf"),
];

/// 第一道闸：扩展名。svg 不在这里——它是文本，走正常的逐行 diff 才对。
fn mime_by_suffix(path: &str) -> Option<&'static str> {
    let dot = path.rfind('.')?;
    let suffix = path[dot + 1..].to_ascii_lowercase();
    IMAGE_SUFFIXES
        .iter()
        .find(|(name, _)| *name == suffix)
        .map(|(_, mime)| *mime)
}

/// 第二道闸：内容头。取到的字节必须以此为开头，否则不是这个格式。
fn matches_magic(mime: &str, head: &[u8]) -> bool {
    match mime {
        "image/png" => head.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a"),
        "image/bmp" => head.starts_with(b"BM"),
        "image/webp" => head.starts_with(b"RIFF") && head.get(8..12) == Some(&b"WEBP"[..]),
        "application/pdf" => head.starts_with(b"%PDF-"),
        _ => false,
    }
}

/// 二进制段能不能升成图片。任一侧对不上就整体退回二进制，不画半张。
fn upgrade_to_images(
    repo: &Path,
    path: &str,
    blobs: (Option<&str>, Option<&str>),
) -> Option<Images> {
    let mime = mime_by_suffix(path)?;
    let (old_sha, new_sha) = (real_blob(blobs.0), real_blob(blobs.1));
    let asked: Vec<&str> = old_sha.into_iter().chain(new_sha).collect();
    if asked.is_empty() {
        return None;
    }
    let fetched = read_blobs(repo, &asked).ok()?;
    let mut sides = fetched.into_iter();
    // 位置对齐：问进去几个就回来几个，顺序一致
    let old = old_sha.map(|_| sides.next().flatten()).flatten();
    let new = new_sha.map(|_| sides.next().flatten()).flatten();

    if old.is_none() && new.is_none() {
        return None;
    }
    // 拿到的字节必须真是这个格式；有一侧不是就是伪装
    if old.as_ref().is_some_and(|raw| !matches_magic(mime, raw.as_slice()))
        || new.as_ref().is_some_and(|raw| !matches_magic(mime, raw.as_slice()))
    {
        return None;
    }
    if old.as_ref().is_some_and(|raw| raw.len() > MAX_IMAGE_BYTES)
        || new.as_ref().is_some_and(|raw| raw.len() > MAX_IMAGE_BYTES)
    {
        return None;
    }
    Some(Images {
        old: old.map(|raw| encode(raw, mime)),
        new: new.map(|raw| encode(raw, mime)),
    })
}

/// `index` 行里的全零号（新增侧或删除侧）不是能查的对象。
fn real_blob(sha: Option<&str>) -> Option<&str> {
    let sha = sha?;
    if sha.is_empty() || sha.bytes().all(|byte| byte == b'0') {
        return None;
    }
    Some(sha)
}

fn encode(raw: Vec<u8>, mime: &'static str) -> Blob {
    Blob {
        mime: mime.to_string(),
        base64: base64(&raw),
        bytes: raw.len() as u64,
    }
}

/// 一次 `cat-file --batch` 把多个对象取回来。输出是"头行 + 内容 + 一个换行"的重复结构，
/// 所以按头行声明的字节数切片，不能按行切——图片里全是换行。
fn read_blobs(repo: &Path, shas: &[&str]) -> Result<Vec<Option<Vec<u8>>>, GitError> {
    let stdin: String = shas.iter().map(|sha| format!("{sha}\n")).collect();
    let out = process::run_bytes(Some(repo), &["cat-file", "--batch"], stdin.as_bytes())?
        .expect_success()?;

    let mut results = Vec::with_capacity(shas.len());
    let mut cursor = 0;
    while cursor < out.len() && results.len() < shas.len() {
        let Some(offset) = out[cursor..].iter().position(|byte| *byte == b'\n') else {
            break;
        };
        let end = cursor + offset;
        let header = String::from_utf8_lossy(&out[cursor..end]).into_owned();
        cursor = end + 1;
        let fields: Vec<&str> = header.split(' ').collect();
        // `<sha> missing`：对象取不到（treeless 仓库断网时会这样）
        if fields.len() < 3 || fields[1] == "missing" {
            results.push(None);
            continue;
        }
        let Ok(size) = fields[2].parse::<usize>() else {
            results.push(None);
            continue;
        };
        if cursor + size > out.len() {
            results.push(None);
            break;
        }
        results.push(Some(out[cursor..cursor + size].to_vec()));
        // 内容后面跟着一个换行分隔符
        cursor += size + 1;
    }
    while results.len() < shas.len() {
        results.push(None);
    }
    Ok(results)
}

/// 只要大小，不取内容：`--batch-check` 一趟问完，回来的是**与 shas 同序**的大小。
fn sizes_of(repo: &Path, shas: &[&str]) -> Result<Vec<Option<u64>>, GitError> {
    let stdin: String = shas.iter().map(|sha| format!("{sha}\n")).collect();
    let out = process::run_bytes(Some(repo), &["cat-file", "--batch-check"], stdin.as_bytes())?
        .expect_success()?;

    let text = String::from_utf8_lossy(&out);
    let mut lines = text.lines();
    let mut sizes = Vec::with_capacity(shas.len());
    for _ in shas {
        // 一行一个对象：`<sha> blob <size>`，不存在的是 `<输入> missing`
        sizes.push(match lines.next() {
            Some(line) => {
                let mut fields = line.split(' ');
                match (fields.next(), fields.next(), fields.next()) {
                    (Some(_), Some("blob"), Some(raw)) => raw.parse::<u64>().ok(),
                    _ => None,
                }
            }
            None => None,
        });
    }
    Ok(sizes)
}

struct Parsed<'a> {
    render: Render,
    hunks: Vec<Hunk>,
    added: usize,
    deleted: usize,
    line_count: usize,
    /// 二进制段里 `index old..new` 给的两个 blob，取图片要用
    blobs: (Option<&'a str>, Option<&'a str>),
}

/// 按 `git diff` 原文逐行扫。**不重算 diff**：hunk 头是唯一定位依据，
/// 自己算一套行号就和 git 对不上，而这类偏差不会报错，只会把内容挪错行。
fn parse(text: &str) -> Result<Parsed<'_>, GitError> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_no = 0;
    let mut new_no = 0;
    let mut added = 0;
    let mut deleted = 0;
    let mut binary = false;
    let mut blobs: (Option<&str>, Option<&str>) = (None, None);

    // split 而不是 lines：`lines()` 会把行尾的 \r 一起吃掉，那正好抹掉 CRLF 改动
    let mut lines: Vec<&str> = text.split('\n').collect();
    // 末尾那个空片段是最后一个换行符的产物，不是内容。diff 的每一行正文都带前缀字符，
    // 所以只有这一种情况会让最后一片是空的
    if lines.last().is_some_and(|tail| tail.is_empty()) {
        lines.pop();
    }
    let line_count = lines.len();

    for raw in lines {
        if raw.starts_with("diff --git ") {
            // 一个 pathspec 通常只有一段；配不成改名时会是"一删一增"两段，
            // 两段都画完才是这件事的全貌，所以只收尾、不提前停
            push_hunk(&mut hunks, &mut current);
            continue;
        }
        if raw.starts_with("@@") {
            let hunk = hunk_header(raw)?;
            push_hunk(&mut hunks, &mut current);
            old_no = hunk.old_start;
            new_no = hunk.new_start;
            current = Some(hunk);
            continue;
        }
        if current.is_none() {
            // 还没进任何一个 hunk：这一段全是文件头（index / mode / --- / +++ / Binary）。
            // `index` 那一行紧挨在 `Binary files` 前面，所以最后留下的就是那段的 blob。
            if let Some(rest) = raw.strip_prefix("index ") {
                blobs = index_pair(rest);
            } else if raw.starts_with("Binary files ") {
                binary = true;
            }
            continue;
        }

        let (kind, body) = match raw.as_bytes().first() {
            Some(b'+') => (LineKind::Add, &raw[1..]),
            Some(b'-') => (LineKind::Delete, &raw[1..]),
            // 空行是"内容为空的上下文行"：git 会输出一个空格前缀，但某些终端会把它去掉
            Some(b' ') | None => (LineKind::Context, raw.strip_prefix(' ').unwrap_or(raw)),
            Some(b'\\') => (LineKind::Meta, raw),
            // 不认识的前缀：按上下文画，内容不丢。宁可少个颜色也不能少一行
            _ => (LineKind::Context, raw),
        };

        let (line_old, line_new) = match kind {
            LineKind::Context => {
                let pair = (Some(old_no), Some(new_no));
                old_no += 1;
                new_no += 1;
                pair
            }
            LineKind::Add => {
                added += 1;
                let pair = (None, Some(new_no));
                new_no += 1;
                pair
            }
            LineKind::Delete => {
                deleted += 1;
                let pair = (Some(old_no), None);
                old_no += 1;
                pair
            }
            // 「\ No newline at end of file」紧跟在它说明的那一行后面，不占行号
            LineKind::Meta => (None, None),
        };
        let hunk = current.as_mut().expect("上面刚确认过有 hunk");
        hunk.lines.push(Line {
            kind,
            old_no: line_old,
            new_no: line_new,
            text: body.to_string(),
        });
    }
    push_hunk(&mut hunks, &mut current);

    // 有 hunk 就说明这是能按行比的内容，哪怕同一次里还夹着别段
    let render = if hunks.is_empty() {
        if binary {
            Render::Binary
        } else {
            Render::Empty
        }
    } else {
        Render::Text
    };
    Ok(Parsed {
        render,
        hunks,
        added,
        deleted,
        line_count,
        blobs,
    })
}

fn push_hunk(hunks: &mut Vec<Hunk>, current: &mut Option<Hunk>) {
    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }
}

/// `@@ -1,7 +1,8 @@ 段落名`。count 为 1 时 git 会省略 `,1`，两种写法都要接。
///
/// 区间夹在两个 `@@` 之间，段落名在第二个后面——按空白整串拆开再接会把 `@@` 当成段落名
/// 的一部分带出去（段落名那一栏就多个 @@ 在前面，而函数名里真能出现 `@@`，C++ 的
/// `Class@@method` 那种）。找不到收尾的 `@@` 就是形态不认识，报错。
fn hunk_header(raw: &str) -> Result<Hunk, GitError> {
    let after = raw.strip_prefix("@@").ok_or_else(|| GitError::ParseFailure {
        snippet: format!("hunk 头不以 @@ 开头：{raw}"),
    })?;
    let Some(closing) = after.find("@@") else {
        return Err(GitError::ParseFailure {
            snippet: format!("hunk 头没有收尾的 @@：{raw}"),
        });
    };
    let mut fields = after[..closing].split_whitespace();
    let old = fields.next().ok_or_else(|| GitError::ParseFailure {
        snippet: format!("hunk 头没有旧区间：{raw}"),
    })?;
    let new = fields.next().ok_or_else(|| GitError::ParseFailure {
        snippet: format!("hunk 头没有新区间：{raw}"),
    })?;
    let (old_start, old_count) = range(old, '-')?;
    let (new_start, new_count) = range(new, '+')?;
    Ok(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        header: after[closing + 2..].trim_start().to_string(),
        lines: Vec::new(),
    })
}

fn range(raw: &str, sign: char) -> Result<(usize, usize), GitError> {
    let body = raw
        .strip_prefix(sign)
        .ok_or_else(|| GitError::ParseFailure {
            snippet: format!("hunk 区间少了 {sign}：{raw}"),
        })?;
    let mut parts = body.split(',');
    let start = parts
        .next()
        .unwrap_or_default()
        .parse::<usize>()
        .map_err(|_| GitError::ParseFailure {
            snippet: format!("hunk 起点不是数字：{raw}"),
        })?;
    let count = match parts.next() {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| GitError::ParseFailure {
                snippet: format!("hunk 长度不是数字：{raw}"),
            })?,
        // 省略写法就是 1
        None => 1,
    };
    Ok((start, count))
}

/// `index 9dae196..24636d0 100644` → 两个 blob 号。缩写形态留着，取字节时 cat-file 认前缀。
fn index_pair(raw: &str) -> (Option<&str>, Option<&str>) {
    let head = raw.split(' ').next().unwrap_or(raw);
    match head.split_once("..") {
        Some((old, new)) if !old.is_empty() && !new.is_empty() => (Some(old), Some(new)),
        _ => (None, None),
    }
}

/// 标准 base64，手写：为一个显示用途往 Cargo.toml 加依赖，换来的是他那边多一次拉包。
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let byte = |index: usize| chunk.get(index).copied().unwrap_or(0) as u32;
        let group = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        out.push(char::from(ALPHABET[((group >> 18) & 63) as usize]));
        out.push(char::from(ALPHABET[((group >> 12) & 63) as usize]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[((group >> 6) & 63) as usize])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[(group & 63) as usize])
        } else {
            '='
        });
    }
    out
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
        // 本机 global 里的 core.autocrlf 会跟着 temp 仓库一起生效：真那样的话写进去的
        // CRLF 在 add 时就被normalize 成 LF，"把第一行换成 LF"这次改动根本不存在，
        // 行尾 \r 那个测试就成了碰运气。这一条把行尾换算按死在"不换算"。
        git_in(dir.path(), &["config", "core.autocrlf", "false"]);
        dir
    }

    fn commit(dir: &StdPath, msg: &str) -> String {
        git_in(dir, &["add", "-A"]);
        git_in(dir, &["commit", "-q", "-m", msg]);
        git_in(dir, &["rev-parse", "HEAD"]).trim().to_string()
    }

    #[test]
    fn line_numbers_come_from_the_hunk_header_and_count_up() {
        let dir = repo();
        fs::write(
            dir.path().join("a.txt"),
            "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n",
        )
        .expect("write");
        commit(dir.path(), "chore: 铺底");
        fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\n")
            .expect("write");
        let sha = commit(dir.path(), "fix: 改第二行");

        let diff = read(dir.path(), &sha, "a.txt", None, false).expect("diff");
        assert_eq!(diff.render, Render::Text, "{diff:?}");
        assert_eq!(diff.hunks.len(), 1);
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.new_start, 1);
        // 只改一行：新旧都是 8 行，段里有 上下文 + 一删一增 + 上下文
        assert_eq!((diff.added, diff.deleted), (1, 1));
        let dels: Vec<Option<usize>> = hunk
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Delete)
            .map(|line| line.old_no)
            .collect();
        assert_eq!(dels, vec![Some(2)], "删除行的旧行号必须是 2");
        let adds: Vec<Option<usize>> = hunk
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Add)
            .map(|line| line.new_no)
            .collect();
        assert_eq!(adds, vec![Some(2)]);
        // 上下文行的新行号要跟着走，否则前端两栏行号一起错位
        let last = hunk.lines.last().expect("hunk 有内容");
        assert_eq!((last.old_no, last.new_no), (Some(8), Some(8)));
    }

    /// `lines()` 会连行尾 \r 一起吃掉，那样 CRLF 改动看着和普通改动一模一样
    #[test]
    fn a_crlf_change_keeps_the_carriage_return() {
        let dir = repo();
        fs::write(dir.path().join("crlf.txt"), "one\r\ntwo\r\n").expect("write");
        commit(dir.path(), "chore: CRLF 铺底");
        fs::write(dir.path().join("crlf.txt"), "one\ntwo\r\n").expect("write");
        let sha = commit(dir.path(), "fix: 把第一行换成 LF");

        let diff = read(dir.path(), &sha, "crlf.txt", None, false).expect("diff");
        let removed = diff.hunks[0]
            .lines
            .iter()
            .find(|line| line.kind == LineKind::Delete)
            .expect("有一行被删");
        assert!(
            removed.text.ends_with('\r'),
            "删除行的行尾 \\r 必须留着，不然 CRLF 改动看不出来：{:?}",
            removed.text
        );
        let kept = diff.hunks[0]
            .lines
            .iter()
            .find(|line| line.kind == LineKind::Context)
            .expect("有上下文行");
        assert!(kept.text.ends_with('\r'), "上下文行也一样：{:?}", kept.text);
    }

    #[test]
    fn a_missing_final_newline_is_meta_and_shifts_no_line_number() {
        let dir = repo();
        fs::write(dir.path().join("eof.txt"), "one\ntwo").expect("write");
        commit(dir.path(), "chore: 没有末尾换行");
        fs::write(dir.path().join("eof.txt"), "one\nthree").expect("write");
        let sha = commit(dir.path(), "fix: 改最后一行");

        let diff = read(dir.path(), &sha, "eof.txt", None, false).expect("diff");
        let metas: Vec<&Line> = diff.hunks[0]
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Meta)
            .collect();
        assert_eq!(metas.len(), 2, "两侧各有一条 no newline：{metas:?}");
        for meta in &metas {
            assert_eq!((meta.old_no, meta.new_no), (None, None), "标记行不占行号");
            assert!(meta.text.starts_with('\\'), "原文照留：{meta:?}");
        }
        let last = diff.hunks[0]
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Add)
            .last()
            .expect("有新增行");
        assert_eq!(last.new_no, Some(2), "「\\\\」标记把行号顶歪了：{diff:?}");
    }

    /// 纯空白改动：忽略空白之后这一段就没有 hunk 了，界面要显示"无差异"而不是空块
    #[test]
    fn ignoring_whitespace_empties_a_whitespace_only_change() {
        let dir = repo();
        fs::write(dir.path().join("ws.txt"), "one\n").expect("write");
        commit(dir.path(), "chore: 铺底");
        fs::write(dir.path().join("ws.txt"), "one   \n").expect("write");
        let sha = commit(dir.path(), "style: 尾随空格");

        let plain = read(dir.path(), &sha, "ws.txt", None, false).expect("diff");
        assert_eq!(plain.render, Render::Text);
        assert_eq!(plain.hunks.len(), 1);

        let ignored = read(dir.path(), &sha, "ws.txt", None, true).expect("diff");
        assert_eq!(ignored.render, Render::Empty, "忽略空白后不该再有 hunk：{ignored:?}");
        assert!(ignored.hunks.is_empty());
    }

    #[test]
    fn a_renamed_file_stays_one_section() {
        let dir = repo();
        fs::write(dir.path().join("old.txt"), "alpha\nbeta\ngamma\n").expect("write");
        commit(dir.path(), "chore: 铺底");
        fs::remove_file(dir.path().join("old.txt")).expect("rm");
        fs::write(dir.path().join("new.txt"), "alpha\nBETA\ngamma\n").expect("write");
        let sha = commit(dir.path(), "refactor: 改名又改一行");

        let diff = read(dir.path(), &sha, "new.txt", Some("old.txt"), false).expect("diff");
        assert_eq!(diff.render, Render::Text, "改名要配成一段：{diff:?}");
        assert_eq!((diff.added, diff.deleted), (1, 1), "两段各一半就不对了：{diff:?}");
        assert_eq!(diff.hunks.len(), 1);
    }

    /// 图片：二进制段 + 扩展名 + 内容头，三个条件都成立才取字节
    #[test]
    fn a_png_pair_comes_back_as_base64() {
        let dir = repo();
        let header: Vec<u8> = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 1, 2, 3];
        fs::write(dir.path().join("p.png"), &header).expect("write");
        commit(dir.path(), "chore: 一张假 png");
        let mut bigger = header.clone();
        bigger.push(4);
        fs::write(dir.path().join("p.png"), &bigger).expect("write");
        let sha = commit(dir.path(), "fix: 换一张");

        let diff = read(dir.path(), &sha, "p.png", None, false).expect("diff");
        assert_eq!(diff.render, Render::Image, "内容头是 PNG 就该升成图片：{diff:?}");
        let images = diff.images.as_ref().expect("两个 blob");
        assert_eq!(images.new.as_ref().expect("新侧").bytes, 12);
        assert_eq!(images.old.as_ref().expect("旧侧").base64, base64(&header));
        assert_eq!(images.new.as_ref().expect("新侧").mime, "image/png");
        assert_eq!(diff.old_size, Some(11));
        assert_eq!(diff.new_size, Some(12));
    }

    /// 只有扩展名像、内容不是图片：不能升成图片，否则浏览器画出来是一张坏图
    #[test]
    fn a_fake_png_stays_binary() {
        let dir = repo();
        fs::write(dir.path().join("fake.png"), [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9]).expect("write");
        commit(dir.path(), "chore: 一个假冒的 png");
        fs::write(dir.path().join("fake.png"), [0u8; 12]).expect("write");
        let sha = commit(dir.path(), "fix: 换一坨");

        let diff = read(dir.path(), &sha, "fake.png", None, false).expect("diff");
        assert_eq!(diff.render, Render::Binary, "内容头不对就不该升成图片：{diff:?}");
        assert!(diff.images.is_none());
        assert_eq!(diff.old_size, Some(10), "退回二进制也得给大小差");
        assert_eq!(diff.new_size, Some(12));
    }

    /// 真正新增的图片：旧侧那个全零号不是对象，取不到也不算失败
    #[test]
    fn a_newly_added_image_has_only_the_new_side() {
        let dir = repo();
        fs::write(dir.path().join("keep.txt"), "one\n").expect("write");
        commit(dir.path(), "chore: 铺底");
        fs::write(
            dir.path().join("new.gif"),
            [b'G', b'I', b'F', b'8', b'9', b'a', 1, 2, 3],
        )
        .expect("write");
        let sha = commit(dir.path(), "feat: 加一张 gif");

        let diff = read(dir.path(), &sha, "new.gif", None, false).expect("diff");
        assert_eq!(diff.render, Render::Image, "{diff:?}");
        let images = diff.images.as_ref().expect("至少一侧有图");
        assert!(images.old.is_none(), "新增不该有旧侧：{images:?}");
        assert_eq!(images.new.as_ref().expect("新侧").bytes, 9);
    }

    #[test]
    fn an_oversized_diff_degrades_to_stats() {
        let dir = repo();
        let small: String = (0..5).map(|index| format!("line {index}\n")).collect();
        fs::write(dir.path().join("big.txt"), &small).expect("write");
        commit(dir.path(), "chore: 铺底");
        let large: String = (0..(MAX_DIFF_LINES + 50))
            .map(|index| format!("line {index}\n"))
            .collect();
        fs::write(dir.path().join("big.txt"), &large).expect("write");
        let sha = commit(dir.path(), "perf: 一次塞很多行");

        let diff = read(dir.path(), &sha, "big.txt", None, false).expect("diff");
        assert_eq!(diff.render, Render::TooLarge, "{:?}", diff.render);
        assert!(diff.hunks.is_empty(), "降级不能把原文一起带回去");
        assert!(diff.line_count > MAX_DIFF_LINES);
        assert!(diff.added > MAX_DIFF_LINES, "统计要留下：{}", diff.added);
    }

    #[test]
    fn base64_matches_the_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn hunk_ranges_accept_the_omitted_count() {
        let hunk = hunk_header("@@ -1 +1,2 @@ fn main()").expect("省略写法");
        assert_eq!((hunk.old_start, hunk.old_count), (1, 1));
        assert_eq!((hunk.new_start, hunk.new_count), (1, 2));
        assert_eq!(hunk.header, "fn main()");

        let bare = hunk_header("@@ -3,7 +3,9 @@").expect("没有段落名");
        assert_eq!(bare.header, "");

        // C++ 的函数上下文真会带 @@，段落名要从收尾的 @@ 后面完整取出来
        let cpp = hunk_header("@@ -1,2 +1,3 @@ Class@@method(int)").expect("带 @@ 的段落名");
        assert_eq!(cpp.header, "Class@@method(int)");

        let err = hunk_header("@@ 坏了 @@").expect_err("区间不是区间要报错");
        assert!(format!("{err:?}").contains("hunk"), "{err:?}");
        assert!(hunk_header("@@ -1 +1 没有收尾").is_err(), "找不到收尾的 @@ 就报错");
    }

    /// 认不出来的 hunk 头要带着原文报错，不能悄悄把行号算错
    #[test]
    fn an_unreadable_hunk_header_is_a_parse_failure() {
        // `.err()` 而不是 `.expect_err()`：Parsed 没有 Debug，后者编译不过
        let err = parse("@@ 这不是区间 @@\n").err().expect("该报错");
        assert!(format!("{err:?}").contains("hunk"), "{err:?}");
    }

    #[test]
    fn suffix_and_magic_have_to_agree() {
        assert_eq!(mime_by_suffix("a/b.PNg"), Some("image/png"));
        assert_eq!(mime_by_suffix("Makefile"), None);
        assert_eq!(mime_by_suffix("archive.7z"), None);
        assert!(matches_magic("image/png", b"\x89PNG\r\n\x1a\nxx"));
        assert!(!matches_magic("image/png", b"\x89PN"));
        assert!(matches_magic("image/webp", b"RIFF\x00\x00\x00\x00WEBPVP8 "));
        assert!(!matches_magic("image/webp", b"RIFF\x00\x00\x00\x00XXXXYYYY"));
        // 短于 12 字节的 RIFF 头不能算 webp，越界读取不行
        assert!(!matches_magic("image/webp", b"RIFF"));
    }
}
