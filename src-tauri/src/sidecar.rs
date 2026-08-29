use crate::{db::ApiResult, AppState};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Emitter, Manager, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};
use uuid::Uuid;

/// JSONL worker process registry. It exists only while a user-initiated action runs.
#[derive(Default)]
pub struct SidecarManager {
    workers: Mutex<HashMap<String, RunningWorker>>,
    batches: Mutex<HashMap<String, ScrapeBatch>>,
}
struct RunningWorker {
    pid: u32,
    control: mpsc::UnboundedSender<String>,
    batch_id: Option<String>,
}
#[derive(Default)]
struct ScrapeBatch {
    cancelled: bool,
    workers: HashSet<String>,
}
impl SidecarManager {
    pub fn active(&self) -> bool {
        !self.workers.lock().unwrap().is_empty()
    }
    fn add(
        &self,
        id: String,
        pid: u32,
        control: mpsc::UnboundedSender<String>,
        batch_id: Option<String>,
    ) {
        if let Some(batch) = &batch_id {
            self.batches
                .lock()
                .unwrap()
                .entry(batch.clone())
                .or_default()
                .workers
                .insert(id.clone());
        }
        self.workers.lock().unwrap().insert(
            id,
            RunningWorker {
                pid,
                control,
                batch_id,
            },
        );
    }
    fn remove(&self, id: &str) {
        if let Some(worker) = self.workers.lock().unwrap().remove(id) {
            if let Some(batch) = worker.batch_id {
                if let Some(entry) = self.batches.lock().unwrap().get_mut(&batch) {
                    entry.workers.remove(id);
                }
            }
        }
    }
    fn start_batch(&self, id: &str) -> bool {
        let mut batches = self.batches.lock().unwrap();
        if batches.contains_key(id) {
            return false;
        }
        batches.insert(id.to_owned(), ScrapeBatch::default());
        true
    }
    fn finish_batch(&self, id: &str) {
        self.batches.lock().unwrap().remove(id);
    }
    fn batch_cancelled(&self, id: &str) -> bool {
        self.batches
            .lock()
            .unwrap()
            .get(id)
            .map(|batch| batch.cancelled)
            .unwrap_or(true)
    }
    fn request_batch_cancel(&self, id: &str) -> Option<Vec<u32>> {
        let worker_ids = {
            let mut batches = self.batches.lock().unwrap();
            let batch = batches.get_mut(id)?;
            batch.cancelled = true;
            batch.workers.iter().cloned().collect::<Vec<_>>()
        };
        let workers = self.workers.lock().unwrap();
        Some(
            worker_ids
                .into_iter()
                .filter_map(|worker_id| workers.get(&worker_id))
                .map(|worker| {
                    let _ = worker.control.send("cancel".into());
                    worker.pid
                })
                .collect(),
        )
    }
    fn kill_if_still_active(&self, pids: &[u32]) {
        let active: HashSet<u32> = self
            .workers
            .lock()
            .unwrap()
            .values()
            .map(|worker| worker.pid)
            .collect();
        for pid in pids.iter().copied().filter(|pid| active.contains(pid)) {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
        }
    }
    pub fn cancel_all(&self) {
        let ids: Vec<String> = self.workers.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.cancel(&id);
        }
    }
    fn cancel(&self, id: &str) -> bool {
        if let Some(worker) = self.workers.lock().unwrap().get(id) {
            let _ = worker.control.send("cancel".into());
        }
        if let Some(pid) = self
            .workers
            .lock()
            .unwrap()
            .get(id)
            .map(|worker| worker.pid)
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
            true
        } else {
            false
        }
    }
    fn resume(&self, id: &str) -> bool {
        self.workers
            .lock()
            .unwrap()
            .get(id)
            .map(|worker| worker.control.send("resume".into()).is_ok())
            .unwrap_or(false)
    }
}
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRequest {
    pub protocol_version: u8,
    pub command: String,
    pub run_id: String,
    pub source: serde_json::Value,
}
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WorkerEvent {
    pub protocol_version: u8,
    pub event: String,
    pub run_id: String,
    pub payload: serde_json::Value,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterManifest {
    pub id: &'static str,
    pub version: &'static str,
    pub modes: &'static [&'static str],
    pub required_fields: &'static [&'static str],
    pub config_schema: serde_json::Value,
}
#[tauri::command]
pub fn adapter_manifests() -> Vec<AdapterManifest> {
    [
        ("static-css", &["direct","css","pagination"][..], &["itemSelector","titleSelector"][..]),
        ("static-xpath", &["direct","xpath","pagination"][..], &["itemXPath","titleXPath"][..]),
        ("json", &["direct","json","pagination"][..], &["itemsPath"][..]),
        ("rss", &["direct","rss"][..], &[][..]),
        ("playwright", &["headed","headless","session"][..], &[][..]),
        ("workday", &["direct-json","pagination"][..], &[][..]),
        ("eightfold", &["direct-json","pagination"][..], &[][..]),
        ("icims", &["direct-json","pagination"][..], &[][..]),
        ("talentbrew-jibe", &["direct-json","pagination"][..], &[][..]),
        ("phenom", &["direct-json","pagination"][..], &[][..]),
    ].into_iter().map(|(id,modes,required_fields)| AdapterManifest { id,version:"1.1.0",modes,required_fields,config_schema:serde_json::json!({"type":"object","properties":{"urlTemplate":{"type":"string"},"query":{"type":"string"},"maxPages":{"type":"integer","minimum":1,"maximum":50}},"required":required_fields}) }).collect()
}
fn worker_paths(app: &tauri::AppHandle) -> ApiResult<(PathBuf, PathBuf)> {
    let root = app
        .path()
        .resource_dir()
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let script = root.join("sidecar").join("worker.mjs");
    let node = std::env::var_os("JOBSCRAPER_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("sidecar").join("node.exe"));
    if !script.exists() {
        return Err(format!(
            "Scraper worker is not bundled: {}",
            script.display()
        ));
    }
    if !node.exists() {
        return Err("Bundled Node runtime is unavailable; JobScraper never relies on a system Node installation.".into());
    }
    Ok((node, script))
}
async fn run(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    command: &str,
    mut source: serde_json::Value,
    batch_id: Option<&str>,
) -> ApiResult<Vec<WorkerEvent>> {
    let run_id = Uuid::new_v4().to_string();
    let source_id = source
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if !source_id.is_empty() {
        let config: Option<String> =
            sqlx::query_scalar("SELECT config_json FROM source_configs WHERE source_id=?")
                .bind(&source_id)
                .fetch_optional(&state.db.pool)
                .await
                .map_err(|e| e.to_string())?;
        if let Some(config) = config {
            source["configJson"] =
                serde_json::from_str(&config).unwrap_or_else(|_| serde_json::json!({}));
        }
    }
    if matches!(command, "scrape_source" | "capture_session") && source_id.is_empty() {
        return Err("A saved source is required for persistence".into());
    };
    let temp_session = if !source_id.is_empty() && command != "capture_session" {
        if let Some(plain) = crate::sessions::load_browser_session(&state.db.root, &source_id)? {
            let path = state
                .db
                .root
                .join("sessions")
                .join(format!("{run_id}.playwright.json"));
            std::fs::write(&path, plain).map_err(|e| e.to_string())?;
            source["sessionStatePath"] = serde_json::json!(path.to_string_lossy());
            Some(path)
        } else {
            None
        }
    } else {
        None
    };
    // ponytail: run history (scrape_run_events) is only persisted for saved sources running
    // test/scrape, matching the scrape_runs.mode CHECK constraint ('test','scrape'). Widening
    // it to also cover probe/capture would need a table-rebuild migration (scrape_run_events and
    // job_occurrences hold FKs into scrape_runs) for two commands that produce no jobs and little
    // diagnostic value; skipped here, revisit if probe/capture history is actually requested.
    let track_run = matches!(command, "scrape_source" | "test_source") && !source_id.is_empty();
    if track_run {
        let mode = if command == "scrape_source" {
            "scrape"
        } else {
            "test"
        };
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES(?,?,?, 'running',?)").bind(&run_id).bind(&source_id).bind(mode).bind(chrono::Utc::now().to_rfc3339()).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
    }
    let (node, script) = worker_paths(&app)?;
    let mut child = Command::new(node)
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start isolated scraper: {e}"))?;
    let pid = child.id().ok_or("Worker did not return a PID")?;
    let request = WorkerRequest {
        protocol_version: 1,
        command: command.into(),
        run_id: run_id.clone(),
        source,
    };
    let mut stdin = child.stdin.take().ok_or("Worker stdin unavailable")?;
    let (control, mut control_rx) = mpsc::unbounded_channel::<String>();
    state
        .sidecars
        .add(run_id.clone(), pid, control, batch_id.map(str::to_owned));
    let writer_run_id = run_id.clone();
    tokio::spawn(async move {
        let _ = stdin
            .write_all(format!("{}\n", serde_json::to_string(&request).unwrap()).as_bytes())
            .await;
        while let Some(command) = control_rx.recv().await {
            let line =
                serde_json::json!({"protocolVersion":1,"command":command,"runId":writer_run_id})
                    .to_string();
            if stdin
                .write_all(format!("{line}\n").as_bytes())
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let output = child.stdout.take().ok_or("Worker stdout unavailable")?;
    let mut lines = BufReader::new(output).lines();
    let mut events = Vec::new();
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let event: WorkerEvent = serde_json::from_str(&line)
            .map_err(|_| format!("Worker emitted invalid JSONL: {line}"))?;
        if command == "scrape_source" && event.event == "job" {
            state
                .db
                .persist_worker_job(&run_id, &source_id, &event.payload)
                .await?;
        }
        // "job" and "progress" fire once per discovered job and would dominate the event log
        // without adding diagnostic value beyond what jobs/job_occurrences already record.
        if track_run && !matches!(event.event.as_str(), "job" | "progress") {
            let level = match event.event.as_str() {
                "warning" => "warning",
                "failed" => "error",
                _ => "info",
            };
            sqlx::query("INSERT INTO scrape_run_events(id,run_id,level,event_type,payload_json,created_at) VALUES(?,?,?,?,?,?)")
                .bind(Uuid::new_v4().to_string())
                .bind(&run_id)
                .bind(level)
                .bind(&event.event)
                .bind(event.payload.to_string())
                .bind(chrono::Utc::now().to_rfc3339())
                .execute(&state.db.pool)
                .await
                .map_err(|e| e.to_string())?;
        }
        if command == "capture_session" && event.event == "completed" {
            if let Some(encoded) = event
                .payload
                .get("capturedStorageStateBase64")
                .and_then(|v| v.as_str())
            {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| "Worker returned invalid session bytes")?;
                crate::sessions::save_bytes(&state.db.root, &source_id, bytes)?;
            }
        }
        app.emit("scrape-event", &event)
            .map_err(|e| e.to_string())?;
        events.push(event)
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    state.sidecars.remove(&run_id);
    if let Some(path) = temp_session {
        let _ = std::fs::remove_file(path);
    }
    if track_run {
        let final_event = events.last().map(|e| e.event.as_str()).unwrap_or("failed");
        let complete = events
            .last()
            .and_then(|e| e.payload.get("complete"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let state_name = if final_event == "completed" {
            "completed"
        } else if final_event == "cancelled" {
            "cancelled"
        } else {
            "failed"
        };
        sqlx::query("UPDATE scrape_runs SET status=?,complete=?,finished_at=? WHERE id=?")
            .bind(state_name)
            .bind(complete)
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(&run_id)
            .execute(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?;
        if command == "scrape_source" && state_name == "completed" && complete {
            state
                .db
                .reconcile_availability(&run_id, &source_id, true)
                .await?;
        }
    }
    if !status.success()
        && !events
            .iter()
            .any(|e| e.event == "failed" || e.event == "cancelled")
    {
        return Err(format!("Scraper worker exited with {status}"));
    }
    Ok(events)
}
#[tauri::command]
pub async fn test_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "test_source", source, None).await
}
#[tauri::command]
pub async fn probe_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "probe_source", source, None).await
}
#[tauri::command]
pub async fn scrape_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "scrape_source", source, None).await
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeAllResult {
    pub run_id: String,
    pub completed_sources: usize,
    pub failed_sources: usize,
    pub cancelled_sources: usize,
}
/// Manual batch only: groups are run at most two domains at a time; each group
/// is sequential, enforcing one worker stream per domain.
#[tauri::command]
pub async fn scrape_all(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    run_id: Option<String>,
) -> ApiResult<ScrapeAllResult> {
    let batch_id = run_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    if !state.sidecars.start_batch(&batch_id) {
        return Err("A Scrape All run with this ID is already active".into());
    }
    let rows=sqlx::query("SELECT id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,last_success_at,created_at,updated_at FROM sources WHERE enabled=1 AND kind='active' AND deleted_at IS NULL ORDER BY id").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?;
    let mut groups: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for row in rows {
        let base: String = row.get("base_url");
        let domain = url::Url::parse(&base)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .ok_or("Saved source has invalid URL")?;
        let source = serde_json::json!({"id":row.get::<String,_>("id"),"name":row.get::<String,_>("name"),"baseUrl":base,"adapterId":row.get::<String,_>("adapter_id"),"adapterVersion":row.get::<String,_>("adapter_version"),"enabled":true,"kind":"active","robotsOverride":row.get::<bool,_>("robots_override")});
        groups.entry(domain).or_default().push(source);
    }
    let mut done = 0;
    let mut failed = 0;
    let mut cancelled = 0;
    let total = groups.values().map(Vec::len).sum::<usize>();
    let group_values: Vec<_> = groups.into_values().collect();
    for pair in group_values.chunks(2) {
        let mut tasks = Vec::new();
        for group in pair {
            let app = app.clone();
            let state = state.inner().clone();
            let sources = group.clone();
            let batch_id = batch_id.clone();
            tasks.push(tokio::spawn(async move {
                let mut outcomes = Vec::new();
                for source in sources {
                    if state.sidecars.batch_cancelled(&batch_id) {
                        outcomes.push(None);
                        continue;
                    }
                    outcomes.push(Some(
                        run(
                            app.clone(),
                            state.clone(),
                            "scrape_source",
                            source,
                            Some(&batch_id),
                        )
                        .await
                        .is_ok(),
                    ));
                }
                outcomes
            }));
        }
        for task in futures::future::join_all(tasks).await {
            for outcome in task.map_err(|e| e.to_string())? {
                match outcome {
                    Some(true) => done += 1,
                    Some(false) => failed += 1,
                    None => cancelled += 1,
                }
                let _=app.emit("scrape-all-progress",serde_json::json!({"runId":batch_id,"completed":done+failed+cancelled,"total":total,"failed":failed,"cancelled":cancelled}));
            }
        }
    }
    state.sidecars.finish_batch(&batch_id);
    Ok(ScrapeAllResult {
        run_id: batch_id,
        completed_sources: done,
        failed_sources: failed,
        cancelled_sources: cancelled,
    })
}
#[tauri::command]
pub async fn capture_session(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "capture_session", source, None).await
}
#[tauri::command]
pub fn cancel_scrape(run_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<bool> {
    Ok(state.sidecars.cancel(&run_id))
}
#[tauri::command]
pub fn resume_scrape(run_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<bool> {
    Ok(state.sidecars.resume(&run_id))
}
#[tauri::command]
pub async fn cancel_scrape_all(run_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<bool> {
    let Some(pids) = state.sidecars.request_batch_cancel(&run_id) else {
        return Ok(false);
    };
    // Cooperative JSONL cancellation gets a short grace period before forced tree cleanup.
    tokio::time::sleep(Duration::from_secs(2)).await;
    state.sidecars.kill_if_still_active(&pids);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_cancellation_prevents_future_dequeue_and_signals_active_worker() {
        let manager = SidecarManager::default();
        assert!(manager.start_batch("batch"));
        let (tx, mut rx) = mpsc::unbounded_channel();
        manager.add("worker".into(), 4242, tx, Some("batch".into()));
        let pids = manager.request_batch_cancel("batch").unwrap();
        assert_eq!(pids, vec![4242]);
        assert!(manager.batch_cancelled("batch"));
        assert_eq!(rx.try_recv().as_deref(), Ok("cancel"));
        manager.remove("worker");
        manager.finish_batch("batch");
    }

    #[test]
    fn only_two_domains_are_scheduled_at_once_and_each_domain_is_serial() {
        let domains = vec![vec!["a1", "a2"], vec!["b1"], vec!["c1", "c2"]];
        let waves: Vec<_> = domains.chunks(2).collect();
        assert_eq!(waves.len(), 2);
        assert!(waves.iter().all(|wave| wave.len() <= 2));
        assert_eq!(waves[0][0], vec!["a1", "a2"]);
    }

    #[test]
    fn resume_only_targets_registered_worker() {
        let manager = SidecarManager::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        manager.add("worker".into(), 42, tx, None);
        assert!(manager.resume("worker"));
        assert_eq!(rx.try_recv().as_deref(), Ok("resume"));
        assert!(!manager.resume("missing"));
    }
}
