use crate::{backup, db, notifications, AppState};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager};

/// Startup progress, shared with the window so it can show a splash instead of a blank frame.
/// Deliberately independent of `AppState`: when the database cannot be opened at all, this is the
/// one piece of state that still answers, and the splash becomes an error screen.
#[derive(Default)]
pub struct Startup(Mutex<StartupStatus>);
#[derive(Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    pub phase: String,
    pub message: String,
    pub error: Option<String>,
}
impl Startup {
    fn set(&self, app: &tauri::AppHandle, phase: &str, message: &str) {
        let status = {
            let mut current = self.0.lock().unwrap();
            current.phase = phase.into();
            current.message = message.into();
            current.clone()
        };
        let _ = app.emit("startup", status);
    }
    fn fail(&self, app: &tauri::AppHandle, error: String) {
        let status = {
            let mut current = self.0.lock().unwrap();
            current.phase = "failed".into();
            current.message = "JobScraper could not start.".into();
            current.error = Some(error);
            current.clone()
        };
        let _ = app.emit("startup", status);
    }
}
#[tauri::command]
pub fn startup_status(state: tauri::State<'_, Startup>) -> StartupStatus {
    state.0.lock().unwrap().clone()
}
/// Everything startup used to do inline, now off the window's critical path. Each phase names
/// itself for the splash before it runs, so a slow one (the scheduler shells out to schtasks.exe
/// once per reminder) reads as progress rather than as a hang.
async fn start(
    app: &tauri::AppHandle,
    startup: &Startup,
    state: &Arc<AppState>,
    local: &Path,
) -> Result<(), String> {
    startup.set(app, "opening", "Preparing your local files");
    for folder in ["documents", "sessions", "logs"] {
        std::fs::create_dir_all(local.join(folder)).map_err(|e| e.to_string())?;
    }
    // Restore can replace the database and controlled documents only while no SQLite pool has
    // connected. The lazy pool has not, so this still runs before the first query.
    backup::apply_pending_restore(local)?;
    startup.set(app, "migrating", "Preparing the database");
    state.db.prepare().await?;
    state.db.recover_interrupted_runs().await?;
    // Listings collected before the country resolver existed carry no codes, so the Jobs page
    // location filter would miss every one of them until they were next scraped.
    let placed = db::backfill_location_countries(&state.db.pool).await?;
    if placed > 0 {
        startup.set(app, "migrating", "Working out where existing jobs are");
    }
    startup.set(app, "seeding", "Setting up job sources");
    state.db.install_starter_pack().await?;
    // The installer's answer to "Include semiconductor companies?", applied once.
    db::apply_starter_pack_opt_in(&state.db.pool, local).await?;
    if !state.headless {
        startup.set(app, "scheduling", "Setting up reminders");
        // Scheduler failure is recorded on reminder rows and never blocks local application use.
        let _ = db::reconcile_reminders_pool(&state.db.pool).await;
        // Reconciliation failure must be visible without blocking local application use.
        let refresh = notifications::refresh_login_tasks().await;
        if !matches!(refresh, Ok(false)) {
            let (level, message) = match refresh {
                Ok(_) => ("info", "Sign-in and background tasks updated.".to_string()),
                Err(error) => ("warn", error),
            };
            db::log(
                &state.db.pool,
                level,
                None,
                None,
                "launch_at_login",
                None,
                &message,
                serde_json::json!({}),
            )
            .await;
        }
    }
    Ok(())
}
/// Runs `start` on a task so the window can paint immediately, and records the outcome either way.
pub fn spawn(app: &tauri::AppHandle, state: Arc<AppState>, local: std::path::PathBuf) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let startup = handle.state::<Startup>();
        match start(&handle, &startup, &state, &local).await {
            Err(error) => startup.fail(&handle, error),
            Ok(()) => startup.set(&handle, "ready", "Ready."),
        }
    });
}

/// Headless startup performs the same migrations and starter-pack work, then exits after one
/// guarded batch. Reminder reconciliation is skipped because it adds no value to this process.
pub fn spawn_headless(app: &tauri::AppHandle, state: Arc<AppState>, local: std::path::PathBuf) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let startup = handle.state::<Startup>();
        let exit_code = match start(&handle, &startup, &state, &local).await {
            Err(error) => {
                startup.fail(&handle, error);
                1
            }
            Ok(()) => {
                startup.set(&handle, "ready", "Ready.");
                match crate::sidecar::run_automatic_sync(handle.clone(), state.clone()).await {
                    Ok(_) => 0,
                    Err(error) => {
                        db::log(
                            &state.db.pool,
                            "error",
                            None,
                            None,
                            "background_sync",
                            None,
                            &error,
                            serde_json::json!({}),
                        )
                        .await;
                        1
                    }
                }
            }
        };
        handle.exit(exit_code);
    });
}
