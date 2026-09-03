//! One-shot Windows Scheduler boundary. No tray, daemon, timer, or shell interpolation.
use chrono::{DateTime, Local, Utc};
use std::{
    collections::{HashMap, HashSet},
    os::windows::process::CommandExt,
    path::Path,
    process::Command,
};
use uuid::Uuid;
use windows::{
    core::{w, BSTR, HRESULT, PCWSTR, PWSTR},
    Win32::{
        Foundation::{
            CloseHandle, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HANDLE, HLOCAL,
        },
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, LookupAccountNameW,
            TokenUser, PSID, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER,
        },
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_MULTITHREADED,
            },
            Registry::{RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ},
            TaskScheduler::{
                IRegisteredTask, ITaskService, TaskScheduler, TASK_CREATE_OR_UPDATE,
                TASK_ENUM_HIDDEN, TASK_LOGON_INTERACTIVE_TOKEN,
            },
            Threading::{GetCurrentProcess, OpenProcessToken},
            Variant::VARIANT,
        },
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
/// All mutations are scoped to JobScraper tasks and use the current token's SID.
#[derive(Clone)]
pub struct WindowsTaskScheduler {
    pub executable: String,
}
impl WindowsTaskScheduler {
    pub fn packaged(executable: String) -> Self {
        Self { executable }
    }
    fn register(&self, task_id: &str, xml: &str) -> Result<(), String> {
        validate_task_path(task_id)?;
        let sid = current_user()?;
        // Explicit user rights survive registration from an elevated installer/app.
        let security = VARIANT::from(task_security(&sid).as_str());
        with_scheduler(|service| unsafe {
            let folder = match service.GetFolder(&BSTR::from("\\JobScraper")) {
                Ok(folder) => folder,
                Err(error) if task_not_found(&error) => service
                    .GetFolder(&BSTR::from("\\"))?
                    .CreateFolder(&BSTR::from("JobScraper"), &VARIANT::default())?,
                Err(error) => return Err(error),
            };
            let task = folder.RegisterTask(
                &BSTR::from(task_id.rsplit(['\\', '/']).next().unwrap()),
                &BSTR::from(xml),
                TASK_CREATE_OR_UPDATE.0,
                &VARIANT::from(sid.as_str()),
                &VARIANT::default(),
                TASK_LOGON_INTERACTIVE_TOKEN,
                &security,
            )?;
            // RegisterTask's SDDL applies to creation, not an existing task's ACL.
            task.SetSecurityDescriptor(&BSTR::from(task_security(&sid)), 0)?;
            Ok(())
        })
    }
    fn run(args: &[String]) -> Result<std::process::Output, String> {
        Command::new("schtasks.exe")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Task Scheduler unavailable: {e}"))
    }
    fn list_managed_native() -> Result<Vec<String>, String> {
        with_scheduler(|service| unsafe {
            let folder = match service.GetFolder(&BSTR::from("\\JobScraper")) {
                Ok(folder) => folder,
                Err(error) if task_not_found(&error) => return Ok(Vec::new()),
                Err(error) => return Err(error),
            };
            let tasks = folder.GetTasks(TASK_ENUM_HIDDEN.0)?;
            let mut managed = Vec::new();
            for index in 1..=tasks.Count()? {
                let task = tasks.get_Item(&VARIANT::from(index))?;
                let path = task.Path()?.to_string();
                if is_managed_task_path(&path) {
                    managed.push(path);
                }
            }
            Ok(managed)
        })
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
        ensure_unattended_launch(&self.executable)?;
        self.register(SYNC_TASK, &sync_task_xml(&self.executable)?)
    }
    /// Missing tasks are distinct from permission and service failures.
    pub fn task_xml(&self, task_id: &str) -> Result<Option<String>, String> {
        validate_task_path(task_id)?;
        with_scheduler(|service| unsafe {
            find_task(service, task_id)?
                .map(|task| task.Xml().map(|xml| xml.to_string()))
                .transpose()
        })
    }
    pub fn create_startup(&self) -> Result<(), String> {
        ensure_unattended_launch(&self.executable)?;
        self.register(STARTUP_TASK, &startup_task_xml(&self.executable)?)
    }
    pub fn status(&self, task_id: &str) -> Result<TaskStatus, String> {
        validate_task_path(task_id)?;
        let snapshot = with_scheduler(|service| unsafe {
            find_task(service, task_id)?
                .map(|task| {
                    Ok((
                        task.Enabled()?.as_bool(),
                        task.LastTaskResult()? as u32,
                        task.Xml()?.to_string(),
                    ))
                })
                .transpose()
        })?;
        let Some((enabled, last_result, xml)) = snapshot else {
            return Ok(TaskStatus {
                enabled: false,
                issue: None,
                last_result: None,
            });
        };
        let expected = if is_managed_startup_task(task_id) {
            startup_task_xml(&self.executable)?
        } else {
            sync_task_xml(&self.executable)?
        };
        let issue = ensure_unattended_launch(&self.executable).err()
            .or_else(|| (!task_definition_is_current(&xml, &expected)).then(|| "Scheduled task needs updating. Turn this option off and on; if Windows denies access, run repair-startup.ps1 from the installation folder once as administrator.".into()))
            .or_else(|| task_result_issue(last_result));
        Ok(TaskStatus {
            enabled,
            issue,
            last_result: Some(last_result),
        })
    }
}
fn task_security(sid: &str) -> String {
    format!("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;{sid})")
}
fn task_not_found(error: &windows::core::Error) -> bool {
    matches!(error.code(), HRESULT(value) if value == 0x8007_0002_u32 as i32 || value == 0x8007_0003_u32 as i32)
}
fn scheduler_error(error: windows::core::Error) -> String {
    if error.code() == HRESULT(0x8007_0005_u32 as i32) {
        "Windows denied access to the JobScraper task. Run repair-startup.ps1 from the installation folder once as administrator, then use JobScraper normally.".into()
    } else {
        format!("Task Scheduler failed: {error}")
    }
}
fn validate_task_path(task_id: &str) -> Result<(), String> {
    if is_managed_task_path(task_id)
        || is_managed_sync_task(task_id)
        || is_managed_startup_task(task_id)
    {
        Ok(())
    } else {
        Err("Task path is outside JobScraper scope".into())
    }
}
fn with_scheduler<T>(
    operation: impl FnOnce(&ITaskService) -> windows::core::Result<T>,
) -> Result<T, String> {
    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(scheduler_error)?;
        let _guard = ComGuard;
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
            .map_err(scheduler_error)?;
        let empty = VARIANT::default();
        service
            .Connect(&empty, &empty, &empty, &empty)
            .map_err(scheduler_error)?;
        operation(&service).map_err(scheduler_error)
    }
}
unsafe fn find_task(
    service: &ITaskService,
    path: &str,
) -> windows::core::Result<Option<IRegisteredTask>> {
    match service
        .GetFolder(&BSTR::from("\\JobScraper"))
        .and_then(|folder| folder.GetTask(&BSTR::from(path.rsplit(['\\', '/']).next().unwrap())))
    {
        Ok(task) => Ok(Some(task)),
        Err(error) if task_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}
fn current_user() -> Result<String, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| e.to_string())?;
        let result = (|| -> windows::core::Result<String> {
            let mut length = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut length);
            // Word alignment is required for TOKEN_USER and its embedded SID pointer.
            let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                length,
                &mut length,
            )?;
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut sid = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid)?;
            let result = sid.to_string();
            LocalFree(Some(HLOCAL(sid.0.cast())));
            result.map_err(windows::core::Error::from)
        })();
        let _ = CloseHandle(token);
        result.map_err(|e| format!("Cannot identify the signed-in user: {e}"))
    }
}
fn ensure_unattended_launch(executable: &str) -> Result<(), String> {
    let name: Vec<u16> = executable.encode_utf16().chain(Some(0)).collect();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        unsafe {
            let key = w!("Software\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Layers");
            let mut bytes = 0;
            let result = RegGetValueW(
                hive,
                key,
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut bytes),
            );
            if result == ERROR_FILE_NOT_FOUND || result == ERROR_PATH_NOT_FOUND {
                continue;
            }
            result
                .ok()
                .map_err(|e| format!("Cannot check Windows compatibility settings: {e}"))?;
            let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
            RegGetValueW(
                hive,
                key,
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
            .ok()
            .map_err(|e| e.to_string())?;
            let flags = String::from_utf16_lossy(&buffer);
            if flags
                .trim_end_matches('\0')
                .split_whitespace()
                .any(|flag| flag.eq_ignore_ascii_case("RUNASADMIN"))
            {
                return Err("Windows forces JobScraper to run as administrator, so scheduled launches cannot start. Run repair-startup.ps1 from the installation folder once as administrator.".into());
            }
        }
    }
    Ok(())
}
// Task Scheduler exports a logon trigger's SID as DOMAIN\name on some Windows versions.
fn account_sid(account: &str) -> Option<String> {
    if account.starts_with("S-1-") {
        return Some(account.into());
    }
    let name: Vec<u16> = account.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let (mut sid_size, mut domain_size) = (0, 0);
        let mut kind = SID_NAME_USE::default();
        let _ = LookupAccountNameW(
            None,
            PCWSTR(name.as_ptr()),
            None,
            &mut sid_size,
            None,
            &mut domain_size,
            &mut kind,
        );
        if sid_size == 0 {
            return None;
        }
        let mut sid = vec![0usize; (sid_size as usize).div_ceil(size_of::<usize>())];
        let mut domain = vec![0u16; domain_size as usize];
        let sid = PSID(sid.as_mut_ptr().cast());
        LookupAccountNameW(
            None,
            PCWSTR(name.as_ptr()),
            Some(sid),
            &mut sid_size,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_size,
            &mut kind,
        )
        .ok()?;
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(sid, &mut text).ok()?;
        let result = text.to_string().ok();
        LocalFree(Some(HLOCAL(text.0.cast())));
        result
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
  <Triggers><TimeTrigger><StartBoundary>{}</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><StartWhenAvailable>true</StartWhenAvailable><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT1H</ExecutionTimeLimit></Settings>
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
        user = xml_escape(&current_user()?),
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
  <RegistrationInfo><Version>2</Version></RegistrationInfo>
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
        user = xml_escape(&current_user()?),
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
  <RegistrationInfo><Version>2</Version></RegistrationInfo>
  <Triggers><LogonTrigger><UserId>{user}</UserId><Enabled>true</Enabled></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><StartWhenAvailable>true</StartWhenAvailable><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command><Arguments>{}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>
</Task>"#,
        xml_escape(executable),
        STARTUP_ARGUMENT,
        xml_escape(&work),
        user = xml_escape(&current_user()?),
    ))
}
/// Signing in should not throw a window at you. The app starts in the notification area instead —
/// where it lives once the window is closed anyway — and its background work runs either way.
pub const STARTUP_ARGUMENT: &str = "--hidden";
fn task_fields(xml: &str) -> Result<HashMap<String, Vec<String>>, quick_xml::Error> {
    use quick_xml::{events::Event, Reader};
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut path = Vec::new();
    let mut fields: HashMap<String, Vec<String>> = HashMap::new();
    loop {
        match reader.read_event()? {
            Event::Start(tag) => {
                path.push(String::from_utf8_lossy(tag.local_name().as_ref()).into_owned());
                fields
                    .entry(path.join("/"))
                    .or_default()
                    .push(String::new());
            }
            Event::Text(text) => {
                if let Some(value) = fields
                    .get_mut(&path.join("/"))
                    .and_then(|values| values.last_mut())
                {
                    value.push_str(&text.unescape()?);
                }
            }
            Event::End(_) => {
                path.pop();
            }
            Event::Eof => return Ok(fields),
            _ => {}
        }
    }
}
fn task_definition_is_current(xml: &str, expected: &str) -> bool {
    let (Ok(actual), Ok(expected)) = (task_fields(xml), task_fields(expected)) else {
        return false;
    };
    expected.iter().all(|(path, values)| {
        if path.ends_with("/StartBoundary") {
            return true;
        }
        let default = match path.as_str() {
            "Task/Principals/Principal/RunLevel" => Some("LeastPrivilege"),
            "Task/Triggers/LogonTrigger/Enabled" | "Task/Triggers/CalendarTrigger/Enabled" => {
                Some("true")
            }
            _ => None,
        };
        let fallback = default.map(|value| vec![value.to_string()]);
        actual.get(path).or(fallback.as_ref()).is_some_and(|found| {
            found == values
                || (path.ends_with("/UserId")
                    && found.len() == values.len()
                    && found.iter().zip(values).all(|(left, right)| {
                        account_sid(left).is_some_and(|left| Some(left) == account_sid(right))
                    }))
        })
    }) && !actual.keys().any(|path| {
        (path.starts_with("Task/Triggers/") || path.starts_with("Task/Actions/"))
            && !expected.contains_key(path)
    })
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub enabled: bool,
    pub issue: Option<String>,
    pub last_result: Option<u32>,
}
fn task_result_issue(result: u32) -> Option<String> {
    match result {
        0 | 0x0004_1300..=0x0004_1303 => None, // Success, ready, running, disabled, never run.
        0x8007_02e4 => Some("The last scheduled launch required administrator privileges (0x800702E4). Repair Windows compatibility settings before the next sign-in.".into()),
        _ => Some(format!("The last scheduled launch returned 0x{result:08X}. Check Task Scheduler history.")),
    }
}
impl Scheduler for WindowsTaskScheduler {
    fn create(&self, reminder: &Reminder) -> Result<String, String> {
        let name = task_name(&reminder.id)?;
        ensure_unattended_launch(&self.executable)?;
        self.register(
            &format!("\\JobScraper\\{name}"),
            &task_xml(&self.executable, &reminder.id, reminder.due_at)?,
        )?;
        Ok(format!("\\JobScraper\\{name}"))
    }
    fn cancel(&self, task_id: &str) -> Result<(), String> {
        validate_task_path(task_id)?;
        with_scheduler(|service| unsafe {
            let folder = match service.GetFolder(&BSTR::from("\\JobScraper")) {
                Ok(folder) => folder,
                Err(error) if task_not_found(&error) => return Ok(()),
                Err(error) => return Err(error),
            };
            match folder.DeleteTask(&BSTR::from(task_id.rsplit(['\\', '/']).next().unwrap()), 0) {
                Err(error) if task_not_found(&error) => Ok(()),
                result => result,
            }
        })
    }
    fn exists(&self, task_id: &str) -> Result<bool, String> {
        self.task_xml(task_id).map(|xml| xml.is_some())
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
    #[serde(flatten)]
    task: TaskStatus,
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
        let scheduler = current_scheduler()?;
        let mut changed = false;
        for task in [STARTUP_TASK, SYNC_TASK] {
            let Some(xml) = scheduler.task_xml(task)? else {
                continue;
            };
            let expected = if task == STARTUP_TASK {
                startup_task_xml(&scheduler.executable)?
            } else {
                sync_task_xml(&scheduler.executable)?
            };
            if task_definition_is_current(&xml, &expected) {
                continue;
            }
            // Preserve a deliberate disable in Windows Settings / Task Scheduler.
            if !scheduler.status(task)?.enabled {
                continue;
            }
            ensure_unattended_launch(&scheduler.executable)?;
            scheduler.register(task, &expected)?;
            changed = true;
        }
        Ok(changed)
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
        assert!(task_definition_is_current(&xml, &xml));
        // A task written before that, which opens a window every morning, is recognised as stale
        // so a launch can rewrite it.
        assert!(!task_definition_is_current(
            "<Actions><Exec><Command>JobScraper.exe</Command></Exec></Actions>",
            &xml
        ));
        assert!(is_managed_startup_task(STARTUP_TASK));
        assert!(is_managed_startup_task("/JobScraper/JobScraper-Startup"));
        assert!(!is_managed_startup_task(
            "\\JobScraper\\JobScraper-Startup2"
        ));
    }
    #[test]
    fn startup_reconciliation_checks_definition_not_a_hidden_substring() {
        let xml = startup_task_xml("C:\\JobScraper\\jobscraper.exe").unwrap();
        for (old, new) in [
            ("jobscraper.exe", "old.exe"),
            ("--hidden", "--hidden --other"),
            ("<Version>2", "<Version>1"),
            ("<ExecutionTimeLimit>PT0S", "<ExecutionTimeLimit>PT72H"),
            (
                "<DisallowStartIfOnBatteries>false",
                "<DisallowStartIfOnBatteries>true",
            ),
            (
                "<StopIfGoingOnBatteries>false",
                "<StopIfGoingOnBatteries>true",
            ),
            ("LeastPrivilege", "HighestAvailable"),
        ] {
            assert!(
                !task_definition_is_current(&xml.replace(old, new), &xml),
                "{old}"
            );
        }
        assert!(!task_definition_is_current(
            &xml.replace(&current_user().unwrap(), "S-1-5-18"),
            &xml
        ));
        assert!(task_definition_is_current(
            &xml.replace("<RunLevel>LeastPrivilege</RunLevel>", ""),
            &xml
        ));
        assert!(!task_definition_is_current("<broken", &xml));
    }
    #[test]
    fn task_health_distinguishes_failures_from_scheduler_states() {
        for result in [0, 0x41300, 0x41301, 0x41302, 0x41303] {
            assert!(task_result_issue(result).is_none());
        }
        assert!(task_result_issue(0x800702e4)
            .unwrap()
            .contains("administrator"));
        assert!(task_result_issue(0x80070002)
            .unwrap()
            .contains("0x80070002"));
        let sid = current_user().unwrap();
        assert!(sid.starts_with("S-1-"));
        assert!(task_security(&sid).contains(&format!("(A;;FA;;;{sid})")));
        assert!(!task_not_found(&windows::core::Error::from_hresult(
            HRESULT(0x80070005_u32 as i32)
        )));
    }
    #[test]
    #[ignore = "Creates and deletes a unique disposable Task Scheduler task for the current user"]
    fn current_user_can_register_update_disable_and_delete_task() {
        let scheduler =
            WindowsTaskScheduler::packaged(std::env::current_exe().unwrap().display().to_string());
        let path = format!(
            "\\JobScraper\\{}",
            task_name(&Uuid::new_v4().to_string()).unwrap()
        );
        struct Cleanup(WindowsTaskScheduler, String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.cancel(&self.1);
            }
        }
        let _cleanup = Cleanup(scheduler.clone(), path.clone());
        for xml in [
            startup_task_xml(&scheduler.executable).unwrap(),
            sync_task_xml(&scheduler.executable).unwrap(),
            task_xml(
                &scheduler.executable,
                &Uuid::new_v4().to_string(),
                Utc::now() + chrono::Duration::days(1),
            )
            .unwrap(),
        ] {
            let xml = xml.replace("<Enabled>true</Enabled>", "<Enabled>false</Enabled>");
            scheduler.register(&path, &xml).unwrap();
            assert!(scheduler.exists(&path).unwrap());
            let exported = scheduler.task_xml(&path).unwrap().unwrap();
            assert!(task_definition_is_current(&exported, &xml), "{exported}");
            scheduler.register(&path, &xml).unwrap();
        }
        with_scheduler(|service| unsafe {
            let task = find_task(service, &path)?.unwrap();
            task.SetEnabled(false.into())?;
            assert!(!task.Enabled()?.as_bool());
            assert!(task
                .GetSecurityDescriptor(4)?
                .to_string()
                .contains(&format!("(A;;FA;;;{})", current_user().unwrap())));
            Ok(())
        })
        .unwrap();
        scheduler.cancel(&path).unwrap();
        assert!(!scheduler.exists(&path).unwrap());
    }
}
