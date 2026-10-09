use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::GitError;

/// CREATE_NO_WINDOW：Tauri 是 GUI 子系统程序，不加这个标志每次调用 git 都会闪一个控制台窗口。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

impl GitOutput {
    /// 成功时返回 stdout，失败时把 stderr 包成 GitFailed。
    /// 调用方只在"非零退出确实是错误"的场合用它；空仓库这类可预期失败要自己判 success。
    pub fn expect_success(self) -> Result<String, GitError> {
        if self.success {
            Ok(self.stdout)
        } else {
            Err(GitError::GitFailed {
                stderr: self.stderr,
            })
        }
    }
}

/// 全局唯一的 git 调用出口。
///
/// 集中固定四件事，避免每个调用点重复踩坑：
/// 1. `LC_ALL=C` / `LANG=C` —— 让 stderr 保持英文，否则本地化后无法按文本判断错误类型
/// 2. `core.quotepath=false` —— 否则中文文件名被转成八进制转义序列
/// 3. `i18n.logOutputEncoding=utf-8` —— 提交信息里的中文按 UTF-8 输出
/// 4. `--no-pager` —— 防止 log/diff 类命令挂在读管道上等分页器
pub fn run(repo: Option<&Path>, args: &[&str]) -> Result<GitOutput, GitError> {
    let output = build(repo, args, &[])?.output()?;
    Ok(GitOutput {
        stdout: lossy(&output.stdout),
        stderr: lossy(&output.stderr),
        success: output.status.success(),
    })
}

/// 需要给单次调用加环境变量时用这个（比如 `GIT_NO_LAZY_FETCH=1` 做离线完整性校验）。
/// 只追加，上面四条全局固定项照旧。
pub fn run_with_env(
    repo: Option<&Path>,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<GitOutput, GitError> {
    let output = build(repo, args, extra_env)?.output()?;
    Ok(GitOutput {
        stdout: lossy(&output.stdout),
        stderr: lossy(&output.stderr),
        success: output.status.success(),
    })
}

/// 边跑边回调 stderr 的每一行。git 的下载进度全在 stderr，且用 `\r` 原地刷新，
/// 所以这里按 `\r` 和 `\n` 双双断行——只按 `\n` 切的话整个下载期间一个事件都发不出去。
///
/// stdout 是等进程结束才一次读完。这对 clone/fetch 是安全的：它们的进度和状态输出
/// 都走 stderr，stdout 近乎空，撑不满管道，不会跟 stderr reader 互相阻塞。
pub fn run_streaming(
    repo: Option<&Path>,
    args: &[&str],
    mut on_line: impl FnMut(&str),
) -> Result<GitOutput, GitError> {
    let mut cmd = build(repo, args, &[])?;
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    let stderr = child
        .stderr
        .take()
        .expect("stderr 已被声明为 piped，take 一定有值");
    let mut stdout = child
        .stdout
        .take()
        .expect("stdout 已被声明为 piped，take 一定有值");

    let mut collected = String::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut reader = std::io::BufReader::new(stderr);
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        for byte in &chunk[..read] {
            if *byte == b'\r' || *byte == b'\n' {
                if buf.is_empty() {
                    continue;
                }
                let line = String::from_utf8_lossy(&buf).into_owned();
                buf.clear();
                on_line(&line);
                collected.push_str(&line);
                collected.push('\n');
            } else {
                buf.push(*byte);
            }
        }
    }
    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf).into_owned();
        on_line(&line);
        collected.push_str(&line);
    }

    let status = child.wait()?;
    let mut out = String::new();
    stdout.read_to_string(&mut out)?;

    Ok(GitOutput {
        stdout: out,
        stderr: collected,
        success: status.success(),
    })
}

/// stdout 原样按字节返回，**读取二进制内容必须走这里**。
/// `run` 走的是 `String::from_utf8_lossy`，非 UTF-8 字节会被替换成 U+FFFD，图片过了那道
/// 转换就再也拼不回原样；`cat-file --batch-check` 之类的纯文本调用也共用这条出口，
/// 少一个分支就少一处会写错的地方。
///
/// `stdin` 非空时按需写入后立刻关闭管道（`--batch-check` 就是靠读到 EOF 才收工）。
/// 写在前、读在后，所以喂进去的量要小：本项目只喂"这一条提交里的对象号"，几十行，
/// 撑不满管道，不会和子进程的输出互相堵死。
pub fn run_bytes(repo: Option<&Path>, args: &[&str], stdin: &[u8]) -> Result<ByteOutput, GitError> {
    let mut cmd = build(repo, args, &[])?;
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = cmd.spawn()?;
    {
        let mut handle = child
            .stdin
            .take()
            .expect("stdin 已被声明为 piped，take 一定有值");
        handle.write_all(stdin)?;
    }
    let output = child.wait_with_output()?;
    Ok(ByteOutput {
        stdout: output.stdout,
        stderr: lossy(&output.stderr),
        success: output.status.success(),
    })
}

pub struct ByteOutput {
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub success: bool,
}

impl ByteOutput {
    /// 成功时返回 stdout 的原始字节。
    pub fn expect_success(self) -> Result<Vec<u8>, GitError> {
        if self.success {
            Ok(self.stdout)
        } else {
            Err(GitError::GitFailed {
                stderr: self.stderr,
            })
        }
    }
}

/// 拼出来的参数（`Vec<String>`）过不进上面那几个入口：它们收 `&[&str]`，
/// 而 `&Vec<String>` 和它是两种类型，没有隐式转换。
/// 集中在这里转一次，调用点就不用各自 `iter().map` 一遍、也少一处会写错的地方。
pub fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// 统一装配命令。返回 Command 而不是直接执行，让上面几个入口共用同一套环境固定项。
fn build(
    repo: Option<&Path>,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<Command, GitError> {
    let mut cmd = Command::new("git");
    cmd.arg("--no-pager")
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("-c")
        .arg("i18n.logOutputEncoding=utf-8")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .envs(extra_env.iter().map(|(key, value)| (*key, *value)))
        // 私有仓库在没有凭据管理器时会直接失败，而不是挂在一个人看不见、
        // 也回答不了的终端提示上（GUI 进程里 stdin 是 null，提示永远等不到输入）
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .args(args);

    if let Some(repo) = repo {
        cmd.current_dir(repo);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    Ok(cmd)
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 解析失败时带进 detail 和日志的原文片段。截断是必须的：一条 `%b` 可能是几十 KB 的正文，
/// 原样塞进错误载荷会把 IPC 撑爆，而用户要看的是"哪一条坏了"，不是全文。
pub(crate) fn snippet(record: &str) -> String {
    record.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = run(Some(dir.path()), &["init", "-q", "."]).expect("git init");
        assert!(out.success, "init 失败：{}", out.stderr);
        dir
    }

    fn envs(cmd: &Command) -> Vec<(String, String)> {
        cmd.get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect()
    }

    #[test]
    fn every_call_carries_the_language_and_prompt_fixes() {
        let cmd = build(None, &["--version"], &[("GIT_NO_LAZY_FETCH", "1")]).expect("build");
        let pairs = envs(&cmd);

        for (key, value) in [
            ("LC_ALL", "C"),
            ("LANG", "C"),
            // 没有这个，私有仓库会挂在一个 GUI 进程永远看不见的终端输入提示上
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_NO_LAZY_FETCH", "1"),
        ] {
            assert!(
                pairs.iter().any(|(k, v)| k == key && v == value),
                "缺少环境变量 {key}={value}，实际注入：{pairs:?}"
            );
        }
    }

    #[test]
    fn streaming_reports_stderr_lines_one_by_one() {
        let repo = init_repo();
        let mut lines: Vec<String> = Vec::new();

        // 空仓库里 checkout -b 也会往 stderr 打一行提示，且不碰网络
        let out = run_streaming(Some(repo.path()), &["checkout", "-b", "side"], |line| {
            lines.push(line.to_string())
        })
        .expect("streaming run");

        assert!(out.success, "checkout 失败：{}", out.stderr);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Switched to a new branch")),
            "提示行要逐条回调出去，实际：{lines:?}"
        );
        // 回调过的行同时进 stderr 字段，失败时才有的报
        assert!(out.stderr.contains("Switched to a new branch"));
    }

    #[test]
    fn extra_env_can_force_a_failure() {
        let repo = init_repo();
        // GIT_DIR 指到不存在的位置，git 必然失败：证明单次调用的环境注入生效了
        let out = run_with_env(
            Some(repo.path()),
            &["rev-parse", "--git-dir"],
            &[("GIT_DIR", "definitely-not-a-git-dir")],
        )
        .expect("run");
        assert!(
            !out.success,
            "环境注入没生效，命令反而成功了：{}",
            out.stdout
        );
    }

    #[test]
    fn quiet_commit_writes_nothing_and_still_reports_success() {
        let repo = init_repo();
        fs::write(repo.path().join("a.txt"), "one\n").expect("write");
        run(Some(repo.path()), &["add", "a.txt"]).expect("add");

        let out = run(
            Some(repo.path()),
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
        )
        .expect("commit");

        assert!(out.success, "commit 失败：{}", out.stderr);
        assert!(out.stdout.is_empty(), "-q 下 stdout 应为空：{}", out.stdout);
    }
}
