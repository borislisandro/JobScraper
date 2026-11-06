//! Linux implementation of the scheduler boundary. Reminders and the four-hourly check are systemd
//! *user* timers, and starting at sign-in is an XDG autostart entry — all plain files under the
//! user's own config directory, driven through `systemctl --user` with an argument vector (no
//! shell, no root). Every name carries the JobScraper prefix and is validated before use, so
//! nothing else in the user's systemd or autostart folders is ever read as ours or removed.
use super::{
    is_managed_startup_task, is_managed_sync_task, is_managed_task_path, task_name,
    validate_task_path, Reminder, Scheduler, TaskStatus, STARTUP_ARGUMENT, STARTUP_TASK, SYNC_TASK,
};
use chrono::{DateTime, Utc};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const SYNC_UNIT: &str = "JobScraper-Sync";
const AUTOSTART_FILE: &str = "jobscraper.desktop";
const NEEDS_UPDATE: &str =
    "Scheduled task needs updating. Turn this option off and on to rewrite it.";

#[derive(Clone)]
pub struct SystemdScheduler {
    pub executable: String,
    config_home: PathBuf,
    data_home: PathBuf,
    /// False only in tests, which exercise the files without a systemd to talk to.
    use_systemctl: bool,
}
impl SystemdScheduler {
    pub fn packaged(executable: String) -> Self {
        Self {
            executable,
            config_home: xdg_dir("XDG_CONFIG_HOME", ".config"),
            data_home: xdg_dir("XDG_DATA_HOME", ".local/share"),
            use_systemctl: true,
        }
    }
    fn unit_dir(&self) -> PathBuf {
        self.config_home.join("systemd/user")
    }
    fn autostart_path(&self) -> PathBuf {
        self.config_home.join("autostart").join(AUTOSTART_FILE)
    }
    /// systemd records when a persistent timer last fired in this file. Without one, a timer that
    /// came due while the computer was off is treated as never having been scheduled and is
    /// skipped; with one dated at creation, it fires as soon as the user manager starts — the
    /// "run when available" behaviour the Windows task asks for.
    fn stamp_path(&self, base: &str) -> PathBuf {
        self.data_home
            .join("systemd/timers")
            .join(format!("stamp-{base}.timer"))
    }
    fn service_path(&self, base: &str) -> PathBuf {
        self.unit_dir().join(format!("{base}.service"))
    }
    fn timer_path(&self, base: &str) -> PathBuf {
        self.unit_dir().join(format!("{base}.timer"))
    }
    fn ctl(&self, args: &[&str]) -> Result<std::process::Output, String> {
        if !self.use_systemctl {
            return Ok(std::process::Output {
                status: success_status(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }
        super::quiet(Command::new("systemctl").arg("--user").args(args))
            .output()
            .map_err(|e| format!("systemd is not available on this system: {e}"))
    }
    fn ctl_ok(&self, args: &[&str]) -> Result<(), String> {
        let output = self.ctl(args)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "systemctl --user {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
    /// Writes both units, reloads, and (re)arms the timer so a changed schedule takes effect.
    fn install(&self, base: &str, service: &str, timer: &str, stamp: bool) -> Result<(), String> {
        write_atomic(&self.service_path(base), service)?;
        write_atomic(&self.timer_path(base), timer)?;
        if stamp {
            let path = self.stamp_path(base);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&path, b"").map_err(|e| e.to_string())?;
        }
        let timer_unit = format!("{base}.timer");
        self.ctl_ok(&["daemon-reload"])?;
        self.ctl_ok(&["enable", &timer_unit])?;
        self.ctl_ok(&["restart", &timer_unit])
    }
    pub fn create_sync(&self) -> Result<(), String> {
        self.install(
            SYNC_UNIT,
            &sync_service(&self.executable)?,
            &sync_timer(),
            false,
        )
    }
    pub fn create_startup(&self) -> Result<(), String> {
        write_atomic(&self.autostart_path(), &autostart_entry(&self.executable)?)
    }
    pub fn status(&self, task_id: &str) -> Result<TaskStatus, String> {
        validate_task_path(task_id)?;
        if is_managed_startup_task(task_id) {
            return self.startup_status();
        }
        let base = unit_base(task_id)?;
        let (service, timer) = (self.service_path(&base), self.timer_path(&base));
        if !service.exists() && !timer.exists() {
            return Ok(TaskStatus {
                enabled: false,
                issue: None,
                last_result: None,
            });
        }
        let state = self.ctl(&["is-enabled", &format!("{base}.timer")])?;
        let enabled = matches!(
            String::from_utf8_lossy(&state.stdout).trim(),
            "enabled" | "enabled-runtime"
        );
        let mut issue = None;
        if enabled && is_managed_sync_task(task_id) {
            let current = fs::read_to_string(&service).ok()
                == Some(sync_service(&self.executable)?)
                && fs::read_to_string(&timer).ok() == Some(sync_timer());
            if !current {
                issue = Some(NEEDS_UPDATE.to_string());
            }
        }
        let mut last_result = None;
        if self.use_systemctl {
            let shown = self.ctl(&[
                "show",
                &format!("{base}.service"),
                "-p",
                "Result",
                "-p",
                "ExecMainStatus",
            ])?;
            let (result, code) = parse_service_result(&String::from_utf8_lossy(&shown.stdout));
            last_result = code;
            if issue.is_none() {
                issue = result_issue(&result, code, &base);
            }
        }
        Ok(TaskStatus {
            enabled,
            issue,
            last_result,
        })
    }
    fn startup_status(&self) -> Result<TaskStatus, String> {
        let entry = match fs::read_to_string(self.autostart_path()) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(TaskStatus {
                    enabled: false,
                    issue: None,
                    last_result: None,
                })
            }
            Err(error) => return Err(error.to_string()),
        };
        let enabled = autostart_is_enabled(&entry);
        let issue = (enabled
            && entry_line(&entry, "Exec")
                != entry_line(&autostart_entry(&self.executable)?, "Exec"))
        .then(|| NEEDS_UPDATE.to_string());
        Ok(TaskStatus {
            enabled,
            issue,
            last_result: None,
        })
    }
}
impl Scheduler for SystemdScheduler {
    fn create(&self, reminder: &Reminder) -> Result<String, String> {
        let name = task_name(&reminder.id)?;
        self.install(
            &name,
            &reminder_service(&self.executable, &reminder.id)?,
            &reminder_timer(&reminder.id, reminder.due_at),
            true,
        )?;
        Ok(format!("\\JobScraper\\{name}"))
    }
    fn cancel(&self, task_id: &str) -> Result<(), String> {
        validate_task_path(task_id)?;
        if is_managed_startup_task(task_id) {
            return remove_if_present(&self.autostart_path());
        }
        let base = unit_base(task_id)?;
        // Disabling is best effort: a timer that was never enabled, or a machine without systemd,
        // still has its files removed, and the files are what make a task exist.
        let _ = self.ctl(&["disable", "--now", &format!("{base}.timer")]);
        remove_if_present(&self.timer_path(&base))?;
        remove_if_present(&self.service_path(&base))?;
        remove_if_present(&self.stamp_path(&base))?;
        let _ = self.ctl(&["daemon-reload"]);
        let _ = self.ctl(&["reset-failed", &format!("{base}.service")]);
        Ok(())
    }
    fn exists(&self, task_id: &str) -> Result<bool, String> {
        validate_task_path(task_id)?;
        if is_managed_startup_task(task_id) {
            return Ok(self.autostart_path().exists());
        }
        let base = unit_base(task_id)?;
        Ok(self.timer_path(&base).exists() || self.service_path(&base).exists())
    }
    fn list_managed(&self) -> Result<Vec<String>, String> {
        let entries = match fs::read_dir(self.unit_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let mut present: Vec<String> = entries
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter_map(|name| name.strip_suffix(".timer").map(str::to_owned))
            .map(|base| format!("\\JobScraper\\{base}"))
            .filter(|task| is_managed_task_path(task))
            .collect();
        present.sort();
        if present.is_empty() || !self.use_systemctl {
            return Ok(present);
        }
        // A timer file that is not enabled will never fire, so it must not count as scheduled:
        // leaving it out lets reconciliation write it again.
        let units: Vec<String> = present
            .iter()
            .map(|task| format!("{}.timer", unit_base(task).unwrap_or_default()))
            .collect();
        let mut args = vec!["is-enabled"];
        args.extend(units.iter().map(String::as_str));
        let output = self.ctl(&args)?;
        let states = String::from_utf8_lossy(&output.stdout);
        Ok(present
            .into_iter()
            .zip(states.lines().chain(std::iter::repeat("")))
            .filter(|(_, state)| matches!(state.trim(), "enabled" | "enabled-runtime"))
            .map(|(task, _)| task)
            .collect())
    }
}
/// Rewrites a sign-in or background task that an older build left pointing at the wrong thing,
/// but never re-enables one the user switched off.
pub fn refresh_login_tasks_blocking() -> Result<bool, String> {
    let scheduler = super::current_scheduler()?;
    let mut changed = false;
    for task in [STARTUP_TASK, SYNC_TASK] {
        if !scheduler.exists(task)? {
            continue;
        }
        let status = scheduler.status(task)?;
        if !status.enabled || status.issue.as_deref() != Some(NEEDS_UPDATE) {
            continue;
        }
        if task == STARTUP_TASK {
            scheduler.create_startup()?;
        } else {
            scheduler.create_sync()?;
        }
        changed = true;
    }
    Ok(changed)
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    // The spec says a relative value is invalid and must be ignored.
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
        })
}
fn unit_base(task_id: &str) -> Result<String, String> {
    validate_task_path(task_id)?;
    let base = task_id
        .rsplit(['\\', '/'])
        .next()
        .ok_or("Task path is outside JobScraper scope")?;
    Ok(base.to_owned())
}
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("Scheduler path has no directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, text).map_err(|e| e.to_string())?;
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}
fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.to_string()),
        _ => Ok(()),
    }
}
fn success_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}
fn check_executable(executable: &str) -> Result<(), String> {
    if executable.is_empty()
        || !Path::new(executable).is_absolute()
        || executable.chars().any(char::is_control)
    {
        return Err("Invalid packaged executable".into());
    }
    Ok(())
}
/// systemd expands `%` specifiers and `$` variables even inside quoted ExecStart arguments, and
/// treats `\` and `"` as escapes, so a path containing any of them needs each one spelled out.
fn exec_start(executable: &str, arguments: &str) -> Result<String, String> {
    check_executable(executable)?;
    let quoted = executable
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    Ok(format!("\"{quoted}\" {arguments}"))
}
fn reminder_service(executable: &str, id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Reminder ID must be UUID")?;
    Ok(format!(
        "# Managed by JobScraper; edits are overwritten.\n[Unit]\nDescription=JobScraper reminder {id}\n\n[Service]\nType=oneshot\nExecStart={}\nTimeoutStartSec=1h\n",
        exec_start(executable, &format!("--deliver-reminder {id}"))?
    ))
}
/// The due time is written in UTC, exactly as it is stored, so the local timezone changing
/// between creating the reminder and its coming due cannot move it.
fn reminder_timer(id: &str, due: DateTime<Utc>) -> String {
    format!(
        "# Managed by JobScraper; edits are overwritten.\n[Unit]\nDescription=JobScraper reminder {id}\n\n[Timer]\nOnCalendar={}\nPersistent=true\nAccuracySec=1s\n\n[Install]\nWantedBy=timers.target\n",
        due.format("%Y-%m-%d %H:%M:%S UTC")
    )
}
fn sync_service(executable: &str) -> Result<String, String> {
    Ok(format!(
        "# Managed by JobScraper; edits are overwritten.\n[Unit]\nDescription=JobScraper background check\n\n[Service]\nType=oneshot\nExecStart={}\nTimeoutStartSec=2h\n",
        exec_start(executable, "--sync")?
    ))
}
/// A first check five minutes after the timer starts — which is shortly after signing in, and not
/// at the moment the setting is switched on — then four hours after each one finishes.
fn sync_timer() -> String {
    "# Managed by JobScraper; edits are overwritten.\n[Unit]\nDescription=JobScraper background check every 4 hours\n\n[Timer]\nOnActiveSec=5min\nOnUnitInactiveSec=4h\nAccuracySec=1min\n\n[Install]\nWantedBy=timers.target\n".into()
}
/// Desktop Entry `Exec` values are quoted, with `"`, `` ` ``, `$` and `\` backslash-escaped, then
/// the whole value goes through the file format's own string escaping, and a literal `%` is `%%`.
fn autostart_entry(executable: &str) -> Result<String, String> {
    check_executable(executable)?;
    let quoted = executable
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('\\', "\\\\")
        .replace('%', "%%");
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=JobScraper\nComment=Start JobScraper in the notification area\nExec=\"{quoted}\" {STARTUP_ARGUMENT}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    ))
}
fn entry_line<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
    entry
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}
/// Desktop environments disable an autostart entry by marking it hidden rather than deleting it.
fn autostart_is_enabled(entry: &str) -> bool {
    entry_line(entry, "Hidden") != Some("true")
        && entry_line(entry, "X-GNOME-Autostart-enabled") != Some("false")
        && entry_line(entry, "X-KDE-AutostartEnabled") != Some("false")
}
fn parse_service_result(shown: &str) -> (String, Option<u32>) {
    let mut result = String::new();
    let mut code = None;
    for line in shown.lines() {
        if let Some(value) = line.strip_prefix("Result=") {
            result = value.trim().to_owned();
        } else if let Some(value) = line.strip_prefix("ExecMainStatus=") {
            code = value.trim().parse().ok();
        }
    }
    (result, code)
}
fn result_issue(result: &str, code: Option<u32>, base: &str) -> Option<String> {
    if result.is_empty() || result == "success" {
        return None;
    }
    Some(format!(
        "The last scheduled launch failed ({result}{}). Run `journalctl --user -u {base}.service` for details.",
        code.map(|code| format!(", exit status {code}"))
            .unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn scheduler(root: &Path) -> SystemdScheduler {
        SystemdScheduler {
            executable: "/usr/bin/jobscraper".into(),
            config_home: root.join("config"),
            data_home: root.join("data"),
            use_systemctl: false,
        }
    }
    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jobscraper-systemd-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn exec_start_escapes_everything_systemd_would_expand() {
        let line = exec_start("/opt/A \"B\" 100%/$HOME\\x/jobscraper", "--sync").unwrap();
        assert_eq!(
            line,
            "\"/opt/A \\\"B\\\" 100%%/$$HOME\\\\x/jobscraper\" --sync"
        );
        assert!(exec_start("relative/jobscraper", "--sync").is_err());
        assert!(exec_start("/usr/bin/job\nscraper", "--sync").is_err());
        assert!(exec_start("", "--sync").is_err());
    }
    #[test]
    fn reminder_units_run_the_exact_reminder_in_utc() {
        let id = Uuid::new_v4().to_string();
        let due = DateTime::parse_from_rfc3339("2026-10-14T09:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let service = reminder_service("/usr/bin/jobscraper", &id).unwrap();
        assert!(service.contains(&format!(
            "ExecStart=\"/usr/bin/jobscraper\" --deliver-reminder {id}\n"
        )));
        assert!(service.contains("Type=oneshot"));
        let timer = reminder_timer(&id, due);
        assert!(timer.contains("OnCalendar=2026-10-14 09:30:00 UTC\n"));
        assert!(timer.contains("Persistent=true"));
        assert!(timer.contains("WantedBy=timers.target"));
        assert!(reminder_service("/usr/bin/jobscraper", "not-a-uuid").is_err());
    }
    #[test]
    fn background_check_starts_after_sign_in_then_repeats_every_four_hours() {
        let service = sync_service("/usr/bin/jobscraper").unwrap();
        assert!(service.contains("ExecStart=\"/usr/bin/jobscraper\" --sync\n"));
        assert!(service.contains("TimeoutStartSec=2h"));
        let timer = sync_timer();
        assert!(timer.contains("OnActiveSec=5min"));
        assert!(timer.contains("OnUnitInactiveSec=4h"));
    }
    #[test]
    fn sign_in_entry_starts_hidden_and_reads_back_as_enabled() {
        let entry = autostart_entry("/usr/bin/jobscraper").unwrap();
        assert!(entry.contains("Exec=\"/usr/bin/jobscraper\" --hidden\n"));
        assert!(autostart_is_enabled(&entry));
        assert!(!autostart_is_enabled(&format!("{entry}Hidden=true\n")));
        assert!(!autostart_is_enabled(&entry.replace(
            "X-GNOME-Autostart-enabled=true",
            "X-GNOME-Autostart-enabled=false"
        )));
        assert!(!autostart_is_enabled(&format!(
            "{entry}X-KDE-AutostartEnabled=false\n"
        )));
        let odd = autostart_entry("/opt/50%/a`b$c/jobscraper").unwrap();
        assert!(odd.contains("Exec=\"/opt/50%%/a\\\\`b\\\\$c/jobscraper\" --hidden"));
    }
    #[test]
    fn service_results_become_plain_issues() {
        let (result, code) = parse_service_result("Result=exit-code\nExecMainStatus=1\n");
        assert_eq!(result, "exit-code");
        assert_eq!(code, Some(1));
        let issue = result_issue(&result, code, "JobScraper-Sync").unwrap();
        assert!(issue.contains("exit-code") && issue.contains("exit status 1"));
        assert!(issue.contains("journalctl --user -u JobScraper-Sync.service"));
        assert!(result_issue("success", Some(0), "x").is_none());
        assert!(result_issue("", None, "x").is_none());
    }
    #[test]
    fn reminder_lifecycle_touches_only_scoped_files() {
        let root = scratch();
        let scheduler = scheduler(&root);
        let id = Uuid::new_v4().to_string();
        let reminder = Reminder {
            id: id.clone(),
            due_at: Utc::now() + chrono::Duration::days(1),
            status: "pending".into(),
            kind: "interview".into(),
        };
        // A neighbour's unit in the same directory must survive everything below.
        let neighbour = scheduler.unit_dir().join("other-app.timer");
        fs::create_dir_all(scheduler.unit_dir()).unwrap();
        fs::write(&neighbour, "[Timer]\n").unwrap();
        let task = scheduler.create(&reminder).unwrap();
        assert_eq!(task, format!("\\JobScraper\\JobScraper-Reminder-{id}"));
        assert!(scheduler.exists(&task).unwrap());
        assert_eq!(scheduler.list_managed().unwrap(), vec![task.clone()]);
        assert!(scheduler
            .stamp_path(&format!("JobScraper-Reminder-{id}"))
            .exists());
        scheduler.cancel(&task).unwrap();
        assert!(!scheduler.exists(&task).unwrap());
        assert!(scheduler.list_managed().unwrap().is_empty());
        assert!(neighbour.exists());
        // Cancelling something already gone is not an error, and out-of-scope paths are refused.
        scheduler.cancel(&task).unwrap();
        assert!(scheduler.cancel("\\Microsoft\\Windows\\Update").is_err());
        assert!(scheduler.cancel("/JobScraper/../other-app").is_err());
        assert!(neighbour.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sign_in_and_sync_tasks_report_and_clear() {
        let root = scratch();
        let scheduler = scheduler(&root);
        assert!(!scheduler.status(STARTUP_TASK).unwrap().enabled);
        assert!(!scheduler.exists(STARTUP_TASK).unwrap());
        scheduler.create_startup().unwrap();
        let status = scheduler.status(STARTUP_TASK).unwrap();
        assert!(status.enabled && status.issue.is_none());
        // A task written for another install location is reported stale, not silently trusted.
        let mut moved = scheduler.clone();
        moved.executable = "/opt/elsewhere/jobscraper".into();
        assert!(moved
            .status(STARTUP_TASK)
            .unwrap()
            .issue
            .is_some_and(|issue| issue.contains("needs updating")));
        // Disabling from the desktop's settings marks the file hidden; that reads as off.
        let path = scheduler.autostart_path();
        fs::write(
            &path,
            format!("{}Hidden=true\n", fs::read_to_string(&path).unwrap()),
        )
        .unwrap();
        assert!(!scheduler.status(STARTUP_TASK).unwrap().enabled);
        scheduler.cancel(STARTUP_TASK).unwrap();
        assert!(!scheduler.exists(STARTUP_TASK).unwrap());

        scheduler.create_sync().unwrap();
        assert!(scheduler.exists(SYNC_TASK).unwrap());
        scheduler.cancel(SYNC_TASK).unwrap();
        assert!(!scheduler.exists(SYNC_TASK).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
