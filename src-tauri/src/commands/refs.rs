use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::error::GitError;
use crate::git::refs::{self, Ref, RepoState};
use crate::store::db::{query, Db};
use crate::store::repos;

/// 一次扫描的两样结果，合成一条命令返回：引用表已经读过一遍 refs，
/// 状态摘要的 ahead/behind 就从这份表里取当前分支那一行，不用再起进程（§7.3）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub refs: Vec<Ref>,
    pub state: RepoState,
}

/// 扫描引用与仓库状态。只接受注册仓库的 id（§7.1）。
///
/// browse（treeless 只读浏览）仓库可以扫：refs、HEAD 与标记文件在 treeless 克隆里
/// 都齐全，一个提交对象都不必下载。
#[tauri::command]
pub async fn refs_scan(state: State<'_, Arc<Db>>, id: i64) -> Result<Scan, GitError> {
    let (path, _kind) = query(state.inner().clone(), move |conn| repos::locate(conn, id)).await?;

    tauri::async_runtime::spawn_blocking(move || -> Result<Scan, GitError> {
        let refs = refs::list(&path)?;
        let summary = refs::state(&path, &refs)?;
        Ok(Scan {
            refs,
            state: summary,
        })
    })
    .await
    .map_err(|err| GitError::Internal(err.to_string()))?
}
