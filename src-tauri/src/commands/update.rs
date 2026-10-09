use std::sync::Arc;

use tauri::State;

use crate::error::GitError;
use crate::store::prefs::Prefs;
use crate::update::{self, CheckResult};

/// 判定一次「有没有新版本」（需求 7.24 的检查部分）。
///
/// **请求在前端发**：入参是 GitHub Releases 那个 URL 拿回来的原文，Rust 侧只负责解析与比较。
/// 这样版本号比较（最容易写错又最难看出错的地方）能被单元测试守住，
/// 而 Rust 侧不必为一次 GET 引入 HTTP 依赖。
#[tauri::command]
pub async fn update_compare(payload: String) -> Result<CheckResult, GitError> {
    update::compare(&payload)
}

/// 启动时那次检查该不该做：偏好里关掉了就不发这个请求。
///
/// 单独一个命令而不是让前端读偏好再决定——**"要不要出网"这个决定只应该有一处知道**。
#[tauri::command]
pub async fn update_check_on_startup(state: State<'_, Arc<Prefs>>) -> Result<bool, GitError> {
    Ok(state.inner().get().auto_update)
}

/// 前端要请求的地址与当前版本：一次 IPC 拿全，省得两边各写一份仓库坐标。
#[tauri::command]
pub async fn update_endpoint() -> Result<UpdateEndpoint, GitError> {
    Ok(UpdateEndpoint {
        latest_release_url: format!(
            "https://api.github.com/repos/{}/releases/latest",
            update::repo_slug()
        ),
        releases_page_url: format!("https://github.com/{}/releases", update::repo_slug()),
        current_version: update::current_version().to_string(),
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEndpoint {
    pub latest_release_url: String,
    pub releases_page_url: String,
    pub current_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_points_at_the_configured_repo() {
        let endpoint = UpdateEndpoint {
            latest_release_url: format!(
                "https://api.github.com/repos/{}/releases/latest",
                update::repo_slug()
            ),
            releases_page_url: format!("https://github.com/{}/releases", update::repo_slug()),
            current_version: update::current_version().to_string(),
        };
        assert!(endpoint.latest_release_url.ends_with("/releases/latest"));
        assert!(endpoint.releases_page_url.contains(update::repo_slug()));
        // 请求地址必须是 https 且只指向 GitHub：出网面就这一个，看得清
        assert!(endpoint
            .latest_release_url
            .starts_with("https://api.github.com/"));
    }
}
