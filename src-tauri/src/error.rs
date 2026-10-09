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
    OperationInProgress { state: crate::git::refs::Interrupt },
    /// 前置校验不通过：工作区脏、中断态之外的状态不满足写命令的要求。
    /// `detail` 说明是哪一条，用户照着就能改，不用猜。
    NotClean { detail: String },
    /// HEAD 与界面加载时不一致（§4 的乐观并发）。IDE 或另一个终端正在写同一个仓库时会发生。
    /// 两个值都带出去：界面上能直接告诉用户"现在是哪个提交，你的界面还停在哪个"。
    HeadMoved { expected: String, actual: String },
    /// 结果校验失败：改写类操作执行完 tree 与执行前不一致（§4 步骤 4）。
    /// 此时已经回滚，这条错误说的是"为什么回滚"，不是"git 报了什么"。
    VerificationFailed { detail: String },
    /// 没有可撤销的写操作（这个仓库没写过，或最后一条不是成功的）。这不是崩溃，是答案。
    NothingToUndo,
    /// `index.lock` 存在：另一个进程正在写索引（§6.13）。**绝不**清掉那个锁文件，
    /// 重试两次仍占用就报这个，让用户去关掉那个进程。
    RepoBusy,
    /// 认证失败。凭据交给系统 git / GCM / ssh-agent（§6.14），所以文案必须可执行：
    /// "在终端跑一次 git push 完成登录，或在 Git Credential Manager 弹窗里授权"。
    AuthRequired { detail: String },
    /// 逐行/分块暂存时补丁不适用。`--check` 预验不过就是它，绝不半途写入索引。
    PatchApplyFailed { detail: String },
    /// 本地与远程已经分叉，快进拉取停住；或者 `--force-with-lease` 被拒。
    /// `detail` 说明接下来该做什么（先 fetch、还是决定合并/变基）。
    Diverged { detail: String },
    /// git 输出与预期的记录结构不符：分隔符数量对不上、时间戳位置落进非数字。
    /// 只把截断后的原文片段放进 detail，用户看到的是通用文案。
    ParseFailure { snippet: String },
    /// 仓库停在游离 HEAD 上（没有分支可指）。分支级改写在这里没有落脚点：
    /// `update-ref refs/heads/x` 造不出一个用户没要过的分支，所以直接拒。
    DetachedHead { detail: String },
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
            Self::NotClean { .. } => "not_clean",
            Self::HeadMoved { .. } => "head_moved",
            Self::VerificationFailed { .. } => "verification_failed",
            Self::NothingToUndo => "nothing_to_undo",
            Self::RepoBusy => "repo_busy",
            Self::AuthRequired { .. } => "auth_required",
            Self::PatchApplyFailed { .. } => "patch_apply_failed",
            Self::Diverged { .. } => "diverged",
            Self::ParseFailure { .. } => "parse_failure",
            Self::DetachedHead { .. } => "detached_head",
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
            Self::NotClean { detail } => format!("工作区不满足这次操作的要求：{detail}"),
            Self::HeadMoved { expected, actual } => format!(
                "仓库已经被别处改动（现在在 {short_actual}，操作界面停在 {short_expected}），已拒绝执行",
                short_actual = short(actual),
                short_expected = short(expected)
            ),
            Self::VerificationFailed { detail } => {
                format!("操作结果校验不通过，已回滚：{detail}")
            }
            Self::NothingToUndo => "没有可撤销的上一步写操作".into(),
            Self::RepoBusy => {
                "另一个进程正在写这个仓库的索引（存在 index.lock）。请先关掉它再试，重试不会清掉那个锁"
                    .into()
            }
            Self::AuthRequired { detail } => format!(
                "需要认证。请在终端里跑一次 git push / git pull 完成登录，或在 Git Credential Manager 弹窗里授权（详情：{detail}）"
            ),
            Self::PatchApplyFailed { detail } => format!(
                "选中的改动没法按补丁暂存，索引没有被动过：{detail}"
            ),
            Self::Diverged { detail } => format!("与远程已经分叉：{detail}"),
            Self::DetachedHead { detail } => format!(
                "仓库当前停在游离 HEAD 上，没有分支可以承接这次改写：{detail}"
            ),
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
            Self::NotClean { detail } => Some(detail.clone()),
            Self::VerificationFailed { detail } => Some(detail.clone()),
            Self::AuthRequired { detail } => Some(detail.clone()),
            Self::PatchApplyFailed { detail } => Some(detail.clone()),
            Self::Diverged { detail } => Some(detail.clone()),
            Self::DetachedHead { detail } => Some(detail.clone()),
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

/// 提交号在错误文案里只给前 8 位：两个 40 位串并排排出来没人读得下去
fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
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
