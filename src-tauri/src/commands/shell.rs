use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use tauri::State;

use crate::error::GitError;
use crate::store::db::{query, Db};
use crate::store::repos;

/// 在终端里打开这个仓库（需求 7.23）。
///
/// 「在资源管理器中显示」与「复制路径」不走这里：前者用 `@tauri-apps/plugin-opener`
/// 的系统 API（它自己按平台处理，且不经过 shell），后者就是写剪贴板。
/// 只有"开一个终端"没有跨平台 API，只能起进程，而**这一处必须保证路径不经过 shell**：
///
/// - 优先的做法是把目录设成子进程的 `current_dir`，让终端自己落在这个目录，
///   路径压根不成为参数；
/// - Windows 的兜底（`cmd /K cd /d <path>`）里，Rust 会把含空格的路径加上双引号，
///   而 `cmd` 的解析器把引号内的 `&`、`;` 当普通字符——所以这条路也是安全的；
/// - 全程没有 `sh -c` / `cmd /c <字符串>` 这种把字符串当代码执行的写法。
#[tauri::command]
pub async fn shell_open_terminal(state: State<'_, Arc<Db>>, id: i64) -> Result<String, GitError> {
    let path = query(state.inner().clone(), move |conn| {
        repos::ensure_worktree(conn, id)
    })
    .await?;
    tauri::async_runtime::spawn_blocking(move || open_terminal(&path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

fn open_terminal(dir: &Path) -> Result<String, GitError> {
    let candidates: Vec<Command> = terminal_candidates(dir);
    let mut last: Option<std::io::Error> = None;
    for mut command in candidates {
        match command.spawn() {
            Ok(_) => return Ok(dir.to_string_lossy().into_owned()),
            Err(err) => last = Some(err),
        }
    }
    let detail = last
        .map(|err| err.to_string())
        .unwrap_or_else(|| "没有可用的终端程序".to_string());
    Err(spawn_failed(&detail))
}

/// 失败文案单独抽出来：它要同时说清“发生了什么”和“可以怎么办”，
/// 所以值得能被直接断言，而不是只能靠真的去启动一个终端来触发。
fn spawn_failed(detail: &str) -> GitError {
    GitError::Internal(format!(
        "没能打开终端（{detail}）。可以复制仓库路径，自己在终端里 cd 过去"
    ))
}

/// 按平台给出候选启动方式，第一个能起来的算成功。
///
/// 都设了 `current_dir`：终端会落在这个目录，路径不需要作为参数传出去。
fn terminal_candidates(dir: &Path) -> Vec<Command> {
    #[cfg(windows)]
    {
        let mut wt = Command::new("wt.exe");
        wt.current_dir(dir);

        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/K").arg("cd").arg("/d").arg(dir);

        vec![wt, cmd]
    }
    #[cfg(target_os = "macos")]
    {
        // macOS 的 Terminal 不继承 current_dir，只能把目录作为参数（仍是独立 argv）
        let mut open = Command::new("open");
        open.arg("-a").arg("Terminal");
        open.arg(dir);
        vec![open]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut terminal = Command::new("x-terminal-emulator");
        terminal.current_dir(dir);

        let mut xterm = Command::new("xterm");
        xterm.current_dir(dir);

        vec![terminal, xterm]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这条测试锁的是"实现形状"而不是运行结果：命令行里必须能看出路径是
    /// 独立参数（或压根没出现），不能是被 shell 二次解析的一整串。
    #[test]
    fn the_path_is_never_glued_into_a_shell_string() {
        let dir = Path::new("/tmp/仓库 with space & echo pwned");
        for command in terminal_candidates(dir) {
            let rendered = format!("{command:?}");
            assert!(
                !rendered.contains("sh -c") && !rendered.contains("/C "),
                "不允许把字符串交给 shell：{rendered}"
            );
            assert!(
                !rendered.contains("echo pwned\";"),
                "路径不能带出命令分隔符：{rendered}"
            );
        }
    }

    #[test]
    fn there_is_at_least_one_candidate_to_try() {
        let dir = Path::new(".");
        assert!(!terminal_candidates(dir).is_empty());
    }

    /// 启动失败时的文案要给人话，并且给出替代做法
    #[test]
    fn a_failure_explains_what_to_do_by_hand() {
        let err = spawn_failed("程序找不到");
        let text = format!("{err:?}");
        assert!(text.contains("终端"), "{text}");
        assert!(text.contains("cd"), "要给出替代做法：{text}");
    }
}
