use std::fmt;

/// 面向前端的统一错误。`code` 供前端做分支判断，`message` 是可直接展示的中文文案，
/// `detail` 只用于"查看原始输出"展开，前端不得据其内容做逻辑判断。
pub enum GitError {
    /// 系统里没有 git 可执行文件
    GitNotFound,
    /// 目标目录不在任何 Git 仓库内
    NotARepo,
    /// git 以非零退出，且该退出码在本上下文里没有更具体的含义
    GitFailed { stderr: String },
    /// 阻塞任务 panic，或异步运行时丢任务
    Internal(String),
}

impl GitError {
    fn code(&self) -> &'static str {
        match self {
            Self::GitNotFound => "git_not_found",
            Self::NotARepo => "not_a_repo",
            Self::GitFailed { .. } => "git_failed",
            Self::Internal(_) => "internal",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::GitNotFound => "未检测到 Git，请先安装 Git 并确保它在系统 PATH 中".into(),
            Self::NotARepo => "所选目录不是 Git 仓库".into(),
            Self::GitFailed { stderr } => {
                let first_line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                if first_line.is_empty() {
                    "Git 命令执行失败".into()
                } else {
                    first_line.to_string()
                }
            }
            Self::Internal(_) => "工具内部错误，请重试".into(),
        }
    }

    fn detail(&self) -> Option<String> {
        match self {
            Self::GitFailed { stderr } if !stderr.trim().is_empty() => Some(stderr.clone()),
            Self::Internal(msg) => Some(msg.clone()),
            _ => None,
        }
    }
}

impl fmt::Debug for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
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
