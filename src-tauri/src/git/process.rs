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
    let mut cmd = Command::new("git");
    cmd.arg("--no-pager")
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("-c")
        .arg("i18n.logOutputEncoding=utf-8")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .args(args);

    if let Some(repo) = repo {
        cmd.current_dir(repo);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output()?;
    Ok(GitOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    })
}
