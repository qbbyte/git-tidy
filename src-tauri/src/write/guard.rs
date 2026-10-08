use std::path::Path;
use std::sync::Arc;

use crate::error::GitError;
use crate::git::refs as refscan;
use crate::store::db::{lock, Db};
use crate::write::{backup, journal};

/// 一次写操作的描述。写命令把它交给 `run`，六步由 `run` 固定顺序执行（§4）。
///
/// 前端只传 id 和参数，路径由注册表解析——所以这个结构体里的 `path` 永远不是界面给的。
pub struct Request {
    pub repo_id: i64,
    pub path: std::path::PathBuf,
    /// 日志里的动作名。用命令名（如 `branch_delete`），界面上照它分组。
    pub action: &'static str,
    /// 界面加载时看到的 HEAD。给了就做乐观并发校验：IDE 可能正在同一个仓库里写。
    pub expected_head: Option<String>,
    /// 需要工作区干净（切换分支、reset --hard、改写类）。stash 与暂存类不要求。
    pub require_clean: bool,
    /// 改写类操作（reset / cherry-pick / revert / 改写）：执行后比对 tree 必须一致。
    pub verify_tree: bool,
    /// 受影响的区间，写进日志给界面显示。分支/tag 这类写作用起点。
    pub affected_from: Option<String>,
    pub affected_to: Option<String>,
}

impl Request {
    pub fn new(repo_id: i64, path: std::path::PathBuf, action: &'static str) -> Request {
        Request {
            repo_id,
            path,
            action,
            expected_head: None,
            require_clean: false,
            verify_tree: false,
            affected_from: None,
            affected_to: None,
        }
    }

    pub fn expecting_head(mut self, head: Option<String>) -> Request {
        self.expected_head = head;
        self
    }

    pub fn clean(mut self) -> Request {
        self.require_clean = true;
        self
    }

    pub fn verifying_tree(mut self) -> Request {
        self.verify_tree = true;
        self
    }

    pub fn affecting(mut self, from: Option<String>, to: Option<String>) -> Request {
        self.affected_from = from;
        self.affected_to = to;
        self
    }
}

/// 写完之后的报量。命令层把它原样序列化给界面。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub action: String,
    /// 还原 ref。界面上要一直显示它：这是"怎么回去"的答案
    pub backup_ref: String,
    pub head_before: Option<String>,
    pub head_after: Option<String>,
    pub journal_id: i64,
}

/// 执行体。拿到还原 ref 之后才真正动手。
///
/// 约定：失败时**不要自己回滚**，把错误交回 `run`——回滚与日志只能有一个地方做，
/// 两处各做一次就会出现"日志说成功、仓库已回滚"这种最难查的状态。
pub type Body<'a> = &'a mut dyn FnMut() -> Result<(), GitError>;

/// 所有写命令的唯一入口。六步固定顺序，缺任一步即拒绝（§4）。
///
/// 1. **前置校验**：工作区状态、非中断态、HEAD 与界面加载时一致、ref 是否已被远程包含（由调用方按需加）。
/// 2. **还原点** + 落一条日志。
/// 3. **执行**：调用方的命令体。
/// 4. **结果校验**：改写类比对 `oldHead^{tree}` 与新 HEAD 的 tree。
/// 5. **失败回滚**：HEAD 回到 `head_before`，日志记 `rolled_back`。
/// 6. **审计**：成功也记。
pub fn run(db: &Arc<Db>, request: Request, body: Body<'_>) -> Result<Outcome, GitError> {
    let path = request.path.clone();
    precheck(&request)?;

    let head_before = head_sha(&path)?;
    let branch = current_branch(&path)?;
    let head_before_tree = if request.verify_tree {
        tree_of(&path, head_before.as_deref())?
    } else {
        None
    };

    // 步骤 2：还原点 + 日志。空仓库没有 HEAD 可指，还原点就退化成"这个仓库还没有提交"
    let backup_ref = backup::create(&path, head_before.as_deref().unwrap_or(ZERO_OID))?;
    let journal_id = {
        let conn = lock(&db.conn);
        journal::start(
            &conn,
            request.repo_id,
            request.action,
            request.affected_from.as_deref(),
            request.affected_to.as_deref(),
            &backup_ref,
            head_before.as_deref(),
        )?
    };

    let result = body();
    let head_after = head_sha(&path)?;

    match result {
        Ok(()) => {
            // 步骤 4：改写类的 tree 校验。这一步失败与命令本身失败同权——不回滚就是"改了但说不清"
            if let Some(before) = head_before_tree.as_deref() {
                let after = tree_of(&path, head_after.as_deref())?;
                if after.as_deref() != Some(before) {
                    let detail = format!(
                        "tree 校验失败：{} ≠ {}",
                        before,
                        after.as_deref().unwrap_or("空")
                    );
                    rollback(&path, head_before.as_deref(), branch.as_deref())?;
                    let conn = lock(&db.conn);
                    journal::finish(
                        &conn,
                        journal_id,
                        journal::Status::RolledBack,
                        head_after.as_deref(),
                        Some(&detail),
                    )?;
                    return Err(GitError::VerificationFailed { detail });
                }
            }

            // 步骤 6：成功也要记
            let conn = lock(&db.conn);
            journal::finish(
                &conn,
                journal_id,
                journal::Status::Ok,
                head_after.as_deref(),
                None,
            )?;
            Ok(Outcome {
                action: request.action.to_string(),
                backup_ref,
                head_before,
                head_after,
                journal_id,
            })
        }
        Err(err) => {
            // 步骤 5：失败回滚。冲突这类"半完成序列"不回滚——它停在中断态上，
            // 回滚反而会把用户已经解好的冲突抹掉；这种情况留给 M3 的解决器续跑
            let interrupted = matches!(&err, GitError::OperationInProgress { .. });
            let status = if interrupted {
                journal::Status::Interrupted
            } else {
                rollback(&path, head_before.as_deref(), branch.as_deref())?;
                journal::Status::RolledBack
            };
            let head_now = head_sha(&path)?;
            let conn = lock(&db.conn);
            journal::finish(
                &conn,
                journal_id,
                status,
                head_now.as_deref(),
                Some(&format!("{err:?}")),
            )?;
            Err(err)
        }
    }
}

/// 撤销上一步（§7.17）。
///
/// 判据是**当前 HEAD 必须等于日志里的 `head_after`**：不等说明中间有人动过
/// （终端里手动提交、IDE 提交、另一个实例），这时自动撤销会把别人的操作一起抹掉，
/// 所以改成把还原命令给用户，让他自己判断。
pub fn undo_last(
    db: &Arc<Db>,
    repo_id: i64,
    path: std::path::PathBuf,
    current_head: String,
) -> Result<UndoReport, GitError> {
    let conn = lock(&db.conn);
    let Some(entry) = journal::last(&conn, repo_id)? else {
        return Err(GitError::NothingToUndo);
    };
    drop(conn);

    if entry.status != journal::Status::Ok {
        return Err(GitError::NothingToUndo);
    }
    let Some(head_after) = entry.head_after.as_deref() else {
        // 记录里没有执行后 HEAD（clone/fetch 这类不改 HEAD 的写操作）无从判据，直接拒
        return Err(GitError::NothingToUndo);
    };
    if head_after != current_head {
        return Err(GitError::HeadMoved {
            expected: head_after.to_string(),
            actual: current_head,
        });
    }
    let Some(head_before) = entry.head_before.as_deref() else {
        return Err(GitError::NothingToUndo);
    };
    // 工作区不干净时移动 HEAD 会丢改动：宁可不动
    if !crate::git::status::list(&path)?.is_empty() {
        return Err(GitError::NotClean {
            detail: "工作区还有未提交改动，撤销会覆盖它们".into(),
        });
    }

    let report = UndoReport {
        action: entry.action.clone(),
        backup_ref: entry.backup_ref.clone(),
        head_before: head_before.to_string(),
        head_after: head_after.to_string(),
    };
    crate::git::process::run(
        Some(&path),
        &["reset", "--hard", head_before],
    )?
    .expect_success()?;

    // 还原点完成使命后删掉；但先复制一份到报告里，用户还能从日志里读到它
    let _ = backup::drop(&path, &entry.backup_ref);
    let conn = lock(&db.conn);
    journal::finish(
        &conn,
        entry.id,
        journal::Status::RolledBack,
        Some(head_before),
        Some("已按用户请求撤销"),
    )?;
    Ok(report)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReport {
    pub action: String,
    pub backup_ref: String,
    pub head_before: String,
    pub head_after: String,
}

/// 步骤 1 的前置校验。三件事都在这里挡掉：中断态、脏工作区、HEAD 被别人挪了。
fn precheck(request: &Request) -> Result<(), GitError> {
    let state = refscan::interrupt(&request.path)?;
    if state.kind != refscan::Interrupt::None {
        return Err(GitError::OperationInProgress { state: state.kind });
    }
    if request.require_clean {
        let dirty = crate::git::status::list(&request.path)?;
        if !dirty.is_empty() {
            return Err(GitError::NotClean {
                detail: format!("工作区还有 {} 处未提交改动", dirty.len()),
            });
        }
    }
    if let Some(expected) = request.expected_head.as_deref() {
        let actual = head_sha(&request.path)?.unwrap_or_default();
        if actual != expected {
            return Err(GitError::HeadMoved {
                expected: expected.to_string(),
                actual,
            });
        }
    }
    Ok(())
}

/// 失败回滚：把当前分支指回 `head_before`，并把工作区与索引一起复位。
///
/// 只在 HEAD 真的动了的时候才动手：暂存、stash 这类不改 HEAD 的写操作回滚等于白做一遍，
/// 而且会把用户在这期间做的新改动一起清掉。
///
/// 分支也要切回去：cherry-pick / revert 一类命令可能停在另一个分支上，只挪分支指针
/// 的话用户会发现自己还站在一个半路上（而且那个分支上留着这次操作的痕迹）。
fn rollback(
    path: &Path,
    head_before: Option<&str>,
    branch: Option<&str>,
) -> Result<(), GitError> {
    let Some(head_before) = head_before else {
        // 空仓库上的写操作：没有提交可退，退回"清掉索引"就够了
        crate::git::process::run(Some(path), &["reset", "-q"])?;
        return Ok(());
    };
    let now = head_sha(path)?;
    if now.as_deref() == Some(head_before) && current_branch(path)?.as_deref() == branch {
        return Ok(());
    }
    match branch {
        Some(branch) => {
            // 先把原分支的指针挪回去，再强制切回去：顺序反了会 checkout 到一个已被改动的分支上
            let _ = crate::git::process::run(
                Some(path),
                &["update-ref", &format!("refs/heads/{branch}"), head_before],
            );
            crate::git::process::run(
                Some(path),
                &["checkout", "--force", "-q", branch],
            )?
            .expect_success()?;
            crate::git::process::run(Some(path), &["reset", "--hard", head_before])?
                .expect_success()?;
            Ok(())
        }
        // 游离 HEAD：没有分支可挪，只能挪 HEAD 本身
        None => {
            crate::git::process::run(Some(path), &["reset", "--hard", head_before])?
                .expect_success()?;
            Ok(())
        }
    }
}

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

pub fn head_sha(path: &Path) -> Result<Option<String>, GitError> {
    crate::git::graph::head_sha(path)
}

pub fn tree_of(path: &Path, head: Option<&str>) -> Result<Option<String>, GitError> {
    let Some(head) = head else {
        return Ok(None);
    };
    let out = crate::git::process::run(Some(path), &["rev-parse", "-q", "--verify", &format!("{head}^{{tree}}")])?;
    if !out.success {
        return Ok(None);
    }
    Ok(Some(out.stdout.trim().to_string()))
}

/// 当前分支。游离 HEAD 时是 None——回滚那时只能挪 HEAD 本身。
pub fn current_branch(path: &Path) -> Result<Option<String>, GitError> {
    let out = crate::git::process::run(Some(path), &["symbolic-ref", "-q", "--short", "HEAD"])?;
    if !out.success {
        return Ok(None);
    }
    let branch = out.stdout.trim();
    Ok((!branch.is_empty()).then(|| branch.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::process;
    use crate::store::db::Db;

    fn git_in(dir: &Path, args: &[&str]) {
        let out = process::run(Some(dir), args).expect("spawn git");
        assert!(out.success, "git {args:?} 失败：{}", out.stderr);
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git_in(dir.path(), &["init", "-q", "-b", "main", "."]);
        git_in(dir.path(), &["config", "user.name", "t"]);
        git_in(dir.path(), &["config", "user.email", "t@example.com"]);
        git_in(dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(dir.path().join("a.txt"), "1\n").expect("write");
        git_in(dir.path(), &["add", "-A"]);
        git_in(dir.path(), &["commit", "-q", "-m", "feat: a"]);
        dir
    }

    fn db() -> (tempfile::TempDir, Arc<Db>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(Db::open(dir.path()).expect("db"));
        (dir, db)
    }

    fn request(dir: &tempfile::TempDir, action: &'static str) -> Request {
        Request::new(1, dir.path().to_path_buf(), action)
    }

    /// 改一行再提交：`-am` 前提是工作区真有改动，否则 git 直接拒绝
    fn commit_touching_a(dir: &Path, content: &str, message: &str) {
        std::fs::write(dir.join("a.txt"), content).expect("write");
        git_in(dir, &["commit", "-qam", message]);
    }

    /// 六步的主路径：成功要留下还原点与一条 ok 日志
    #[test]
    fn a_successful_write_leaves_a_backup_and_an_audit_row() {
        let dir = repo();
        let (_tmp, db) = db();

        let outcome = {
            let mut body = || {
                commit_touching_a(dir.path(), "2\n", "feat: 第二次");
                Ok(())
            };
            run(&db, request(&dir, "commit"), &mut body).expect("写操作")
        };

        assert_ne!(outcome.head_before, outcome.head_after);
        assert_eq!(
            backup::target(dir.path(), &outcome.backup_ref).expect("target"),
            outcome.head_before,
            "还原点必须指回执行前的 HEAD"
        );
        let entry = journal::last(&lock(&db.conn), 1).expect("last").expect("有一条");
        assert_eq!(entry.status, journal::Status::Ok);
        assert_eq!(entry.backup_ref, outcome.backup_ref);
    }

    /// 步骤 5：命令失败要把 HEAD 拉回去，并如实记 rolled_back
    #[test]
    fn a_failing_write_rolls_the_head_back() {
        let dir = repo();
        let (_tmp, db) = db();
        let before = head_sha(dir.path()).expect("head").expect("有提交");

        let result = {
            let mut body = || {
                git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
                commit_touching_a(dir.path(), "3\n", "feat: 写了又失败");
                Err(GitError::GitFailed {
                    stderr: "故意失败".into(),
                })
            };
            run(&db, request(&dir, "branch_delete"), &mut body)
        };

        assert!(result.is_err());
        assert_eq!(head_sha(dir.path()).expect("head").as_deref(), Some(before.as_str()));
        let current = current_branch(dir.path()).expect("branch").expect("在某个分支上");
        assert_eq!(current, "main", "回滚要回到执行前的分支，不是留在新建的那个上");
        let entry = journal::last(&lock(&db.conn), 1).expect("last").expect("有一条");
        assert_eq!(entry.status, journal::Status::RolledBack);
        assert!(entry.detail.unwrap().contains("故意失败"));
    }

    /// 步骤 4：tree 校验不过就是"改了但说不清"，与命令失败同权
    #[test]
    fn a_rewrite_that_changes_the_tree_is_rejected() {
        let dir = repo();
        let (_tmp, db) = db();
        let before = head_sha(dir.path()).expect("head").expect("有提交");

        let result = {
            let mut body = || {
                // 换掉文件内容：提交历史变了，但 tree 也变了——改写类操作不允许
                commit_touching_a(dir.path(), "2\n", "feat: 偷改内容");
                Ok(())
            };
            run(&db, request(&dir, "reset").verifying_tree(), &mut body)
        };

        let err = result.expect_err("tree 变了就该被拒");
        assert!(matches!(err, GitError::VerificationFailed { .. }), "{err:?}");
        assert_eq!(
            head_sha(dir.path()).expect("head").as_deref(),
            Some(before.as_str()),
            "被拒之后必须回到原状态"
        );
    }

    /// tree 一致的改写（改的是提交信息）就该放行
    #[test]
    fn a_rewrite_that_keeps_the_tree_goes_through() {
        let dir = repo();
        let (_tmp, db) = db();

        let outcome = {
            let mut body = || {
                git_in(dir.path(), &["commit", "-q", "--amend", "-m", "refactor: 改个标题"]);
                Ok(())
            };
            run(&db, request(&dir, "reword").verifying_tree(), &mut body).expect("tree 一致就该过")
        };
        assert_ne!(outcome.head_before, outcome.head_after);
    }

    /// 步骤 1：HEAD 与界面加载时不一致就拒（乐观并发，IDE 可能正在写）
    #[test]
    fn a_moved_head_is_refused_before_anything_happens() {
        let dir = repo();
        let (_tmp, db) = db();
        let stale = "0".repeat(40);

        let mut body = || Ok(());
        let err = run(
            &db,
            request(&dir, "reset").expecting_head(Some(stale.clone())),
            &mut body,
        )
        .expect_err("HEAD 不一致就该拒");

        assert!(matches!(err, GitError::HeadMoved { .. }), "{err:?}");
        assert!(
            journal::last(&lock(&db.conn), 1).expect("last").is_none(),
            "被前置校验挡下的写操作不该留还原点或日志"
        );
    }

    /// 步骤 1：脏工作区时要求干净的操作要被拒
    #[test]
    fn a_dirty_worktree_blocks_the_operations_that_need_it() {
        let dir = repo();
        let (_tmp, db) = db();
        std::fs::write(dir.path().join("a.txt"), "还没提交\n").expect("write");

        let mut body = || Ok(());
        let err = run(&db, request(&dir, "branch_checkout").clean(), &mut body)
            .expect_err("工作区脏就该拒");
        assert!(matches!(err, GitError::NotClean { .. }), "{err:?}");
    }

    /// 暂存类操作不要求工作区干净——工作区脏正是它存在的理由
    #[test]
    fn staging_operations_do_not_require_a_clean_worktree() {
        let dir = repo();
        let (_tmp, db) = db();
        std::fs::write(dir.path().join("a.txt"), "改了\n").expect("write");

        let outcome = {
            let mut body = || {
                git_in(dir.path(), &["add", "-A"]);
                Ok(())
            };
            run(&db, request(&dir, "files_stage"), &mut body).expect("暂存不该被脏工作区挡住")
        };
        assert_eq!(
            outcome.head_before, outcome.head_after,
            "暂存不动 HEAD，日志里也要如实记成没动"
        );
    }

    /// 撤销的判据：HEAD 不等于日志里的 head_after 就拒绝自动撤销
    #[test]
    fn undo_needs_the_head_to_be_where_the_log_says() {
        let dir = repo();
        let (_tmp, db) = db();

        let outcome = {
            let mut body = || {
                commit_touching_a(dir.path(), "2\n", "feat: 第二次");
                std::fs::write(dir.path().join("a.txt"), "3\n").expect("write");
                commit_touching_a(dir.path(), "3\n", "feat: 第三次");
                Ok(())
            };
            run(&db, request(&dir, "commit"), &mut body).expect("写操作")
        };

        // 中间有人（终端/IDE）又提交了一次
        commit_touching_a(dir.path(), "别人改的\n", "feat: 别人提交的");
        let now = head_sha(dir.path()).expect("head").expect("有提交");

        let err = undo_last(&db, 1, dir.path().to_path_buf(), now.clone()).expect_err("HEAD 变了就该拒");
        assert!(matches!(err, GitError::HeadMoved { .. }), "{err:?}");
        assert_eq!(
            head_sha(dir.path()).expect("head").as_deref(),
            Some(now.as_str()),
            "拒绝之后仓库要一动不动"
        );
        let _ = outcome;
    }

    #[test]
    fn undo_moves_the_head_back_and_drops_the_backup_ref() {
        let dir = repo();
        let (_tmp, db) = db();
        let before = head_sha(dir.path()).expect("head").expect("有提交");

        let outcome = {
            let mut body = || {
                commit_touching_a(dir.path(), "2\n", "feat: 第二次");
                Ok(())
            };
            run(&db, request(&dir, "commit"), &mut body).expect("写操作")
        };

        let report = undo_last(
            &db,
            1,
            dir.path().to_path_buf(),
            outcome.head_after.clone().expect("有执行后 HEAD"),
        )
        .expect("撤销");

        assert_eq!(report.head_before, before);
        assert_eq!(
            head_sha(dir.path()).expect("head").as_deref(),
            Some(before.as_str()),
            "撤销后 HEAD 要回到执行前"
        );
        assert!(
            backup::list(dir.path()).expect("list").is_empty(),
            "撤销完就把还原点清掉，否则 refs/git-tidy/ 会越积越多"
        );
    }

    #[test]
    fn undo_refuses_a_dirty_worktree_instead_of_discarding_changes() {
        let dir = repo();
        let (_tmp, db) = db();

        let outcome = {
            let mut body = || {
                commit_touching_a(dir.path(), "2\n", "feat: 第二次");
                Ok(())
            };
            run(&db, request(&dir, "commit"), &mut body).expect("写操作")
        };
        std::fs::write(dir.path().join("a.txt"), "还没提交\n").expect("write");

        let err = undo_last(
            &db,
            1,
            dir.path().to_path_buf(),
            outcome.head_after.clone().expect("有执行后 HEAD"),
        )
        .expect_err("工作区脏就该拒");
        assert!(matches!(err, GitError::NotClean { .. }), "{err:?}");
    }

    #[test]
    fn nothing_to_undo_is_its_own_answer_not_an_error_crash() {
        let dir = repo();
        let (_tmp, db) = db();
        let head = head_sha(dir.path()).expect("head").expect("有提交");
        let err = undo_last(&db, 42, dir.path().to_path_buf(), head).expect_err("没有可撤销的");
        assert!(matches!(err, GitError::NothingToUndo), "{err:?}");
    }
}