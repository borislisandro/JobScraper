mod availability;
mod backup;
mod db;
mod documents;
mod domain;
mod embedding;
mod matching;
mod notifications;
mod sessions;
mod sidecar;

use db::Database;
use notifications::{Scheduler, WindowsTaskScheduler};
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

pub struct AppState {
    pub db: Database,
    pub sidecars: sidecar::SidecarManager,
    pub shown_apply_attempt: Mutex<Option<String>>,
}

fn deliver_reminder_argument() -> Result<Option<String>, String> {
    let args: Vec<String> = std::env::args().collect();
    parse_deliver_reminder_args(&args)
}
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let local = app.path().local_data_dir()?.join("JobScraper");
            if let Some(reminder_id) = deliver_reminder_argument().map_err(std::io::Error::other)? {
                std::fs::create_dir_all(&local)?;
                let db =
                    tauri::async_runtime::block_on(Database::open(local.join("jobscraper.db")))?;
                if let Some(reminder) = tauri::async_runtime::block_on(
                    db::pending_reminder_delivery(&db.pool, &reminder_id),
                )
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
                    let executable = std::env::current_exe()?.display().to_string();
                    let scheduler = WindowsTaskScheduler::packaged(executable);
                    let task = format!(
                        "\\JobScraper\\{}",
                        notifications::task_name(&reminder_id).map_err(std::io::Error::other)?
                    );
                    let _ = scheduler.cancel(&task);
                }
                app.exit(0);
                return Ok(());
            }
            std::fs::create_dir_all(local.join("documents"))?;
            std::fs::create_dir_all(local.join("sessions"))?;
            std::fs::create_dir_all(local.join("logs"))?;
            // Restore can replace the database and controlled documents only while no
            // SQLite pool is open. It records its result for Diagnostics rather than
            // aborting startup after a recoverable rollback.
            let restore_result =
                backup::apply_pending_restore(&local).map_err(std::io::Error::other)?;
            let db = tauri::async_runtime::block_on(Database::open(local.join("jobscraper.db")))?;
            tauri::async_runtime::block_on(db.install_starter_pack())?;
            // Reconcile one-shot tasks during startup. Scheduler failure is recorded
            // on reminder rows and never blocks local application use.
            let _ = tauri::async_runtime::block_on(db::reconcile_reminders_pool(&db.pool));
            app.manage(Arc::new(AppState {
                db,
                sidecars: sidecar::SidecarManager::default(),
                shown_apply_attempt: Mutex::new(None),
            }));
            app.manage(restore_result);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::list_sources,
            db::get_source_config,
            db::save_source,
            db::delete_source,
            db::list_duplicate_candidates,
            db::list_merged_jobs,
            db::dismiss_duplicate_candidate,
            db::merge_duplicate_jobs,
            db::unmerge_duplicate_jobs,
            db::list_jobs,
            db::search_jobs,
            db::list_personas,
            db::get_persona,
            db::save_persona,
            db::archive_persona,
            db::delete_persona,
            db::list_review_queue,
            db::set_review_decision,
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
            db::pending_apply_confirmation,
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
            db::rescore_persona,
            db::rescore_match,
            db::rescore_stale_matches,
            db::cancel_rescore,
            db::get_match_explanation,
            documents::import_resume,
            documents::update_resume_text,
            documents::list_resume_documents,
            documents::delete_resume_document,
            embedding::embedding_status,
            sessions::save_browser_session,
            sessions::has_browser_session,
            sessions::delete_browser_session,
            sidecar::test_source,
            sidecar::adapter_manifests,
            sidecar::probe_source,
            sidecar::scrape_source,
            sidecar::scrape_all,
            sidecar::cancel_scrape_all,
            sidecar::cancel_scrape,
            sidecar::resume_scrape,
            sidecar::capture_session
        ])
        .build(tauri::generate_context!())
        .expect("failed to build JobScraper")
        .run(|app, event| match event {
            tauri::RunEvent::Exit => app.state::<Arc<AppState>>().sidecars.cancel_all(),
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Focused(true),
                ..
            } if label == "main" => {
                let state = app.state::<Arc<AppState>>();
                let _ = tauri::async_runtime::block_on(db::emit_apply_confirmation_on_focus(
                    app, &state,
                ));
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
}
