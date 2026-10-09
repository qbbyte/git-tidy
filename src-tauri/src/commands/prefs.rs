use std::sync::Arc;

use tauri::State;

use crate::error::GitError;
use crate::store::prefs::{Preferences, Prefs};

/// 读个人偏好（需求 6.7 的第二层）。纯读，界面每次进设置页都能调。
#[tauri::command]
pub async fn prefs_get(state: State<'_, Arc<Prefs>>) -> Result<Preferences, GitError> {
    Ok(state.inner().get())
}

/// 整份覆盖并落盘。返回的是**实际存进去的值**（含夹取后的窗口尺寸），
/// 界面据此显示真值，而不是把用户填的数再显示一遍。
#[tauri::command]
pub async fn prefs_update(
    state: State<'_, Arc<Prefs>>,
    next: Preferences,
) -> Result<Preferences, GitError> {
    let prefs = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || prefs.replace(next))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 恢复默认。同样走一次覆盖，好让内存与磁盘保持一致。
#[tauri::command]
pub async fn prefs_reset(state: State<'_, Arc<Prefs>>) -> Result<Preferences, GitError> {
    let prefs = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || prefs.replace(Preferences::default()))
        .await
        .map_err(|err| GitError::Internal(err.to_string()))?
}

/// 设置文件的位置。设置页要把它显示出来：用户得知道自己在改哪个文件。
#[tauri::command]
pub async fn prefs_path(state: State<'_, Arc<Prefs>>) -> Result<String, GitError> {
    Ok(state.inner().path().to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_through_the_store_updates_both_memory_and_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prefs = Prefs::load(dir.path());
        let mut next = prefs.get();
        next.auto_update = false;
        prefs.replace(next).expect("write");

        assert!(!prefs.get().auto_update, "内存立刻生效");
        let reopened = Prefs::load(dir.path());
        assert!(!reopened.get().auto_update, "重启后也还在");
    }
}
