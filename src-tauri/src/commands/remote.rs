use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::GitError;
use crate::git::clone::{self, Progress};
use crate::git::repo;
use crate::store::db::{query, Db};
use crate::store::repos::{self, Repo, RepoKind};

/// 克隆与补齐过程中的进度事件。参数里没有路径，前端只用 url 认领属于自己的那条进度。
pub const PROGRESS_EVENT: &str = "repo-progress";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    url: String,
    phase: String,
    percent: u8,
}

fn emit(app: &AppHandle, url: &str, progress: &Progress) {
    let payload = ProgressPayload {
        url: url.to_string(),
        phase: progress.phase.clone(),
        percent: progress.percent,
    };
    // 发不出去只是丢一格进度，不该让整次克隆失败
    let _ = app.emit(PROGRESS_EVENT, payload);
}

/// 克隆目录的根：`<app data>/remote`。放在应用数据目录里，用户不需要自己选路径，
/// 界面传进来的地址也就永远不会被解释成磁盘路径（§7.1）。
fn remote_root(app: &AppHandle) -> Result<PathBuf, GitError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|err| GitError::Internal(format!("拿不到应用数据目录：{err}")))?;
    let root = data_dir.join("remote");
    std::fs::create_dir_all(&root)
        .map_err(|err| GitError::Internal(format!("无法创建克隆目录 {}：{err}", root.display())))?;
    Ok(root)
}

/// 目录已是 Git 仓库时，确认它确实来自用户要的这个地址。
/// treeless 克隆不保存原始 URL，只能拿 origin 比对；对不上就报错——
/// 把别人的仓库当用户要的那个登记进列表，比克隆失败更难排查。
fn check_adoptable(dest: &Path, url: &str) -> Result<repo::RepoInfo, GitError> {
    let stored = clone::remote_url(dest);
    if stored.as_deref() != Some(url) {
        return Err(GitError::Internal(format!(
            "{} 已存在，但它的远程地址是 {stored:?}，与要添加的地址不一致。请换个地址，或先手动删除该目录",
            dest.display()
        )));
    }
    let info = repo::probe(dest)?;
    if info.head_commit.is_none() {
        return Err(GitError::Internal(format!(
            "{} 里读不到 HEAD 提交，可能是上次克隆中断留下的目录，请删除后重试",
            dest.display()
        )));
    }
    Ok(info)
}

/// pick_dest 的结果：目录位置，以及它是否已经是这个地址的仓库。
struct Dest {
    path: PathBuf,
    /// true = 目录已经是该地址的仓库，不用重新下载
    adopt: bool,
}

/// 给这个地址挑一个克隆目录。同名不同仓库（比如两个 fork 都叫 app）不能互相覆盖，
/// 所以被别的地址占用的名字往后加 `-2`、`-3`；同一地址复用原目录，不重复下载。
fn pick_dest(root: &Path, base: &str, url: &str) -> Result<Dest, GitError> {
    for i in 0..100u32 {
        let name = if i == 0 {
            base.to_string()
        } else {
            // 让位从 -2 开始：`app-1` 容易和一个真叫 app-1 的仓库混淆
            format!("{base}-{}", i + 1)
        };
        let candidate = root.join(name);
        match clone::remote_url(&candidate) {
            Some(existing) if existing == url => {
                return Ok(Dest {
                    path: candidate,
                    adopt: true,
                })
            }
            // 别的仓库占了这个名字，往后找空位
            Some(_) => continue,
            // 目录不存在，或存在但不是仓库（残缺克隆）——两种都由调用方分辨处理
            None => {
                return Ok(Dest {
                    path: candidate,
                    adopt: false,
                })
            }
        }
    }
    Err(GitError::Internal(format!(
        "{base} 及其编号变体在克隆目录里都被占用了，请给仓库改个名字再添加"
    )))
}

/// 贴地址添加仓库：只下 commit 对象（treeless），没有工作区，因此只读。
/// 只读的浏览类操作对它开放，工作区级操作在 Rust 侧就被拒绝（§7.5）。
#[tauri::command]
pub async fn repo_add_remote(
    app: AppHandle,
    state: State<'_, Arc<Db>>,
    url: String,
) -> Result<Repo, GitError> {
    clone::validate_url(&url)?;
    let url = url.trim().to_string();
    let root = remote_root(&app)?;
    let dest = pick_dest(&root, &clone::dest_name(&url), &url)?;

    // 地址一致 = 上次克隆成功过：直接登记，不再下载
    let info = if dest.adopt {
        check_adoptable(&dest.path, &url)?
    } else {
        if dest.path.exists() {
            return Err(GitError::Internal(format!(
                "{} 已存在但不是 Git 仓库，请先删除该目录再重试",
                dest.path.display()
            )));
        }
        let app_for_clone = app.clone();
        let url_for_clone = url.clone();
        let dest_for_clone = dest.path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            clone::clone_browse(&url_for_clone, &dest_for_clone, |progress| {
                emit(&app_for_clone, &url_for_clone, &progress)
            })
        })
        .await
        .map_err(|err| GitError::Internal(err.to_string()))??;
        repo::probe(&dest.path)?
    };

    let canonical = info.work_tree.to_string_lossy().into_owned();
    query(state.inner().clone(), move |conn| {
        repos::add(conn, &canonical, RepoKind::Browse, Some(&url))
    })
    .await
}

/// 克隆补齐成完整本地仓库：解 filter → refetch → 建工作区 → 离线校验对象齐全。
/// 全通过才把注册表里的 kind 改成 worktree；中途失败时保持 browse，
/// 用户面对的还是一个能读（只是不能写）的仓库，而不是一个坏掉的目录。
#[tauri::command]
pub async fn repo_materialize(
    app: AppHandle,
    state: State<'_, Arc<Db>>,
    id: i64,
) -> Result<Repo, GitError> {
    let registered = query(state.inner().clone(), move |conn| repos::get(conn, id)).await?;
    if registered.kind == RepoKind::Worktree {
        // 幂等：按钮此时不该出现，但命令被重复调用也不该有副作用
        return Ok(registered);
    }

    let path = PathBuf::from(&registered.path);
    let branch = repo::probe(&path)?
        .branch
        .ok_or_else(|| GitError::GitFailed {
            stderr: "这个仓库当前是游离 HEAD，先切到分支再补齐".into(),
        })?;

    let url = registered.remote_url.clone().unwrap_or_default();
    let app_for_clone = app.clone();
    let url_for_clone = url.clone();
    tauri::async_runtime::spawn_blocking(move || {
        clone::to_worktree(&path, &branch, |progress| {
            emit(&app_for_clone, &url_for_clone, &progress)
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))??;

    query(state.inner().clone(), move |conn| {
        repos::set_kind(conn, id, RepoKind::Worktree)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_with_origin(root: &Path, name: &str, url: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("mkdir");
        for args in [
            &["init", "-q", "."][..],
            &["remote", "add", "origin", url][..],
        ] {
            let out = crate::git::process::run(Some(&dir), args).expect("spawn git");
            assert!(out.success, "git {args:?} 失败：{}", out.stderr);
        }
        dir
    }

    #[test]
    fn the_same_url_reuses_its_existing_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let url = "https://github.com/a/app.git";
        repo_with_origin(root, "app", url);

        let dest = pick_dest(root, "app", url).expect("pick");
        assert_eq!(dest.path, root.join("app"));
        assert!(dest.adopt, "同一地址不该再下一遍");
    }

    /// 两个 fork 都叫 app 是常见情况：不能覆盖别人的目录，也不能把它当成本次要加的仓库
    #[test]
    fn a_name_held_by_another_remote_moves_to_a_numbered_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        repo_with_origin(root, "app", "https://github.com/a/app.git");

        let other = "https://github.com/b/app.git";
        let dest = pick_dest(root, "app", other).expect("pick");
        assert_eq!(dest.path, root.join("app-2"), "被占用的名字要往后让");
        assert!(!dest.adopt, "app-2 还不存在，必须真的克隆");
    }

    #[test]
    fn an_existing_non_repo_directory_is_not_adopted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("app")).expect("mkdir");

        let dest = pick_dest(root, "app", "https://github.com/a/app.git").expect("pick");
        assert!(!dest.adopt, "空目录不是仓库，不能走复用分支");
        assert!(dest.path.exists(), "克隆前的残缺目录必须被上层看见并拒绝");
    }
}
