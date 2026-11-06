//! One-shot OS scheduler boundary: Windows Task Scheduler, or systemd user timers plus an XDG
//! autostart entry on Linux. No daemon, timer thread, or shell interpolation.
//!
//! Tasks keep one logical identity on every platform — `\JobScraper\JobScraper-Reminder-<uuid>`,
//! `\JobScraper\JobScraper-Sync`, `\JobScraper\JobScraper-Startup` — so the database, which stores
//! that identity per reminder, does not care which scheduler is underneath.
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use uuid::Uuid;

#[cfg(target_os = "linux")]
mod systemd;
#[cfg(windows)]
mod task_scheduler;

#[cfg(target_os = "linux")]
pub use systemd::SystemdScheduler as PlatformScheduler;
#[cfg(windows)]
pub use task_scheduler::WindowsTaskScheduler as PlatformScheduler;

/// Console children of a GUI process get their own console window unless told otherwise, so every
/// schtasks/taskkill call flashed a terminal over the app. CREATE_NO_WINDOW suppresses that window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Keeps a helper process from opening a console window over the app. Other platforms have no
/// such window to suppress.
pub fn quiet(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
/// `quiet` for the async `Command` the worker is spawned with.
pub fn quiet_async(command: &mut tokio::process::Command) -> &mut tokio::process::Command {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}
pub const SYNC_TASK: &str = "\\JobScraper\\JobScraper-Sync";
pub const STARTUP_TASK: &str = "\\JobScraper\\JobScraper-Startup";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub due_at: DateTime<Utc>,
    pub status: String,
    pub kind: String,
}
pub trait Scheduler {
    fn create(&self, reminder: &Reminder) -> Result<String, String>;
    fn cancel(&self, task_id: &str) -> Result<(), String>;
    fn exists(&self, task_id: &str) -> Result<bool, String>;
    fn list_managed(&self) -> Result<Vec<String>, String>;
}
/// Signing in should not throw a window at you. The app starts in the notification area instead —
/// where it lives once the window is closed anyway — and its background work runs either way.
pub const STARTUP_ARGUMENT: &str = "--hidden";
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub enabled: bool,
    pub issue: Option<String>,
    pub last_result: Option<u32>,
}
/// The absolute path scheduled launches must run. A package upgrade replaces the binary under a
/// running process, and Linux then reports its own path as "/usr/bin/jobscraper (deleted)" —
/// scheduling that would point every future launch at nothing.
pub fn executable_path() -> Result<String, String> {
    let path = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    Ok(path.strip_suffix(" (deleted)").unwrap_or(&path).to_owned())
}
pub fn current_scheduler() -> Result<PlatformScheduler, String> {
    Ok(PlatformScheduler::packaged(executable_path()?))
}
pub async fn run_scheduler_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|e| format!("Task Scheduler worker failed: {e}"))?
}
pub fn task_name(id: &str) -> Result<String, String> {
    Uuid::parse_str(id).map_err(|_| "Reminder ID must be UUID")?;
    Ok(format!("JobScraper-Reminder-{id}"))
}
pub fn is_managed_task_path(value: &str) -> bool {
    let normalized = value.replace('/', "\\");
    let prefix = "\\JobScraper\\JobScraper-Reminder-";
    normalized
        .strip_prefix(prefix)
        .and_then(|id| Uuid::parse_str(id).ok())
        .is_some()
}
pub fn is_managed_sync_task(value: &str) -> bool {
    value.replace('/', "\\") == SYNC_TASK
}
pub fn is_managed_startup_task(value: &str) -> bool {
    value.replace('/', "\\") == STARTUP_TASK
}
/// Every scheduler call takes a task path from somewhere — the database, a command — so each one
/// checks it first: nothing outside JobScraper's own namespace is ever touched.
pub fn validate_task_path(task_id: &str) -> Result<(), String> {
    if is_managed_task_path(task_id)
        || is_managed_sync_task(task_id)
        || is_managed_startup_task(task_id)
    {
        Ok(())
    } else {
        Err("Task path is outside JobScraper scope".into())
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchAtLoginStatus {
    #[serde(flatten)]
    task: TaskStatus,
    debug_build: bool,
}
/// Async so Tauri runs it off the main thread: the scheduler shells out (schtasks.exe,
/// systemctl) and a sync command would block the window while it runs.
#[tauri::command]
pub async fn launch_at_login_status() -> Result<LaunchAtLoginStatus, String> {
    let scheduler = current_scheduler()?;
    Ok(LaunchAtLoginStatus {
        task: run_scheduler_blocking(move || scheduler.status(STARTUP_TASK)).await?,
        debug_build: cfg!(debug_assertions),
    })
}
#[tauri::command]
pub async fn set_launch_at_login(enabled: bool) -> Result<LaunchAtLoginStatus, String> {
    let scheduler = current_scheduler()?;
    let task = run_scheduler_blocking(move || {
        if enabled {
            scheduler.create_startup()?;
        } else if scheduler.exists(STARTUP_TASK)? {
            scheduler.cancel(STARTUP_TASK)?;
        }
        scheduler.status(STARTUP_TASK)
    })
    .await?;
    Ok(LaunchAtLoginStatus {
        task,
        debug_build: cfg!(debug_assertions),
    })
}
/// A sign-in task written by an earlier version still opens a window every morning, and nothing
/// about a scheduled task changes on its own. Checked once per launch; rewritten only when it is
/// genuinely out of date, so the usual case costs one query and no change at all.
pub async fn refresh_login_tasks() -> Result<bool, String> {
    run_scheduler_blocking(move || {
        // Development launches must never repoint the installed app's login tasks.
        if cfg!(debug_assertions) {
            return Ok(false);
        }
        #[cfg(windows)]
        return task_scheduler::refresh_login_tasks_blocking();
        #[cfg(target_os = "linux")]
        return systemd::refresh_login_tasks_blocking();
    })
    .await
}
/// Safety boundary shared by DB reconciliation and tests. Names outside this
/// UUID namespace cannot become deletion candidates.
pub fn scoped_orphans(managed: &[String], expected: &HashSet<String>) -> Vec<String> {
    managed
        .iter()
        .filter(|task| is_managed_task_path(task) && !expected.contains(*task))
        .cloned()
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reminder_names_are_scoped_to_uuids() {
        let id = Uuid::new_v4().to_string();
        assert_eq!(task_name(&id).unwrap(), format!("JobScraper-Reminder-{id}"));
        assert!(task_name("nope").is_err());
        assert!(is_managed_task_path(&format!(
            "\\JobScraper\\JobScraper-Reminder-{id}"
        )));
        assert!(is_managed_task_path(&format!(
            "/JobScraper/JobScraper-Reminder-{id}"
        )));
        assert!(!is_managed_task_path("\\Microsoft\\Windows\\Update"));
        assert!(!is_managed_task_path(
            "\\JobScraper\\JobScraper-Reminder-not-a-uuid"
        ));
        assert!(validate_task_path("..\\..\\etc\\passwd").is_err());
    }
    #[test]
    fn only_scoped_uuid_orphans_can_be_deleted() {
        let kept = format!("\\JobScraper\\JobScraper-Reminder-{}", Uuid::new_v4());
        let orphan = format!("\\JobScraper\\JobScraper-Reminder-{}", Uuid::new_v4());
        let all = vec![
            kept.clone(),
            orphan.clone(),
            "\\Microsoft\\Windows\\Update".into(),
        ];
        assert_eq!(scoped_orphans(&all, &HashSet::from([kept])), vec![orphan]);
    }
    #[test]
    fn only_the_exact_sync_task_path_is_managed() {
        assert!(is_managed_sync_task(SYNC_TASK));
        assert!(is_managed_sync_task("/JobScraper/JobScraper-Sync"));
        assert!(!is_managed_sync_task("\\JobScraper\\JobScraper-Sync2"));
        assert!(!is_managed_sync_task("\\JobScraper\\"));
        assert!(!is_managed_sync_task("\\Microsoft\\Windows\\Update"));
    }
}
