#!/bin/bash
# The Rust crate calls Windows-only APIs (Task Scheduler, process creation flags), so it does not build on Linux or macOS.
# This builds a SCRATCH COPY with just those pieces stubbed, so the database and profile logic (db.rs) can be compiled and
# tested anywhere. Nothing in the repository is changed.
#   scripts/rust-linux-check.sh test --lib                         # all unit tests (backup path test fails off Windows)
#   UPDATE_PROFILE_FIXTURE=1 scripts/rust-linux-check.sh test --lib profile_sources_match   # rewrites the copy's fixture;
#   cp "$JOBSCRAPER_SHIM_DIR/sidecar/profile-sources.fixture.json" sidecar/                 # then copy it back
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
W="${JOBSCRAPER_SHIM_DIR:-${TMPDIR:-/tmp}/jobscraper-rust-shim}"
mkdir -p "$W"
rsync -a --delete --exclude target --exclude node_modules --exclude .git --exclude dist --exclude docs --exclude 'src-tauri/sidecar' "$REPO/src-tauri" "$REPO/sidecar" "$REPO/scripts" "$W/"
mkdir -p "$W/src-tauri/sidecar" && touch "$W/src-tauri/sidecar/.keep"
W="$W" python3 - <<'P'
import os, re
W = os.environ['W'] + '/src-tauri/'
cargo = open(W + 'Cargo.toml').read()
i = cargo.index('windows = {'); j = cargo.index('\n', i)
line = cargo[i:j + 1]; cargo = cargo[:i] + cargo[j + 1:]
open(W + 'Cargo.toml', 'w').write(cargo.rstrip() + "\n\n[target.'cfg(windows)'.dependencies]\n" + line)
side = open(W + 'src/sidecar.rs').read()
side = side.replace('os::windows::process::CommandExt,', '')
side = re.sub(r'\s*\.creation_flags\(crate::notifications::CREATE_NO_WINDOW\)', '', side)
open(W + 'src/sidecar.rs', 'w').write(side)
open(W + 'src/notifications.rs', 'w').write('''use chrono::{DateTime, Utc};
use std::collections::HashSet;
use uuid::Uuid;
pub const CREATE_NO_WINDOW: u32 = 0;
pub const SYNC_TASK: &str = "\\\\JobScraper\\\\JobScraper-Sync";
pub const STARTUP_TASK: &str = "\\\\JobScraper\\\\JobScraper-Startup";
pub const STARTUP_ARGUMENT: &str = "--hidden";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder { pub id: String, pub due_at: DateTime<Utc>, pub status: String, pub kind: String }
pub trait Scheduler {
    fn create(&self, reminder: &Reminder) -> Result<String, String>;
    fn cancel(&self, task_id: &str) -> Result<(), String>;
    fn exists(&self, task_id: &str) -> Result<bool, String>;
    fn list_managed(&self) -> Result<Vec<String>, String>;
}
#[derive(Clone)]
pub struct WindowsTaskScheduler { pub executable: String }
#[derive(Debug, serde::Serialize)] #[serde(rename_all = "camelCase")]
pub struct TaskStatus { pub enabled: bool, pub issue: Option<String>, pub last_result: Option<u32> }
impl WindowsTaskScheduler {
    pub fn packaged(executable: String) -> Self { Self { executable } }
    pub fn status(&self, _t: &str) -> Result<TaskStatus, String> { Ok(TaskStatus { enabled: false, issue: None, last_result: None }) }
    pub fn create_sync(&self) -> Result<(), String> { Ok(()) }
}
impl Scheduler for WindowsTaskScheduler {
    fn create(&self, r: &Reminder) -> Result<String, String> { task_name(&r.id) }
    fn cancel(&self, _t: &str) -> Result<(), String> { Ok(()) }
    fn exists(&self, _t: &str) -> Result<bool, String> { Ok(false) }
    fn list_managed(&self) -> Result<Vec<String>, String> { Ok(vec![]) }
}
pub async fn run_scheduler_blocking<T, F>(operation: F) -> Result<T, String>
where T: Send + 'static, F: FnOnce() -> Result<T, String> + Send + 'static {
    tokio::task::spawn_blocking(operation).await.map_err(|e| format!("worker failed: {e}"))?
}
pub fn task_name(id: &str) -> Result<String, String> {
    Uuid::parse_str(id).map_err(|_| "Reminder ID must be UUID")?;
    Ok(format!("JobScraper-Reminder-{id}"))
}
pub fn is_managed_task_path(value: &str) -> bool { value.starts_with("\\\\JobScraper\\\\JobScraper-Reminder-") }
pub fn scoped_orphans(managed: &[String], expected: &HashSet<String>) -> Vec<String> {
    managed.iter().filter(|t| !expected.contains(*t)).cloned().collect()
}
#[derive(serde::Serialize)] #[serde(rename_all = "camelCase")]
pub struct LaunchAtLoginStatus { enabled: bool, debug_build: bool }
#[tauri::command]
pub async fn launch_at_login_status() -> Result<LaunchAtLoginStatus, String> { Ok(LaunchAtLoginStatus { enabled: false, debug_build: true }) }
#[tauri::command]
pub async fn set_launch_at_login(_enabled: bool) -> Result<LaunchAtLoginStatus, String> { Ok(LaunchAtLoginStatus { enabled: false, debug_build: true }) }
pub async fn refresh_login_tasks() -> Result<bool, String> { Ok(false) }
''')
P
cd "$W/src-tauri" && CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/jobscraper-rust-target}" cargo "$@"
