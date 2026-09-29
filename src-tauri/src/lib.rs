mod agent;
mod cache;
mod commands;
mod errorlog;
mod git;
mod models;
mod opencode;
mod opencode_server;
mod permissions;
mod scheduler;
mod secrets;
mod state;

use tauri::Manager;

use state::AppState;

/// GUI apps on macOS/Linux are started with a minimal `PATH`, so CLIs the user
/// installed via a shell (Homebrew, nvm, `~/.local/bin`, …) are invisible and
/// tasks fail to launch with ENOENT. Ask the login shell for its `PATH` and adopt
/// it. No-op on Windows (the system `PATH` is inherited normally).
#[cfg(unix)]
fn augment_path_from_login_shell() {
    use std::process::Command;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let Ok(out) = Command::new(&shell)
        .args(["-ilc", "printf '%s' \"$PATH\""])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Interactive shells may print extra lines; take the last PATH-looking one.
    let path = stdout.lines().map(str::trim).rfind(|l| l.contains('/')).unwrap_or("");
    if !path.is_empty() {
        std::env::set_var("PATH", path);
    }
}

#[cfg(not(unix))]
fn augment_path_from_login_shell() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    augment_path_from_login_shell();
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
            commands::restore_task,
            commands::start_task_now,
            commands::cancel_task,
            commands::retry_task,
            commands::answer_task,
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
            commands::get_error_log,
            commands::clear_error_log,
            commands::environment_check,
            commands::list_models,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                // Don't leave the opencode server running after the app quits.
                opencode_server::shutdown();
            }
        });
}
