mod backup;
mod db;
mod domain;
mod locations;
mod notifications;
#[cfg(unix)]
mod process_tree;
mod sessions;
mod sidecar;
mod startup;

use db::Database;
use notifications::{current_scheduler, run_scheduler_blocking, Scheduler};
use startup::Startup;
use std::{path::PathBuf, sync::Arc};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

pub struct AppState {
    pub db: Database,
    pub sidecars: sidecar::SidecarManager,
    pub automatic_sync_started: std::sync::atomic::AtomicBool,
    pub headless: bool,
}
/// Turns on start-at-sign-in and the background check from the command line, then exits.
const SETUP_BACKGROUND_ARGUMENT: &str = "--setup-background";
fn parse_deliver_reminder_args(args: &[String]) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|arg| arg == "--deliver-reminder") else {
        return Ok(None);
    };
    if args.len() != index + 2 {
        return Err("Reminder-only invocation requires exactly one UUID".into());
    }
    let id = args[index + 1].clone();
    Uuid::parse_str(&id).map_err(|_| "Reminder-only argument is not a UUID")?;
    Ok(Some(id))
}
fn parse_sync_argument(args: &[String]) -> Result<bool, String> {
    let Some(index) = args.iter().position(|arg| arg == "--sync") else {
        return Ok(false);
    };
    if index != 1 || args.len() != 2 {
        return Err("Background sync invocation accepts only --sync".into());
    }
    Ok(true)
}

/// Closing the window puts the app in the notification area instead of ending it, because the
/// things it does when nobody is looking — the four-hourly background check, reminder delivery —
/// are the point of it. Quitting is therefore a deliberate act: "Exit JobScraper" on the tray menu,
/// and nothing else. A headless `--sync` run never gets here; it has no window to close and must
/// not leave an icon behind.
fn install_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open JobScraper", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "Exit JobScraper", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &exit])?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("the window icon the tray also uses".into())
        })?)
        .tooltip("JobScraper")
        .menu(&menu)
        // The menu belongs to the right button only: a left click is how every app of this kind
        // reopens its window, and showing a menu there instead would be a small daily annoyance.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => reveal(app),
            "exit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
/// Back from the tray: a hidden window is still there, and a minimised one has to be un-minimised
/// before focus will land on it.
fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
/// Where the failed-to-start note goes: the per-user data directory the app would have used, found
/// without a running app because this is the case where there isn't one.
fn startup_log_path() -> PathBuf {
    let data = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    };
    data.unwrap_or_default().join("JobScraper/logs/startup.log")
}
pub fn run() {
    // WebKitGTK's DMA-BUF renderer aborts the whole process on Wayland with NVIDIA's proprietary
    // driver ("Error 71 (Protocol error) dispatching to Wayland display") before a window ever
    // appears. Other GPUs keep the fast path, and an explicit setting is never overridden.
    #[cfg(target_os = "linux")]
    if std::path::Path::new("/proc/driver/nvidia").exists()
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    // The scheduled background check and reminder delivery are separate processes by design, and
    // the single-instance plugin ends any second process outright: it hands its arguments over and
    // calls exit. Filtering inside the callback cannot help, because the callback runs in the
    // process that stays alive. So a scheduled run never registers the plugin at all — with the
    // window now living in the tray the app is almost always running, and every four-hourly check
    // and every reminder would otherwise die the moment it started.
    let scheduled = std::env::args().any(|arg| {
        arg == "--sync" || arg == "--deliver-reminder" || arg == SETUP_BACKGROUND_ARGUMENT
    });
    let mut builder = tauri::Builder::default();
    if !scheduled {
        // Registered first, as the plugin requires: clicking the shortcut while the app sits in the
        // tray reopens the window it already has rather than starting a rival process on the same
        // database. A hidden sign-in launch hands over its arguments and reveals nothing.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args
                .iter()
                .any(|arg| arg == notifications::STARTUP_ARGUMENT)
            {
                return;
            }
            reveal(app);
        }));
    }
    builder
        // The close button hides the window; only the tray's Exit ends the process. A headless run
        // has no window, so this never fires there.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main"
                    && !window.app_handle().state::<Arc<AppState>>().headless
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let args: Vec<String> = std::env::args().collect();
            // A dev build sharing the installed app's data directory migrates that database
            // forward, and the installed binary then refuses to open its own data — a white
            // window with no explanation. Debug builds get their own directory.
            let local = app
                .path()
                .local_data_dir()?
                .join(if cfg!(debug_assertions) {
                    "JobScraper-dev"
                } else {
                    "JobScraper"
                });
            if args.len() == 2 && args[1] == SETUP_BACKGROUND_ARGUMENT {
                std::fs::create_dir_all(&local)?;
                let db =
                    tauri::async_runtime::block_on(Database::open(local.join("jobscraper.db")))?;
                let outcome =
                    tauri::async_runtime::block_on(db::enable_background_defaults(&db.pool));
                match &outcome {
                    Ok(()) => println!("Start at sign-in and background checks are on."),
                    Err(error) => eprintln!("Could not turn them on: {error}"),
                }
                app.handle().exit(i32::from(outcome.is_err()));
                return Ok(());
            }
            if let Some(reminder_id) =
                parse_deliver_reminder_args(&args).map_err(std::io::Error::other)?
            {
                std::fs::create_dir_all(&local)?;
                let db =
                    tauri::async_runtime::block_on(Database::open(local.join("jobscraper.db")))?;
                if let Some(reminder) = tauri::async_runtime::block_on(db::claim_reminder_delivery(
                    &db.pool,
                    &reminder_id,
                ))
                .map_err(std::io::Error::other)?
                {
                    let shown = app
                        .notification()
                        .builder()
                        .title(reminder.title)
                        .body(reminder.body)
                        .show()
                        .map_err(|e| e.to_string());
                    tauri::async_runtime::block_on(db::finish_reminder_delivery(
                        &db.pool,
                        &reminder_id,
                        shown,
                    ))
                    .map_err(std::io::Error::other)?;
                    let scheduler = current_scheduler().map_err(std::io::Error::other)?;
                    let task = format!(
                        "\\JobScraper\\{}",
                        notifications::task_name(&reminder_id).map_err(std::io::Error::other)?
                    );
                    let _ = tauri::async_runtime::block_on(run_scheduler_blocking(move || {
                        scheduler.cancel(&task)
                    }));
                }
                app.handle().exit(0);
                return Ok(());
            }
            let headless = parse_sync_argument(&args).map_err(std::io::Error::other)?;
            // Tauri creates the window before this closure runs, so anything slow or fallible
            // here is a blank frame the user stares at — and anything that returns Err kills the
            // process behind an already-visible window. Nothing below touches the disk: the pool
            // is lazy, state is managed immediately, and the real work runs on a task while the
            // window paints its splash.
            // block_on only to enter the Tokio context: a lazy pool starts its idle-reaper task
            // on construction and panics without one. It still performs no I/O.
            let db = tauri::async_runtime::block_on(async {
                Database::connect_lazy(local.join("jobscraper.db"))
            });
            let state = Arc::new(AppState {
                db,
                sidecars: sidecar::SidecarManager::default(),
                automatic_sync_started: std::sync::atomic::AtomicBool::new(false),
                headless,
            });
            app.manage(state.clone());
            app.manage(Startup::default());
            if headless {
                startup::spawn_headless(app.handle(), state, local);
            } else {
                install_tray(app.handle())?;
                if !args
                    .iter()
                    .any(|arg| arg == notifications::STARTUP_ARGUMENT)
                {
                    if let Some(window) = app.get_webview_window("main") {
                        window.show()?;
                    }
                }
                startup::spawn(app.handle(), state, local);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup::startup_status,
            db::list_sources,
            db::list_company_catalog,
            db::list_profiles,
            db::apply_profile,
            db::list_app_logs,
            db::scrape_performance,
            sidecar::job_description,
            sidecar::fetch_view_descriptions,
            db::purge_all_jobs,
            db::get_setting,
            db::set_setting,
            db::background_sync_status,
            db::set_background_sync,
            notifications::launch_at_login_status,
            notifications::set_launch_at_login,
            db::get_source_config,
            db::save_source,
            db::delete_source,
            db::list_duplicate_candidates,
            db::list_merged_jobs,
            db::list_duplicate_conflicts,
            db::resolve_duplicate_conflict,
            db::dismiss_duplicate_candidate,
            db::merge_duplicate_jobs,
            db::unmerge_duplicate_jobs,
            db::list_jobs,
            db::count_jobs,
            db::job_countries,
            db::set_job_dismissed,
            db::new_since,
            sidecar::capture_job,
            db::save_captured_job,
            db::set_source_enabled,
            db::list_applications,
            db::application_details,
            db::save_application_details,
            db::save_application_note,
            db::delete_application_note,
            db::attach_application_document,
            db::export_application_document,
            db::create_application,
            db::transition_application,
            db::record_apply_decision,
            db::open_apply,
            db::delete_application,
            db::reconcile_reminders,
            db::list_interviews,
            db::save_interview,
            db::delete_interview,
            db::update_ghost_threshold,
            db::analytics,
            db::diagnostics,
            db::backup_database,
            backup::create_backup_archive,
            backup::validate_restore_archive,
            backup::stage_restore_archive,
            backup::restore_status,
            db::export_csv,
            db::export_data,
            db::preview_purge,
            db::apply_purge,
            sessions::save_browser_session,
            sessions::has_browser_session,
            sessions::delete_browser_session,
            sidecar::test_source,
            sidecar::adapter_manifests,
            sidecar::probe_source,
            sidecar::scrape_source,
            sidecar::check_sources,
            sidecar::start_automatic_sync,
            sidecar::scrape_all,
            sidecar::cancel_scrape_all,
            sidecar::cancel_scrape,
            sidecar::resume_scrape,
            sidecar::capture_session
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            // The window is already on screen by the time this could fail, so a panic here is a
            // dead white frame with nowhere to look. Leave a note where support would look first.
            let log = startup_log_path();
            if let Some(folder) = log.parent() {
                let _ = std::fs::create_dir_all(folder);
            }
            let _ = std::fs::write(
                log,
                format!(
                    "JobScraper failed to start: {error}
"
                ),
            );
            std::process::exit(1)
        })
        .run(|app, event| match event {
            // The reminder and setup invocations exit before AppState exists, and asking for
            // state that was never managed panics — turning a clean exit into a crash.
            tauri::RunEvent::Exit => {
                if let Some(state) = app.try_state::<Arc<AppState>>() {
                    state.sidecars.cancel_all();
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod reminder_cli_tests {
    use super::*;
    #[test]
    fn reminder_cli_accepts_only_exact_uuid_argument() {
        let id = Uuid::new_v4().to_string();
        assert_eq!(
            parse_deliver_reminder_args(&["app".into(), "--deliver-reminder".into(), id.clone()])
                .unwrap(),
            Some(id)
        );
        assert!(parse_deliver_reminder_args(&[
            "app".into(),
            "--deliver-reminder".into(),
            "no".into()
        ])
        .is_err());
        assert!(parse_deliver_reminder_args(&[
            "app".into(),
            "--deliver-reminder".into(),
            Uuid::new_v4().to_string(),
            "extra".into()
        ])
        .is_err());
    }
    #[test]
    fn sync_cli_accepts_only_the_bare_flag() {
        assert!(parse_sync_argument(&["app".into(), "--sync".into()]).unwrap());
        assert!(!parse_sync_argument(&["app".into()]).unwrap());
        assert!(parse_sync_argument(&["app".into(), "--sync".into(), "extra".into()]).is_err());
        assert!(parse_sync_argument(&["app".into(), "other".into(), "--sync".into()]).is_err());
    }
}
