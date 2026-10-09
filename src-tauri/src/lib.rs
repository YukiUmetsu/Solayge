// Keep first-party code free of `unsafe`: the only exception is the isolated
// Objective-C interop in `macos_notify`, which is explicitly allowed below.
#![deny(unsafe_code)]

mod agent;
mod analytics;
mod cache;
mod commands;
mod devices;
mod errorlog;
mod git;
mod host_runtime;
#[cfg(feature = "mobile-network")]
mod host;
#[cfg(feature = "mobile-network")]
mod iroh_host;
// Remote-access lifecycle (WebSocket listener + Tailscale Serve). The real
// implementation needs the network transport; the stub keeps the command
// surface compiling when `mobile-network` is disabled.
#[cfg(feature = "mobile-network")]
#[path = "remote.rs"]
mod remote;
#[cfg(not(feature = "mobile-network"))]
#[path = "remote_stub.rs"]
mod remote;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
// Objective-C interop for UNUserNotificationCenter; everything else stays safe Rust.
mod macos_notify;
mod models;
mod opencode;
mod opencode_server;
mod permissions;
mod push;
mod scheduler;
mod secrets;
mod state;
mod summary;

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
    let path = stdout
        .lines()
        .map(str::trim)
        .rfind(|l| l.contains('/'))
        .unwrap_or("");
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
            app.manage(AppState::new(data_dir.clone()));
            app.manage(devices::DeviceManager::new(&data_dir)?);
            #[cfg(target_os = "macos")]
            macos_notify::init();
            scheduler::spawn_scheduler(app.handle().clone());
            #[cfg(feature = "mobile-network")]
            {
                app.manage(remote::RemoteConnectionManager::new(&data_dir));
                remote::restore_on_startup(app.handle().clone());
                iroh_host::maybe_spawn(app.handle().clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::add_project,
            commands::remove_project,
            commands::project_status,
            commands::project_worktrees,
            commands::worktree_detail,
            commands::worktree_commit_files,
            commands::git_revert_commit,
            commands::project_branches,
            commands::git_merge_branch,
            commands::git_delete_branch,
            commands::git_delete_remote_branch,
            commands::git_remove_worktree,
            commands::git_diff,
            commands::project_default_branch,
            commands::project_branch_diff,
            commands::git_stage,
            commands::git_commit,
            commands::git_push,
            commands::git_create_pr,
            commands::git_merge_pr,
            commands::git_checkout_pull,
            commands::merge_preflight,
            commands::commit_worktrees,
            commands::execute_project,
            commands::create_tasks,
            commands::update_task,
            commands::delete_task,
            commands::restore_task,
            commands::start_task_now,
            commands::cancel_task,
            commands::retry_task,
            commands::retry_review,
            commands::answer_task,
            commands::send_task_message,
            commands::remove_task_worktree,
            commands::clear_finished,
            commands::get_task_log,
            commands::set_concurrency,
            commands::set_project_default_profile,
            commands::plan_with_opencode,
            commands::update_settings,
            commands::read_sound_file,
            commands::test_notification,
            // TEMPORARY (remove before release): on-demand background push test.
            commands::push_test_question,
            commands::notification_status,
            commands::open_notification_settings,
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
            commands::get_analytics_overview,
            commands::get_analytics_timeseries,
            commands::get_analytics_projects,
            commands::get_analytics_models,
            commands::get_analytics_reliability,
            commands::get_analytics_interactions,
            commands::get_analytics_task_detail,
            commands::get_analytics_collection_status,
            commands::clear_analytics,
            commands::export_analytics,
            devices::desktop_identity,
            devices::devices_list,
            devices::pairing_start,
            devices::pairing_status,
            devices::pairing_approve,
            devices::pairing_reject,
            devices::device_revoke,
            remote::remote_status,
            remote::remote_apply,
            remote::remote_retry,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Don't leave the opencode server running after the app quits,
                // remove Solayge's Tailscale Serve entry, and flush any
                // analytics still in memory.
                remote::shutdown(app);
                opencode_server::shutdown();
                analytics::flush(app);
            }
        });
}
