use crate::{db::ApiResult, AppState};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{Emitter, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};
use uuid::Uuid;

/// JSONL worker process registry. It exists only while a user-initiated action runs.
#[derive(Default)]
pub struct SidecarManager {
    pids: Mutex<HashMap<String, u32>>,
    controls: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
}
impl SidecarManager {
    pub fn active(&self) -> bool {
        !self.pids.lock().unwrap().is_empty()
    }
    fn add(&self, id: String, pid: u32, control: mpsc::UnboundedSender<String>) {
        self.pids.lock().unwrap().insert(id, pid);
        self.controls.lock().unwrap().insert(id, control);
    }
    fn remove(&self, id: &str) {
        self.pids.lock().unwrap().remove(id);
        self.controls.lock().unwrap().remove(id);
    }
    pub fn cancel_all(&self) {
        let ids: Vec<String> = self.pids.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.cancel(&id);
        }
    }
    fn cancel(&self, id: &str) -> bool {
        if let Some(control) = self.controls.lock().unwrap().get(id) {
            let _ = control.send("cancel".into());
        }
        if let Some(pid) = self.pids.lock().unwrap().remove(id) {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
            true
        } else {
            false
        }
    }
    fn resume(&self, id: &str) -> bool {
        self.controls
            .lock()
            .unwrap()
            .get(id)
            .map(|control| control.send("resume".into()).is_ok())
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
    if command == "scrape_source" {
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES(?,?, 'scrape','running',?)").bind(&run_id).bind(&source_id).bind(chrono::Utc::now().to_rfc3339()).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
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
    state.sidecars.add(run_id.clone(), pid, control);
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
    if command == "scrape_source" {
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
        if state_name == "completed" && complete {
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
    run(app, state.inner().clone(), "test_source", source).await
}
#[tauri::command]
pub async fn probe_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "probe_source", source).await
}
#[tauri::command]
pub async fn scrape_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "scrape_source", source).await
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeAllResult {
    pub run_id: String,
    pub completed_sources: usize,
    pub failed_sources: usize,
}
/// Manual batch only: groups are run at most two domains at a time; each group
/// is sequential, enforcing one worker stream per domain.
#[tauri::command]
pub async fn scrape_all(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ScrapeAllResult> {
    let batch_id = Uuid::new_v4().to_string();
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
    let total = groups.values().map(Vec::len).sum::<usize>();
    let group_values: Vec<_> = groups.into_values().collect();
    for pair in group_values.chunks(2) {
        let mut tasks = Vec::new();
        for group in pair {
            let app = app.clone();
            let state = state.inner().clone();
            let sources = group.clone();
            tasks.push(tokio::spawn(async move {
                let mut outcomes = Vec::new();
                for source in sources {
                    outcomes.push(
                        run(app.clone(), state.clone(), "scrape_source", source)
                            .await
                            .is_ok(),
                    );
                }
                outcomes
            }));
        }
        for task in futures::future::join_all(tasks).await {
            for outcome in task.map_err(|e| e.to_string())? {
                if outcome {
                    done += 1
                } else {
                    failed += 1
                };
                let _=app.emit("scrape-all-progress",serde_json::json!({"runId":batch_id,"completed":done+failed,"total":total,"failed":failed}));
            }
        }
    }
    Ok(ScrapeAllResult {
        run_id: batch_id,
        completed_sources: done,
        failed_sources: failed,
    })
}
#[tauri::command]
pub async fn capture_session(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "capture_session", source).await
}
#[tauri::command]
pub fn cancel_scrape(run_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<bool> {
    Ok(state.sidecars.cancel(&run_id))
}
#[tauri::command]
pub fn resume_scrape(run_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<bool> {
    Ok(state.sidecars.resume(&run_id))
}
