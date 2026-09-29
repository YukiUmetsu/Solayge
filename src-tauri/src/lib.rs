mod agent;
mod cache;
mod commands;
mod git;
mod models;
mod opencode;
mod permissions;
mod scheduler;
mod secrets;
mod state;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            app.manage(AppState::new(data_dir));
            scheduler::spawn_scheduler(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::add_project,
            commands::remove_project,
            commands::project_status,
            commands::project_worktrees,
            commands::git_diff,
            commands::project_default_branch,
            commands::project_branch_diff,
            commands::execute_project,
            commands::create_tasks,
            commands::update_task,
            commands::delete_task,
            commands::start_task_now,
            commands::cancel_task,
            commands::retry_task,
            commands::remove_task_worktree,
            commands::clear_finished,
            commands::get_task_log,
            commands::set_concurrency,
            commands::set_project_default_profile,
            commands::plan_with_opencode,
            commands::update_settings,
            commands::get_prompt_history,
            commands::get_cache_stats,
            commands::clear_cache,
            commands::reorder_projects,
            commands::project_remote,
            commands::open_external,
            commands::open_in_editor,
            commands::update_project_config,
            commands::get_project_secrets,
            commands::get_resolved_config,
            commands::get_review_log,
            commands::list_models,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
