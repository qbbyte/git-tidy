use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::error::GitError;
use crate::git::branch::{self, Deletable};
use crate::git::conflict::{self, Conflict};
use crate::git::reset::{self, ResetMode};
use crate::git::rewrite;
use crate::git::stash::{self, StashEntry};
use crate::git::status::{self, HunkSelection, PartialSupport, WorkingFile};
use crate::git::sync::{self, PullStrategy, SyncReport};
use crate::store::db::{query, Db};
use crate::store::repos;
use crate::write::backup::{self, Backup};
use crate::write::guard::{self, Outcome, Request};
use crate::write::journal::{self, WriteOp};

/// 远程命令的进度事件。与克隆那套一样按行发，界面用一个进度条接住。
pub const SYNC_PROGRESS_EVENT: &str = "sync-progress";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncProgress {
    repo_id: i64,
    action: String,
    line: String,
}

/// 写操作的前置要求（§4 步骤 1）。写命令只描述"这次要做什么"，
/// 六步的固定顺序由 `guard::run` 保证。
struct Opts {
    /// 工作区必须干净
    clean: bool,
    /// 执行后比对 tree 必须一致（改写类）
    verifying_tree: bool,
    /// 界面加载时的 HEAD，做乐观并发校验
    expected_head: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

impl Opts {
    /// 只改引用/索引的操作（暂存、打标签、stash）：工作区脏正是它们存在的理由。
    fn light() -> Opts {
        Opts {
            clean: false,
            verifying_tree: false,
            expected_head: None,
            from: None,
            to: None,
        }
    }

    fn clean() -> Opts {
        Opts {
            clean: true,
            ..Opts::light()
        }
    }

    fn rewriting() -> Opts {
        Opts {
            clean: true,
            verifying_tree: true,
            ..Opts::light()
        }
    }

    fn at(mut self, head: Option<String>) -> Opts {
        self.expected_head = head;
        self
    }

    fn affecting(mut self, from: Option<String>, to: Option<String>) -> Opts {
        self.from = from;
        self.to = to;
        self
    }

    fn into_request(self, repo_id: i64, path: PathBuf, action: &'static str) -> Request {
        let mut request = Request::new(repo_id, path, action).affecting(self.from, self.to);
        if self.clean {
            request = request.clean();
        }
        if self.verifying_tree {
            request = request.verifying_tree();
        }
        request.expecting_head(self.expected_head)
    }
}

/// 每条写命令都从这里解析仓库：browse（treeless 只读浏览）仓库在 Rust 侧就被拒，
/// 前端传不了路径，只能传 id（§7.1、§7.5）。
async fn worktree(db: &Arc<Db>, id: i64) -> Result<PathBuf, GitError> {
    query(db.clone(), move |conn| repos::ensure_worktree(conn, id)).await
}

/// 在阻塞线程里跑一次写操作。六步都在 `guard::run` 里，这里只负责把参数搬进去。
async fn write(
    db: Arc<Db>,
    id: i64,
    path: PathBuf,
    action: &'static str,
    opts: Opts,
    body: impl FnMut(&Path) -> Result<(), GitError> + Send + 'static,
) -> Result<Outcome, GitError> {
    tauri::async_runtime::spawn_blocking(move || {
        let request = opts.into_request(id, path.clone(), action);
        let mut body = body;
        guard::run(&db, request, &mut move || body(&path))
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

// ---------------------------------------------------------------- 分支与标签（§7.10）

/// 建分支，`switch_to_it` 为真时建完就切过去。
#[tauri::command]
pub async fn branch_create(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
    start: Option<String>,
    switch_to_it: bool,
    expected_head: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    let action = if switch_to_it {
        "branch_create_switch"
    } else {
        "branch_create"
    };
    write(
        db,
        id,
        path,
        action,
        Opts::clean()
            .at(expected_head)
            .affecting(Some(name.clone()), Some(name.clone())),
        move |path| {
            branch::create(path, &name, start.as_deref())?;
            if switch_to_it {
                branch::switch(path, &name, false, None)?;
            }
            Ok(())
        },
    )
    .await
}

/// 删之前先问"会丢多少"。这是读操作，不进 guard——它不动仓库。
#[tauri::command]
pub async fn branch_deletable(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
) -> Result<Deletable, GitError> {
    let path = worktree(state.inner(), id).await?;
    // 基准取当前分支：用户在别的分支上删时，看到的是"相对当前分支还差几个"
    let base = guard::current_branch(&path)?.ok_or_else(|| GitError::GitFailed {
        stderr: "当前是游离 HEAD，先切到分支再删别的分支".into(),
    })?;
    tauri::async_runtime::spawn_blocking(move || branch::deletable(&path, &name, &base))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

#[tauri::command]
pub async fn branch_delete(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
    force: bool,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    let action = if force {
        "branch_delete_force"
    } else {
        "branch_delete"
    };
    write(
        db,
        id,
        path,
        action,
        Opts::light().affecting(Some(name.clone()), None),
        move |path| branch::delete(path, &name, force),
    )
    .await
}

#[tauri::command]
pub async fn branch_rename(
    state: State<'_, Arc<Db>>,
    id: i64,
    from: String,
    to: String,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "branch_rename",
        Opts::light().affecting(Some(from.clone()), Some(to.clone())),
        move |path| branch::rename(path, &from, &to),
    )
    .await
}

/// 切换分支。它移动 HEAD 并重写工作区，所以要干净工作区 + HEAD 一致（§7.10）。
#[tauri::command]
pub async fn branch_switch(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
    create: bool,
    start: Option<String>,
    expected_head: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "branch_switch",
        Opts::clean()
            .at(expected_head)
            .affecting(Some(name.clone()), Some(name.clone())),
        move |path| branch::switch(path, &name, create, start.as_deref()),
    )
    .await
}

#[tauri::command]
pub async fn upstream_set(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
    upstream: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "upstream_set",
        Opts::light().affecting(Some(name.clone()), upstream.clone()),
        move |path| branch::set_upstream(path, &name, upstream.as_deref()),
    )
    .await
}

#[tauri::command]
pub async fn tag_create(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
    message: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "tag_create",
        Opts::light().affecting(None, Some(name.clone())),
        move |path| branch::tag(path, &name, message.as_deref()),
    )
    .await
}

#[tauri::command]
pub async fn tag_delete(
    state: State<'_, Arc<Db>>,
    id: i64,
    name: String,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "tag_delete",
        Opts::light().affecting(Some(name.clone()), None),
        move |path| branch::delete_tag(path, &name),
    )
    .await
}

// ---------------------------------------------------------------- stash（§7.9）

#[tauri::command]
pub async fn stash_list(state: State<'_, Arc<Db>>, id: i64) -> Result<Vec<StashEntry>, GitError> {
    let path = worktree(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || stash::list(&path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

#[tauri::command]
pub async fn stash_push(
    state: State<'_, Arc<Db>>,
    id: i64,
    paths: Option<Vec<String>>,
    include_untracked: bool,
    message: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    // stash 存的就是工作区，所以它不能用"工作区干净"这个前置
    write(db, id, path, "stash_push", Opts::light(), move |path| {
        stash::push(
            path,
            paths.as_deref(),
            include_untracked,
            message.as_deref(),
        )
    })
    .await
}

#[tauri::command]
pub async fn stash_apply(
    state: State<'_, Arc<Db>>,
    id: i64,
    reference: String,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "stash_apply",
        Opts::light().affecting(Some(reference.clone()), None),
        move |path| stash::apply(path, &reference),
    )
    .await
}

#[tauri::command]
pub async fn stash_pop(
    state: State<'_, Arc<Db>>,
    id: i64,
    reference: String,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "stash_pop",
        Opts::light().affecting(Some(reference.clone()), None),
        move |path| stash::pop(path, &reference),
    )
    .await
}

#[tauri::command]
pub async fn stash_drop(
    state: State<'_, Arc<Db>>,
    id: i64,
    reference: String,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "stash_drop",
        Opts::light().affecting(Some(reference.clone()), None),
        move |path| stash::drop(path, &reference),
    )
    .await
}

#[tauri::command]
pub async fn stash_branch(
    state: State<'_, Arc<Db>>,
    id: i64,
    reference: String,
    name: String,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "stash_branch",
        Opts::clean().affecting(Some(reference.clone()), Some(name.clone())),
        move |path| stash::branch_from(path, &reference, &name),
    )
    .await
}

// ---------------------------------------------------------------- 摘取 / 回滚 / 复位（§7.11）

/// 冲突时返回 `OperationInProgress`：M2 只给一键退回，逐块解决在 M3（§7.13）。
#[tauri::command]
pub async fn op_cherry_pick(
    state: State<'_, Arc<Db>>,
    id: i64,
    shas: Vec<String>,
    record_source: bool,
    expected_head: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "cherry_pick",
        Opts::rewriting().at(expected_head),
        move |path| reset::cherry_pick(path, &shas, record_source),
    )
    .await
}

#[tauri::command]
pub async fn op_revert(
    state: State<'_, Arc<Db>>,
    id: i64,
    sha: String,
    mainline: Option<usize>,
    expected_head: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "revert",
        Opts::rewriting().at(expected_head),
        move |path| reset::revert(path, &sha, mainline),
    )
    .await
}

/// reset 三档。hard 会扔掉工作区里的东西，所以三档都要求工作区干净。
#[tauri::command]
pub async fn op_reset(
    state: State<'_, Arc<Db>>,
    id: i64,
    mode: ResetMode,
    target: String,
    expected_head: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    let action = match mode {
        ResetMode::Soft => "reset_soft",
        ResetMode::Mixed => "reset_mixed",
        ResetMode::Hard => "reset_hard",
    };
    write(
        db,
        id,
        path,
        action,
        Opts::clean()
            .at(expected_head)
            .affecting(Some(target.clone()), None),
        move |path| reset::reset(path, mode, &target),
    )
    .await
}

/// 中断态的一键退回。M2 不给"逐块取舍"，只给这条精确的退路（§3 的 M2 边界）。
#[tauri::command]
pub async fn op_abort(state: State<'_, Arc<Db>>, id: i64) -> Result<reset::AbortOutcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    // 先读一次中断态：退回之后标记文件就没了，事后读不到是哪一种
    let before = crate::git::refs::interrupt(&path)?;
    write(
        db,
        id,
        path.clone(),
        "operation_abort",
        Opts::clean(),
        move |path| reset::abort(path).map(|_| ()),
    )
    .await?;
    Ok(reset::AbortOutcome {
        aborted: before.kind.label().to_string(),
        branch: before.branch,
    })
}

// ---------------------------------------------------------------- 远程（§7.12）

#[tauri::command]
pub async fn remote_fetch(
    app: AppHandle,
    state: State<'_, Arc<Db>>,
    id: i64,
    remote: Option<String>,
) -> Result<SyncReport, GitError> {
    let path = worktree(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        sync::fetch(&path, remote.as_deref(), |line| {
            emit(&app, id, "fetch", line)
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

#[tauri::command]
pub async fn remote_pull(
    app: AppHandle,
    state: State<'_, Arc<Db>>,
    id: i64,
    remote: Option<String>,
    strategy: PullStrategy,
    expected_head: Option<String>,
) -> Result<Outcome, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    write(
        db,
        id,
        path,
        "pull",
        Opts::clean().at(expected_head),
        move |path| {
            sync::pull(path, remote.as_deref(), strategy, |line| {
                emit(&app, id, "pull", line)
            })
            .map(|_| ())
        },
    )
    .await
}

/// 推送。属于远程写，所以经 `write_guard`（留下还原点与审计行）。
///
/// 本地 HEAD 不动，所以回滚对它没有意义（guard 第 5 步会因 HEAD 未变而跳过），
/// 但审计仍然必要：万一推错了分支，“我推了什么”必须查得到。
#[tauri::command]
pub async fn remote_push(
    state: State<'_, Arc<Db>>,
    id: i64,
    remote: String,
    branch: String,
    set_upstream: bool,
    force_with_lease: bool,
) -> Result<SyncReport, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    let report = tauri::async_runtime::spawn_blocking(move || -> Result<SyncReport, GitError> {
        let mut captured: Option<SyncReport> = None;
        guard::run(
            &db,
            Request::new(id, path.clone(), "push")
                .affecting(Some(remote.clone()), Some(branch.clone())),
            &mut || {
                let report = sync::push(&path, &remote, &branch, set_upstream, force_with_lease)?;
                captured = Some(report);
                Ok(())
            },
        )?;
        // guard 的报量只说仓库状态，推送结果另给一份（更新了哪些引用）
        Ok(captured.expect("body 跑过就一定有结果"))
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))??;
    Ok(report)
}

#[tauri::command]
pub async fn remote_delete_branch(
    state: State<'_, Arc<Db>>,
    id: i64,
    remote: String,
    branch: String,
) -> Result<SyncReport, GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;
    tauri::async_runtime::spawn_blocking(move || -> Result<SyncReport, GitError> {
        let mut captured: Option<SyncReport> = None;
        guard::run(
            &db,
            Request::new(id, path.clone(), "delete_remote_branch")
                .affecting(Some(format!("{remote}/{branch}")), None),
            &mut || {
                let report = sync::delete_remote_branch(&path, &remote, &branch)?;
                captured = Some(report);
                Ok(())
            },
        )?;
        Ok(captured.expect("body 跑过就一定有结果"))
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

// ---------------------------------------------------------------- 交互式改写（§7.14）

/// 改写 todo 的初稿：`base..head` 区间里的每条提交。
///
/// 读操作，不进 write_guard：它不改变任何东西。todo 本身由界面给回 `rewrite_run`——
/// 排顺序、选动作、改信息都在界面上做，这里只负责把原始事实摆出来。
#[tauri::command]
pub async fn rewrite_plan(
    state: State<'_, Arc<Db>>,
    id: i64,
    base: String,
    head: String,
) -> Result<Vec<rewrite::PlanEntry>, GitError> {
    let path = worktree(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || rewrite::plan(&path, &base, &head))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 执行 todo。改写是本项目风险最高的写操作，六步一步不少（§4）。
///
/// tree 校验按 todo 的内容决定：有 `drop` 就**不能**要求 tree 一致——丢掉一条提交
/// 本来就会改内容，还要它一致就等于把这个功能禁用掉。除此之外（重排 / 压缩 / 改信息）
/// tree 必须与改写前逐字节相同，那正是判断改写有没有顺手改掉内容的唯一依据。
#[tauri::command]
pub async fn rewrite_run(
    app: AppHandle,
    state: State<'_, Arc<Db>>,
    id: i64,
    base: String,
    todo: Vec<rewrite::TodoItem>,
    expected_head: Option<String>,
) -> Result<(Outcome, rewrite::RewriteReport), GitError> {
    let db = state.inner().clone();
    let path = worktree(&db, id).await?;

    let drops = todo.iter().any(|item| item.action.changes_tree());
    let mut opts = if drops {
        Opts::clean()
    } else {
        Opts::rewriting()
    };
    opts = opts
        .at(expected_head.clone())
        .affecting(Some(base.clone()), expected_head);
    let action = "rewrite";

    tauri::async_runtime::spawn_blocking(move || {
        let mut captured: Option<rewrite::RewriteReport> = None;
        let mut body = |path: &Path| -> Result<(), GitError> {
            // 这两个值必须在 body 里现读：guard 已经建好还原点，但它不把执行前的
            // HEAD / 分支交给 body，而 `promote` 的乐观锁就靠它们
            let old_head = guard::head_sha(path)?.ok_or_else(|| GitError::DetachedHead {
                detail: "仓库还没有任何提交".into(),
            })?;
            let branch = guard::current_branch(path)?.ok_or_else(|| GitError::DetachedHead {
                detail: "先把这次改写落在某个分支上再试".into(),
            })?;

            let report = rewrite::run(path, &base, &old_head, &todo, &mut |line| {
                emit(&app, id, action, line)
            })?;
            rewrite::promote(
                path,
                &branch,
                &old_head,
                &report.new_head,
                &report.temp_branch,
            )?;
            captured = Some(report);
            Ok(())
        };
        let outcome = guard::run(
            &db,
            opts.into_request(id, path.clone(), action),
            &mut || body(&path),
        )?;
        Ok((outcome, captured.expect("body 跑过就一定有结果")))
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 区间里有多少条提交。超过 `LARGE_TODO` 时界面先提示分段再让人确认：
/// 改写是每条一次进程，两百条就是两百个子进程，用户要有心理准备。
#[tauri::command]
pub async fn rewrite_plan_size(
    state: State<'_, Arc<Db>>,
    id: i64,
    base: String,
    head: String,
) -> Result<PlanSize, GitError> {
    let path = worktree(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let count = rewrite::plan(&path, &base, &head)?.len();
        Ok(PlanSize {
            count,
            large: count >= rewrite::LARGE_TODO,
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSize {
    pub count: usize,
    pub large: bool,
}

// ---------------------------------------------------------------- 暂存（§7.8）

/// 这个文件能不能行级暂存，以及不能做时的原因。界面上不给不能做的入口（§3 的期边界）。
#[tauri::command]
pub async fn file_partial_support(
    state: State<'_, Arc<Db>>,
    id: i64,
    path: String,
) -> Result<PartialSupport, GitError> {
    let repo_path = worktree(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || -> Result<PartialSupport, GitError> {
        let files = status::list(&repo_path)?;
        let file = files
            .iter()
            .find(|file| file.path == path || file.from_path.as_deref() == Some(path.as_str()))
            .ok_or_else(|| GitError::GitFailed {
                stderr: format!("这个文件已经不在待提交列表里了：{path}"),
            })?;
        Ok(status::partial_supported(&repo_path, file))
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 按界面裁出来的补丁暂存。返回最新的整份工作区状态（与整文件暂存一个口径），
/// 界面就不用再补一次读取、也不会在两次 IPC 之间显示过期的勾选态。
#[tauri::command]
pub async fn files_stage_hunks(
    state: State<'_, Arc<Db>>,
    id: i64,
    path: String,
    hunks: Vec<HunkSelection>,
) -> Result<StagedFiles, GitError> {
    let db = state.inner().clone();
    let repo_path = worktree(&db, id).await?;
    let path_for_guard = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = guard::run(
            &db,
            Request::new(id, repo_path.clone(), "files_stage_hunks")
                .affecting(Some(path_for_guard), None),
            &mut || {
                status::stage_hunks(&repo_path, &path, &hunks)?;
                Ok(())
            },
        )?;
        let files: Vec<WorkingFile> = status::list(&repo_path)?;
        Ok(StagedFiles { outcome, files })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFiles {
    #[serde(flatten)]
    pub outcome: Outcome,
    pub files: Vec<WorkingFile>,
}

// ---------------------------------------------------------------- 冲突解决器（§7.13）

/// 当前仓库全部未合并条目。一个卡片一个。
///
/// 读操作，不进 write_guard：它不改变任何东西，只是把索引里的三个 stage 摊给界面。
#[tauri::command]
pub async fn conflict_list(state: State<'_, Arc<Db>>, id: i64) -> Result<Vec<Conflict>, GitError> {
    let repo_path = worktree(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || conflict::list(&repo_path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 解决一个冲突文件。`how` 为 `ours` / `theirs` / `text`（text 带手改正文）。
///
/// 走 write_guard 的 Conflict 场：留还原点与审计，但不校验中断态、失败不回滚
/// （回滚会把用户已经解好的其他冲突一起抹掉）。
#[tauri::command]
pub async fn conflict_resolve(
    state: State<'_, Arc<Db>>,
    id: i64,
    path: String,
    how: ResolutionChoice,
) -> Result<ResolvedConflict, GitError> {
    let db = state.inner().clone();
    let repo_path = worktree(&db, id).await?;
    let file = path.clone();
    let choice = how.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = guard::run(
            &db,
            Request::new(id, repo_path.clone(), "conflict_resolve")
                .affecting(Some(file), None)
                .in_conflict_scope(),
            &mut || {
                conflict::resolve(&repo_path, &path, choice.clone().into())?;
                Ok(())
            },
        )?;
        let remaining = conflict::list(&repo_path)?;
        Ok(ResolvedConflict {
            outcome,
            remaining: remaining.len(),
            conflicts: remaining,
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 界面上「接受删除」那一条。改删/删改冲突里有一方已经把这个文件删了。
#[tauri::command]
pub async fn conflict_accept_deletion(
    state: State<'_, Arc<Db>>,
    id: i64,
    path: String,
) -> Result<ResolvedConflict, GitError> {
    let db = state.inner().clone();
    let repo_path = worktree(&db, id).await?;
    let file = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = guard::run(
            &db,
            Request::new(id, repo_path.clone(), "conflict_accept_deletion")
                .affecting(Some(file), None)
                .in_conflict_scope(),
            &mut || {
                conflict::accept_deletion(&repo_path, &path)?;
                Ok(())
            },
        )?;
        let remaining = conflict::list(&repo_path)?;
        Ok(ResolvedConflict {
            outcome,
            remaining: remaining.len(),
            conflicts: remaining,
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 全部标记完之后续跑（`--continue`）。
///
/// 仍然在 Conflict 场里：续跑要么收尾（仓库回到正常态），要么在下一个提交上再停一次。
/// 后者是正常结局，所以失败不回滚。
#[tauri::command]
pub async fn conflict_continue(
    state: State<'_, Arc<Db>>,
    id: i64,
    message: Option<String>,
) -> Result<Continued, GitError> {
    let db = state.inner().clone();
    let repo_path = worktree(&db, id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        // 中断态由前端读出来再传回来可以避免重复 spawn 一个 git，但那是界面给的，
        // 必须在 Rust 侧重新读一次——传错了不能当成真的
        let kind = crate::git::refs::interrupt(&repo_path)?.kind;
        let outcome = guard::run(
            &db,
            Request::new(id, repo_path.clone(), "conflict_continue").in_conflict_scope(),
            &mut || {
                conflict::continue_operation(&repo_path, kind, message.as_deref())?;
                Ok(())
            },
        )?;
        let state = crate::git::refs::interrupt(&repo_path)?;
        Ok(Continued {
            outcome,
            finished: state.kind == crate::git::refs::Interrupt::None,
            still_interrupted: state.kind,
            branch: state.branch,
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 界面传过来的解决方式。字符串形态而不是枚举，前端用 `call` 传参只能传 JSON 值。
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "how", rename_all = "camelCase")]
pub enum ResolutionChoice {
    Ours,
    Theirs,
    Text { text: String },
}

impl From<ResolutionChoice> for conflict::Resolution {
    fn from(choice: ResolutionChoice) -> Self {
        match choice {
            ResolutionChoice::Ours => Self::Ours,
            ResolutionChoice::Theirs => Self::Theirs,
            ResolutionChoice::Text { text } => Self::Text(text),
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedConflict {
    #[serde(flatten)]
    pub outcome: Outcome,
    /// 还剩几个没解决。界面上直接拿它画进度
    pub remaining: usize,
    pub conflicts: Vec<Conflict>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Continued {
    #[serde(flatten)]
    pub outcome: Outcome,
    /// 操作收尾了。false 时界面要接着显示剩余冲突
    pub finished: bool,
    /// 还在中断态里的话是哪一种：下一个提交也冲突了就是它
    pub still_interrupted: crate::git::refs::Interrupt,
    pub branch: Option<String>,
}

// ---------------------------------------------------------------- 操作日志与撤销（§7.17）

#[tauri::command]
pub async fn write_journal(
    state: State<'_, Arc<Db>>,
    id: i64,
    limit: Option<usize>,
) -> Result<Vec<WriteOpView>, GitError> {
    let conn = crate::store::db::lock(&state.inner().conn);
    let entries = journal::recent(&conn, id, limit.unwrap_or(50).clamp(1, 200))?;
    Ok(entries.into_iter().map(WriteOpView::from).collect())
}

/// 仓库里现有的还原点。界面上用它回答"怎么回到那一次操作之前"。
#[tauri::command]
pub async fn write_backups(state: State<'_, Arc<Db>>, id: i64) -> Result<Vec<Backup>, GitError> {
    let repo_path = worktree(state.inner(), id).await?;
    tauri::async_runtime::spawn_blocking(move || backup::list(&repo_path))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 某个还原 ref 现在指向哪里。界面上给"找回改写前的状态"用：ref 可能已被 gc 或手动清掉，
/// 那时返回 None 而不是报错——用户看到的是一个失效的还原点，不是一次失败。
#[tauri::command]
pub async fn write_backup_target(
    state: State<'_, Arc<Db>>,
    id: i64,
    reference: String,
) -> Result<Option<String>, GitError> {
    let repo_path = worktree(state.inner(), id).await?;
    // 引用名要进 `rev-parse` 的参数位，只放行本工具自己造出来的形态
    let reference = backup::check_reference(&reference)?;
    tauri::async_runtime::spawn_blocking(move || backup::target(&repo_path, &reference))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 撤销上一步。判据在 `guard::undo_last`：HEAD 不等于日志里的 `head_after` 就拒绝。
#[tauri::command]
pub async fn write_undo(state: State<'_, Arc<Db>>, id: i64) -> Result<guard::UndoReport, GitError> {
    let db = state.inner().clone();
    let repo_path = worktree(&db, id).await?;
    let head = guard::head_sha(&repo_path)?.ok_or(GitError::NotClean {
        detail: "这个仓库还没有提交，没什么可撤销的".into(),
    })?;
    tauri::async_runtime::spawn_blocking(move || guard::undo_last(&db, id, repo_path, head))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 日志条目给界面用的形态。`status` 翻成中文，别让前端各处自己拼。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOpView {
    pub id: i64,
    pub repo_id: i64,
    pub ts: i64,
    pub action: String,
    pub affected_from: Option<String>,
    pub affected_to: Option<String>,
    pub backup_ref: String,
    pub head_before: Option<String>,
    pub head_after: Option<String>,
    pub status: &'static str,
    pub status_label: &'static str,
    pub detail: Option<String>,
}

impl From<WriteOp> for WriteOpView {
    fn from(entry: WriteOp) -> WriteOpView {
        let (status, label) = match entry.status {
            journal::Status::Ok => ("ok", "成功"),
            journal::Status::RolledBack => ("rolled_back", "失败已回滚"),
            journal::Status::Interrupted => ("interrupted", "中断待处理"),
        };
        WriteOpView {
            id: entry.id,
            repo_id: entry.repo_id,
            ts: entry.ts,
            action: entry.action,
            affected_from: entry.affected_from,
            affected_to: entry.affected_to,
            backup_ref: entry.backup_ref,
            head_before: entry.head_before,
            head_after: entry.head_after,
            status,
            status_label: label,
            detail: entry.detail,
        }
    }
}

fn emit(app: &AppHandle, repo_id: i64, action: &str, line: &str) {
    let _ = app.emit(
        SYNC_PROGRESS_EVENT,
        SyncProgress {
            repo_id,
            action: action.to_string(),
            line: line.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_journal_status_has_its_own_chinese_label() {
        // 界面各处显示的是这三个词之一，不能有一处落空
        let views = [
            WriteOpView::from(sample(journal::Status::Ok)),
            WriteOpView::from(sample(journal::Status::RolledBack)),
            WriteOpView::from(sample(journal::Status::Interrupted)),
        ];
        let labels: Vec<&str> = views.iter().map(|view| view.status_label).collect();
        assert_eq!(labels, vec!["成功", "失败已回滚", "中断待处理"]);
        let codes: Vec<&str> = views.iter().map(|view| view.status).collect();
        assert_eq!(codes, vec!["ok", "rolled_back", "interrupted"]);
    }

    /// 只有"要求干净"的那几类操作才需要工作区干净：
    /// 暂存与 stash 的前提恰恰是工作区脏
    #[test]
    fn only_operations_that_move_head_require_a_clean_worktree() {
        let light = Opts::light();
        assert!(!light.clean, "暂存/stash 不该要求干净");
        assert!(Opts::clean().clean, "切换/reset 要求干净");
        assert!(Opts::rewriting().verifying_tree, "改写类必须做 tree 校验");
    }

    fn sample(status: journal::Status) -> WriteOp {
        WriteOp {
            id: 1,
            repo_id: 1,
            ts: 0,
            action: "branch_delete".into(),
            affected_from: None,
            affected_to: None,
            backup_ref: "refs/git-tidy/backup-1".into(),
            head_before: None,
            head_after: None,
            status,
            detail: None,
        }
    }
}
