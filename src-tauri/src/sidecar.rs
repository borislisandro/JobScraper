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
};
use uuid::Uuid;

/// JSONL worker process registry. It exists only while a user-initiated action runs.
#[derive(Default)]
pub struct SidecarManager {
    pids: Mutex<HashMap<String, u32>>,
}
impl SidecarManager {
    pub fn active(&self) -> bool {
        !self.pids.lock().unwrap().is_empty()
    }
    fn add(&self, id: String, pid: u32) {
        self.pids.lock().unwrap().insert(id, pid);
    }
    fn remove(&self, id: &str) {
        self.pids.lock().unwrap().remove(id);
    }
    pub fn cancel_all(&self) {
        let ids: Vec<String> = self.pids.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.cancel(&id);
        }
    }
    fn cancel(&self, id: &str) -> bool {
        if let Some(pid) = self.pids.lock().unwrap().remove(id) {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
            true
        } else {
            false
        }
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
    state.sidecars.add(run_id.clone(), pid);
    let request = WorkerRequest {
        protocol_version: 1,
        command: command.into(),
        run_id: run_id.clone(),
        source,
    };
    let mut stdin = child.stdin.take().ok_or("Worker stdin unavailable")?;
    stdin
        .write_all(format!("{}\n", serde_json::to_string(&request).unwrap()).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    stdin.shutdown().await.map_err(|e| e.to_string())?;
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
pub async fn scrape_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(app, state.inner().clone(), "scrape_source", source).await
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
