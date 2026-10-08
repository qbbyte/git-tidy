use std::sync::Arc;

use tauri::Manager;

mod commands;
mod config;
mod error;
mod git;
mod store;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 索引库落在 app data dir（§6.7）。开库失败就是启动失败，不做"没有库也能跑"的降级
            let data_dir = app.path().app_data_dir()?;
            let db = store::db::Db::open(&data_dir).map_err(|err| format!("{err:?}"))?;
            app.manage(Arc::new(db));
            // 提交图的泳道分配结果（§6.3）。纯内存、以 HEAD 的 sha 为键，
            // 提交/amend/reset 之后键自己就变了，丢了只是慢一次，不会画错
            app.manage(Arc::new(commands::graph::Cache::default()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::repo::repo_add,
            commands::repo::repo_list,
            commands::repo::repo_refresh,
            commands::repo::repo_rename,
            commands::repo::repo_remove,
            commands::repo::worktree_status,
            commands::repo::files_stage,
            commands::repo::files_unstage,
            commands::commit::commit_list,
            commands::commit::commit_show,
            commands::commit::commit_create,
            commands::detail::commit_detail,
            commands::diff::commit_file_diff,
            commands::file::file_blame,
            commands::file::file_history,
            commands::file::file_tree,
            commands::file::file_content,
            commands::graph::commit_graph,
            commands::refs::refs_scan,
            commands::spec::spec_for,
            commands::spec::message_check,
            commands::spec::commit_scopes,
            commands::remote::repo_add_remote,
            commands::remote::repo_materialize
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
