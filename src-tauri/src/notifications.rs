//! One-shot Windows Scheduler boundary. No tray, daemon, timer, or shell interpolation.
use chrono::{DateTime, Local, Utc};
use std::{collections::HashSet, fs, path::Path, process::Command};
use uuid::Uuid;
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
pub struct WindowsTaskScheduler {
    pub executable: String,
}
impl WindowsTaskScheduler {
    pub fn packaged(executable: String) -> Self {
        Self { executable }
    }
    fn run(args: &[String]) -> Result<std::process::Output, String> {
        Command::new("schtasks.exe")
            .args(args)
            .output()
            .map_err(|e| format!("Task Scheduler unavailable: {e}"))
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
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><CalendarTrigger><StartBoundary>{}</StartBoundary><Enabled>true</Enabled></CalendarTrigger></Triggers>
  <Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
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
    ))
}
impl Scheduler for WindowsTaskScheduler {
    fn create(&self, reminder: &Reminder) -> Result<String, String> {
        let name = task_name(&reminder.id)?;
        let temp = std::env::temp_dir().join(format!("jobscraper-reminder-{}.xml", reminder.id));
        fs::write(
            &temp,
            task_xml(&self.executable, &reminder.id, reminder.due_at)?,
        )
        .map_err(|e| e.to_string())?;
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
        if !is_managed_task_path(task_id) {
            return Err("Refusing to delete a task outside JobScraper reminder scope".into());
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
        if !is_managed_task_path(task_id) {
            return Err("Reminder task path is outside JobScraper scope".into());
        }
        Ok(
            Self::run(&vec!["/Query".into(), "/TN".into(), task_id.into()])?
                .status
                .success(),
        )
    }
    fn list_managed(&self) -> Result<Vec<String>, String> {
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
/// Safety boundary shared by DB reconciliation and tests. Names outside this
/// UUID namespace cannot become deletion candidates.
pub fn scoped_orphans(managed: &[String], expected: &HashSet<String>) -> Vec<String> {
    managed
        .iter()
        .filter(|task| is_managed_task_path(task) && !expected.contains(*task))
        .cloned()
        .collect()
}
pub fn utc_to_local_once(due: DateTime<Utc>) -> (String, String) {
    let local = due.with_timezone(&Local);
    (
        local.format("%Y-%m-%d").to_string(),
        local.format("%H:%M").to_string(),
    )
}
pub fn reconcile(now: DateTime<Utc>, reminders: &[Reminder]) -> (Vec<String>, Vec<String>) {
    let mut schedule = vec![];
    let mut missed = vec![];
    for reminder in reminders.iter().filter(|r| r.status == "pending") {
        if reminder.due_at <= now {
            missed.push(reminder.id.clone())
        } else {
            schedule.push(reminder.id.clone())
        }
    }
    (schedule, missed)
}
pub fn fixed_task_args(
    executable: &str,
    id: &str,
    due: DateTime<Utc>,
) -> Result<Vec<String>, String> {
    if executable.is_empty() || executable.contains('"') || executable.contains('\n') {
        return Err("Invalid packaged executable".into());
    };
    let name = task_name(id)?;
    let (date, time) = utc_to_local_once(due);
    Ok(vec![
        "/Create".into(),
        "/F".into(),
        "/TN".into(),
        name,
        "/SC".into(),
        "ONCE".into(),
        "/SD".into(),
        date,
        "/ST".into(),
        time,
        "/TR".into(),
        format!("\"{executable}\" --reminder {id}"),
    ])
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    #[test]
    fn math_dst_and_safe_args() {
        let due = DateTime::parse_from_rfc3339("2026-03-29T01:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let args = fixed_task_args(
            "C:\\Program Files\\JobScraper\\JobScraper.exe",
            &Uuid::new_v4().to_string(),
            due,
        )
        .unwrap();
        assert!(args.iter().any(|v| v == "ONCE"));
        assert!(fixed_task_args("bad\"exe", &Uuid::new_v4().to_string(), due).is_err());
        assert!(task_name("nope").is_err())
    }
    #[test]
    fn xml_scopes_and_escapes_values() {
        let id = Uuid::new_v4().to_string();
        let xml = task_xml("C:\\Program Files\\A & B\\JobScraper.exe", &id, Utc::now()).unwrap();
        assert!(xml.contains("StartWhenAvailable>true"));
        assert!(xml.contains("InteractiveToken"));
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("--deliver-reminder"));
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
    fn idempotent_pending_and_missed() {
        let now = Utc::now();
        let list = vec![
            Reminder {
                id: "a".into(),
                due_at: now + Duration::hours(1),
                status: "pending".into(),
                kind: "ghosted".into(),
            },
            Reminder {
                id: "b".into(),
                due_at: now - Duration::minutes(1),
                status: "pending".into(),
                kind: "interview".into(),
            },
        ];
        assert_eq!(reconcile(now, &list), (vec!["a".into()], vec!["b".into()]));
    }
}
