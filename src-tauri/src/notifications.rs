//! One-shot Windows Scheduler boundary. No tray, daemon, timer, or shell interpolation.
use chrono::{DateTime, Local, Utc};
use std::{
    collections::HashSet, fs, os::windows::process::CommandExt, path::Path, process::Command,
};
use uuid::Uuid;
use windows::{
    core::{BSTR, HRESULT},
    Win32::System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        TaskScheduler::{ITaskService, TaskScheduler, TASK_ENUM_HIDDEN},
        Variant::VARIANT,
    },
};
/// Console children of a GUI process get their own console window unless told otherwise, so every
/// schtasks/taskkill call flashed a terminal over the app. CREATE_NO_WINDOW suppresses that window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
/// `schtasks.exe` is executed directly, never through a shell. IDs and task
/// names are UUID-validated; executable path cannot add a command argument.
#[derive(Clone)]
pub struct WindowsTaskScheduler {
    pub executable: String,
}
impl WindowsTaskScheduler {
    pub fn packaged(executable: String) -> Self {
        Self { executable }
    }
    /// schtasks.exe parses a task file as UTF-16 and rejects UTF-8 bytes outright
    /// ("(1,40)::ERROR: unable to switch the encoding"), so the file is written the way its own
    /// `/Query /XML` export is: a UTF-16LE byte-order mark followed by UTF-16LE text.
    fn write_task_xml(path: &Path, xml: &str) -> Result<(), String> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in xml.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(path, bytes).map_err(|e| e.to_string())
    }
    fn run(args: &[String]) -> Result<std::process::Output, String> {
        Command::new("schtasks.exe")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Task Scheduler unavailable: {e}"))
    }
    fn list_managed_native() -> Result<Vec<String>, String> {
        struct ComGuard;
        impl Drop for ComGuard {
            fn drop(&mut self) {
                unsafe { CoUninitialize() }
            }
        }
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| format!("Task Scheduler COM initialization failed: {e}"))?;
            let _guard = ComGuard;
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| format!("Task Scheduler COM service failed: {e}"))?;
            let empty = VARIANT::default();
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(|e| format!("Task Scheduler COM connection failed: {e}"))?;
            let folder = match service.GetFolder(&BSTR::from("\\JobScraper")) {
                Ok(folder) => folder,
                Err(error)
                    if matches!(
                        error.code(),
                        HRESULT(value) if value == 0x8007_0002_u32 as i32 || value == 0x8007_0003_u32 as i32
                    ) =>
                {
                    return Ok(Vec::new())
                }
                Err(error) => {
                    return Err(format!("Task Scheduler COM folder query failed: {error}"))
                }
            };
            let tasks = folder
                .GetTasks(TASK_ENUM_HIDDEN.0)
                .map_err(|e| format!("Task Scheduler COM task query failed: {e}"))?;
            let mut managed = Vec::new();
            for index in 1..=tasks
                .Count()
                .map_err(|e| format!("Task Scheduler COM count failed: {e}"))?
            {
                let task = tasks
                    .get_Item(&VARIANT::from(index))
                    .map_err(|e| format!("Task Scheduler COM item failed: {e}"))?;
                let path = task
                    .Path()
                    .map_err(|e| format!("Task Scheduler COM path failed: {e}"))?
                    .to_string();
                if is_managed_task_path(&path) {
                    managed.push(path);
                }
            }
            Ok(managed)
        }
    }
    fn list_managed_schtasks() -> Result<Vec<String>, String> {
        // CSV avoids PowerShell and commands never receive an untrusted task name.
        let out = Self::run(&["/Query".into(), "/FO".into(), "CSV".into(), "/NH".into()])?;
        if !out.status.success() {
            return Err(format!(
                "Task Scheduler query failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text
            .lines()
            .filter_map(|line| line.split(',').next())
            .map(|name| name.trim().trim_matches('"').replace("\\\\", "\\"))
            .filter(|name| is_managed_task_path(name))
            .collect())
    }
    pub fn create_sync(&self) -> Result<(), String> {
        let temp = std::env::temp_dir().join(format!("jobscraper-sync-{}.xml", std::process::id()));
        Self::write_task_xml(&temp, &sync_task_xml(&self.executable)?)?;
        let out = Self::run(&[
            "/Create".into(),
            "/XML".into(),
            temp.display().to_string(),
            "/TN".into(),
            SYNC_TASK.into(),
            "/F".into(),
        ]);
        let _ = fs::remove_file(&temp);
        let out = out?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Task Scheduler create failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    }
    /// The task as Task Scheduler holds it. schtasks writes /XML output as UTF-16, so the bytes are
    /// decoded as such when they carry its byte-order mark and read as plain text otherwise.
    pub fn task_xml(&self, task_id: &str) -> Result<Option<String>, String> {
        if !is_managed_task_path(task_id)
            && !is_managed_sync_task(task_id)
            && !is_managed_startup_task(task_id)
        {
            return Err("Task path is outside JobScraper scope".into());
        }
        let out = Self::run(&["/Query".into(), "/TN".into(), task_id.into(), "/XML".into()])?;
        if !out.status.success() {
            return Ok(None);
        }
        if out.stdout.starts_with(&[0xFF, 0xFE]) {
            let units: Vec<u16> = out.stdout[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Ok(Some(String::from_utf16_lossy(&units)))
        } else {
            Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
        }
    }
    pub fn create_startup(&self) -> Result<(), String> {
        let temp =
            std::env::temp_dir().join(format!("jobscraper-startup-{}.xml", std::process::id()));
        Self::write_task_xml(&temp, &startup_task_xml(&self.executable)?)?;
        let out = Self::run(&[
            "/Create".into(),
            "/XML".into(),
            temp.display().to_string(),
            "/TN".into(),
            STARTUP_TASK.into(),
            "/F".into(),
        ]);
        let _ = fs::remove_file(&temp);
        let out = out?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Task Scheduler create failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    }
}
/// A logon trigger that names nobody fires for EVERY user who signs in, which only an
/// administrator may register — so Task Scheduler answered "Access is denied" and both switches
/// looked broken on an ordinary account. Naming the current user makes it this user's own task,
/// which needs no elevation at all.
fn current_user() -> String {
    let name = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() && !name.is_empty() => format!("{domain}\\{name}"),
        _ => name,
    }
}
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Task Scheduler interprets `StartBoundary` in local time. Conversion happens
/// once from UTC when task XML is created; the stored DB due time stays UTC.
pub fn task_xml(executable: &str, id: &str, due: DateTime<Utc>) -> Result<String, String> {
    if executable.is_empty() || executable.contains('\n') || executable.contains('\r') {
        return Err("Invalid packaged executable".into());
    }
    Uuid::parse_str(id).map_err(|_| "Reminder ID must be UUID")?;
    let work = Path::new(executable)
        .parent()
        .ok_or("Packaged executable has no directory")?
        .display()
        .to_string();
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><CalendarTrigger><StartBoundary>{}</StartBoundary><Enabled>true</Enabled></CalendarTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><StartWhenAvailable>true</StartWhenAvailable><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command><Arguments>--deliver-reminder {}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>
</Task>"#,
        xml_escape(
            &due.with_timezone(&Local)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string(),
        ),
        xml_escape(executable),
        id,
        xml_escape(&work),
        user = xml_escape(&current_user()),
    ))
}
pub fn sync_task_xml(executable: &str) -> Result<String, String> {
    if executable.is_empty() || executable.contains('\n') || executable.contains('\r') {
        return Err("Invalid packaged executable".into());
    }
    let work = Path::new(executable)
        .parent()
        .ok_or("Packaged executable has no directory")?
        .display()
        .to_string();
    let start = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger><UserId>{user}</UserId><Enabled>true</Enabled></LogonTrigger>
    <CalendarTrigger><Repetition><Interval>PT4H</Interval><Duration>P1D</Duration></Repetition><StartBoundary>{}</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>
  </Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><StartWhenAvailable>true</StartWhenAvailable><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT2H</ExecutionTimeLimit></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command><Arguments>--sync</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>
</Task>"#,
        xml_escape(&start),
        xml_escape(executable),
        xml_escape(&work),
        user = xml_escape(&current_user()),
    ))
}
pub fn startup_task_xml(executable: &str) -> Result<String, String> {
    if executable.is_empty() || executable.contains('\n') || executable.contains('\r') {
        return Err("Invalid packaged executable".into());
    }
    let work = Path::new(executable)
        .parent()
        .ok_or("Packaged executable has no directory")?
        .display()
        .to_string();
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><UserId>{user}</UserId><Enabled>true</Enabled></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><StartWhenAvailable>true</StartWhenAvailable><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command><Arguments>{}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>
</Task>"#,
        xml_escape(executable),
        STARTUP_ARGUMENT,
        xml_escape(&work),
        user = xml_escape(&current_user()),
    ))
}
/// Signing in should not throw a window at you. The app starts in the notification area instead —
/// where it lives once the window is closed anyway — and its background work runs either way.
pub const STARTUP_ARGUMENT: &str = "--hidden";
/// A task written before the app learned to start hidden still opens a window every sign-in, and
/// nothing about it would ever change on its own. Given the XML Task Scheduler holds for that task,
/// this says whether it is still the one this version writes.
pub fn startup_task_is_current(xml: &str) -> bool {
    xml.contains(STARTUP_ARGUMENT)
}
impl Scheduler for WindowsTaskScheduler {
    fn create(&self, reminder: &Reminder) -> Result<String, String> {
        let name = task_name(&reminder.id)?;
        let temp = std::env::temp_dir().join(format!("jobscraper-reminder-{}.xml", reminder.id));
        Self::write_task_xml(
            &temp,
            &task_xml(&self.executable, &reminder.id, reminder.due_at)?,
        )?;
        let out = Self::run(&vec![
            "/Create".into(),
            "/XML".into(),
            temp.display().to_string(),
            "/TN".into(),
            format!("\\JobScraper\\{name}"),
            "/F".into(),
        ]);
        let _ = fs::remove_file(&temp);
        let out = out?;
        if !out.status.success() {
            return Err(format!(
                "Task Scheduler create failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(format!("\\JobScraper\\{name}"))
    }
    fn cancel(&self, task_id: &str) -> Result<(), String> {
        if !is_managed_task_path(task_id)
            && !is_managed_sync_task(task_id)
            && !is_managed_startup_task(task_id)
        {
            return Err("Refusing to delete a task outside JobScraper scope".into());
        }
        let out = Self::run(&vec![
            "/Delete".into(),
            "/F".into(),
            "/TN".into(),
            task_id.into(),
        ])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Task Scheduler delete failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    }
    fn exists(&self, task_id: &str) -> Result<bool, String> {
        if !is_managed_task_path(task_id)
            && !is_managed_sync_task(task_id)
            && !is_managed_startup_task(task_id)
        {
            return Err("Task path is outside JobScraper scope".into());
        }
        Ok(
            Self::run(&vec!["/Query".into(), "/TN".into(), task_id.into()])?
                .status
                .success(),
        )
    }
    fn list_managed(&self) -> Result<Vec<String>, String> {
        Self::list_managed_native().or_else(|_| Self::list_managed_schtasks())
    }
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchAtLoginStatus {
    enabled: bool,
    debug_build: bool,
}
fn current_scheduler() -> Result<WindowsTaskScheduler, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(WindowsTaskScheduler::packaged(
        executable.to_string_lossy().into_owned(),
    ))
}
/// Async so Tauri runs it off the main thread: `schtasks.exe` takes hundreds of milliseconds and a
/// sync command would block the window while it runs.
#[tauri::command]
pub async fn launch_at_login_status() -> Result<LaunchAtLoginStatus, String> {
    let scheduler = current_scheduler()?;
    Ok(LaunchAtLoginStatus {
        enabled: run_scheduler_blocking(move || scheduler.exists(STARTUP_TASK)).await?,
        debug_build: cfg!(debug_assertions),
    })
}
#[tauri::command]
pub async fn set_launch_at_login(enabled: bool) -> Result<LaunchAtLoginStatus, String> {
    let scheduler = current_scheduler()?;
    let enabled = run_scheduler_blocking(move || {
        if enabled {
            scheduler.create_startup()?;
        } else if scheduler.exists(STARTUP_TASK)? {
            scheduler.cancel(STARTUP_TASK)?;
        }
        scheduler.exists(STARTUP_TASK)
    })
    .await?;
    Ok(LaunchAtLoginStatus {
        enabled,
        debug_build: cfg!(debug_assertions),
    })
}
/// A sign-in task written by an earlier version still opens a window every morning, and nothing
/// about a scheduled task changes on its own. Checked once per launch; rewritten only when it is
/// genuinely out of date, so the usual case costs one query and no change at all.
pub async fn refresh_startup_task() -> Result<bool, String> {
    run_scheduler_blocking(move || {
        let scheduler = current_scheduler()?;
        let Some(xml) = scheduler.task_xml(STARTUP_TASK)? else {
            return Ok(false);
        };
        if startup_task_is_current(&xml) {
            return Ok(false);
        }
        scheduler.create_startup()?;
        Ok(true)
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
    fn xml_scopes_and_escapes_values() {
        let id = Uuid::new_v4().to_string();
        let xml = task_xml("C:\\Program Files\\A & B\\JobScraper.exe", &id, Utc::now()).unwrap();
        assert!(xml.contains("StartWhenAvailable>true"));
        assert!(xml.contains("InteractiveToken"));
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("--deliver-reminder"));
        assert!(task_name("nope").is_err());
        assert!(is_managed_task_path(&format!(
            "\\JobScraper\\JobScraper-Reminder-{id}"
        )));
        assert!(!is_managed_task_path("\\Microsoft\\Windows\\Update"));
        assert!(!is_managed_task_path(
            "\\JobScraper\\JobScraper-Reminder-not-a-uuid"
        ));
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
    fn sync_xml_scopes_and_escapes_values() {
        let xml = sync_task_xml("C:\\Program Files\\A & B\\JobScraper.exe").unwrap();
        assert!(xml.contains("<Arguments>--sync</Arguments>"));
        assert!(xml.contains("LogonTrigger"));
        assert!(xml.contains("PT4H"));
        assert!(xml.contains("<ExecutionTimeLimit>PT2H</ExecutionTimeLimit>"));
        assert!(xml.contains("A &amp; B"));
    }
    #[test]
    fn only_the_exact_sync_task_path_is_managed() {
        assert!(is_managed_sync_task(SYNC_TASK));
        assert!(is_managed_sync_task("/JobScraper/JobScraper-Sync"));
        assert!(!is_managed_sync_task("\\JobScraper\\JobScraper-Sync2"));
        assert!(!is_managed_sync_task("\\JobScraper\\"));
        assert!(!is_managed_sync_task("\\Microsoft\\Windows\\Update"));
    }
    #[test]
    fn startup_xml_opens_the_app_at_interactive_login() {
        let xml = startup_task_xml("C:\\Program Files\\A & B\\JobScraper.exe").unwrap();
        assert!(xml.contains("<LogonTrigger>"));
        assert!(xml.contains("InteractiveToken"));
        assert!(xml.contains("<Command>C:\\Program Files\\A &amp; B\\JobScraper.exe</Command>"));
        // Signing in must not throw a window at you: the task starts the app in the notification
        // area, and only asking for it opens the window.
        assert!(xml.contains("<Arguments>--hidden</Arguments>"));
        assert!(startup_task_is_current(&xml));
        // A task written before that, which opens a window every morning, is recognised as stale
        // so a launch can rewrite it.
        assert!(!startup_task_is_current(
            "<Actions><Exec><Command>JobScraper.exe</Command></Exec></Actions>"
        ));
        assert!(is_managed_startup_task(STARTUP_TASK));
        assert!(is_managed_startup_task("/JobScraper/JobScraper-Startup"));
        assert!(!is_managed_startup_task(
            "\\JobScraper\\JobScraper-Startup2"
        ));
    }
}
