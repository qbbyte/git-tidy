use std::fmt;

/// 面向前端的统一错误。`code` 供前端做分支判断，`message` 是可直接展示的中文文案，
/// `detail` 只用于"查看原始输出"展开，前端不得据其内容做逻辑判断。
pub enum GitError {
    /// 系统里没有 git 可执行文件
    GitNotFound,
    /// 目标目录不在任何 Git 仓库内
    NotARepo,
    /// 注册表里查不到这个 id：记录已被移除，或数据库来自另一台机器
    RepoNotFound,
    /// 只读浏览（treeless）仓库没有工作区，一切工作区级操作在 Rust 侧就被拒绝（§7.5）
    ReadOnlyRepo,
    /// 粘贴进来的仓库地址不被接受。git 能把 `ext::`、`fd::` 这类形态当命令执行，
    /// 而地址是从界面传进来的，所以不合规的一律拒绝、绝不传给 git。
    BadRepoUrl,
    /// git 以非零退出，且该退出码在本上下文里没有更具体的含义
    GitFailed { stderr: String },
    /// 提交信息不符合本仓库规范。需求 7.5 要求拦截在 Rust 侧兜底，
    /// 所以这条是"绕过界面直接调命令"时收到的错误，负载就是界面要逐条列出的原因。
    NotConformant {
        violations: Vec<crate::config::check::Violation>,
    },
    /// 索引里没有任何已暂存的改动
    NothingStaged,
    /// 仓库停在 merge / rebase / cherry-pick / revert 的中断态上（§7.3）。
    /// 哪一种由 state 带出去：界面据此决定提示条文案，也据此禁用写入口。
    OperationInProgress {
        state: crate::git::refs::Interrupt,
    },
    /// git 输出与预期的记录结构不符：分隔符数量对不上、时间戳位置落进非数字。
    /// 只把截断后的原文片段放进 detail，用户看到的是通用文案。
    ParseFailure { snippet: String },
    /// 阻塞任务 panic，或异步运行时丢任务
    Internal(String),
}

impl GitError {
    fn code(&self) -> &'static str {
        match self {
            Self::GitNotFound => "git_not_found",
            Self::NotARepo => "not_a_repo",
            Self::RepoNotFound => "repo_not_found",
            Self::ReadOnlyRepo => "read_only_repo",
            Self::BadRepoUrl => "bad_repo_url",
            Self::GitFailed { .. } => "git_failed",
            Self::NotConformant { .. } => "message_not_conformant",
            Self::NothingStaged => "nothing_staged",
            Self::OperationInProgress { .. } => "operation_in_progress",
            Self::ParseFailure { .. } => "parse_failure",
            Self::Internal(_) => "internal",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::GitNotFound => "未检测到 Git，请先安装 Git 并确保它在系统 PATH 中".into(),
            Self::NotARepo => "所选目录不是 Git 仓库".into(),
            Self::RepoNotFound => "这个仓库不在列表里，请重新添加".into(),
            Self::ReadOnlyRepo => "只读浏览的仓库没有工作区，无法查看待提交文件或提交".into(),
            Self::BadRepoUrl => {
                "仓库地址无法识别。支持 https:// 、http:// 、git:// 、ssh:// 或 git@host:org/repo.git"
                    .into()
            }
            Self::GitFailed { stderr } => {
                let first_line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                if first_line.is_empty() {
                    "Git 命令执行失败".into()
                } else {
                    first_line.to_string()
                }
            }
            Self::ParseFailure { .. } => {
                "读取 Git 输出时格式不符预期，请重试；若持续出现请展开原始输出反馈".into()
            }
            Self::NotConformant { violations } => {
                let titles: Vec<String> = violations.iter().map(|v| v.title.clone()).collect();
                if titles.is_empty() {
                    "提交信息不符合本仓库规范".into()
                } else {
                    format!("提交信息不符合本仓库规范：{}", titles.join("；"))
                }
            }
            Self::NothingStaged => "索引里没有任何已暂存的改动，先在「变更」列表里勾上要提交的文件".into(),
            Self::OperationInProgress { state } => {
                let label = state.label();
                format!("{label}：请先完成或中止这次操作，再使用工具的提交")
            }
            Self::Internal(_) => "工具内部错误，请重试".into(),
        }
    }

    fn detail(&self) -> Option<String> {
        match self {
            Self::GitFailed { stderr } if !stderr.trim().is_empty() => Some(stderr.clone()),
            Self::ParseFailure { snippet } => Some(snippet.clone()),
            Self::NotConformant { violations } => Some(
                violations
                    .iter()
                    .map(|v| format!("{}：{}", v.title, v.hint))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Self::Internal(msg) => Some(msg.clone()),
            _ => None,
        }
    }
}

impl fmt::Debug for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())?;
        // detail 不进用户文案，但必须能在日志和测试断言里看到，否则 Internal 一律显示
        // "工具内部错误"，排查时等于没有信息
        if let Some(detail) = self.detail() {
            write!(f, " | {detail}")?;
        }
        Ok(())
    }
}

impl From<std::io::Error> for GitError {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::NotFound {
            Self::GitNotFound
        } else {
            Self::Internal(err.to_string())
        }
    }
}

impl serde::Serialize for GitError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            code: &'a str,
            message: String,
            detail: Option<&'a str>,
        }

        let detail = self.detail();
        Payload {
            code: self.code(),
            message: self.message(),
            detail: detail.as_deref(),
        }
        .serialize(serializer)
    }
}
