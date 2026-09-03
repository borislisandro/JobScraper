use crate::{db::ApiResult, AppState};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    os::windows::process::CommandExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
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
    domains: Mutex<HashSet<String>>,
    /// Runs this process killed on purpose. A cancelled worker is killed where it stands, so it
    /// never gets to say it was cancelled, and its non-zero exit is indistinguishable from a crash
    /// unless we remember that we were the ones who ended it.
    cancelled: Mutex<HashSet<String>>,
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
struct RunFinalizer {
    pool: sqlx::SqlitePool,
    run_id: String,
    armed: bool,
}
impl RunFinalizer {
    fn new(pool: sqlx::SqlitePool, run_id: String, armed: bool) -> Self {
        Self {
            pool,
            run_id,
            armed,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for RunFinalizer {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (pool, run_id) = (self.pool.clone(), self.run_id.clone());
        tauri::async_runtime::spawn(async move {
            if let Ok(mut tx) = pool.begin().await {
                let _ = sqlx::query("DELETE FROM job_occurrences WHERE run_id=?")
                    .bind(&run_id)
                    .execute(&mut *tx)
                    .await;
                let _ = sqlx::query("UPDATE scrape_runs SET status='failed',complete=0,failure_code='internal_error',diagnostics='The run ended before a terminal result; partial sightings were discarded.',finished_at=? WHERE id=? AND status='running'")
                    .bind(chrono::Utc::now().to_rfc3339())
                    .bind(&run_id)
                    .execute(&mut *tx)
                    .await;
                let _ = tx.commit().await;
            }
        });
    }
}
struct WorkerFinalizer {
    state: Arc<AppState>,
    run_id: String,
    armed: bool,
}
impl WorkerFinalizer {
    fn new(state: Arc<AppState>, run_id: String) -> Self {
        Self {
            state,
            run_id,
            armed: true,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for WorkerFinalizer {
    fn drop(&mut self) {
        if self.armed {
            self.state.sidecars.cancel(&self.run_id);
            self.state.sidecars.remove(&self.run_id);
        }
    }
}
struct DomainFinalizer {
    state: Arc<AppState>,
    domain: String,
}
impl Drop for DomainFinalizer {
    fn drop(&mut self) {
        self.state.sidecars.release_domain(&self.domain);
    }
}
impl SidecarManager {
    pub fn active(&self) -> bool {
        !self.workers.lock().unwrap().is_empty()
    }
    fn claim_domain(&self, domain: &str) -> bool {
        domain.is_empty() || self.domains.lock().unwrap().insert(domain.to_owned())
    }
    fn release_domain(&self, domain: &str) {
        if !domain.is_empty() {
            self.domains.lock().unwrap().remove(domain);
        }
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
        if !batches.is_empty() {
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
    /// True once, for a run this process cancelled: the answer is taken out of the set so a later
    /// run reusing the id — or a second read of the same one — cannot inherit it.
    pub fn was_cancelled(&self, id: &str) -> bool {
        self.cancelled.lock().unwrap().remove(id)
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
                .creation_flags(crate::notifications::CREATE_NO_WINDOW)
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
        self.cancelled.lock().unwrap().insert(id.to_string());
        if let Some(pid) = self
            .workers
            .lock()
            .unwrap()
            .get(id)
            .map(|worker| worker.pid)
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(crate::notifications::CREATE_NO_WINDOW)
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
    /// The single vacancy to read, for "capture_job". Every other command works from the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Listing hashes already stored for this source. The worker skips the detail page of any
    /// listing whose hash it recognises, which is the difference between a Workday tenant taking
    /// half an hour and taking half a minute. Empty means read everything.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub known: Vec<String>,
    /// Lower-cased title fragments selected by the user. The worker applies these while it still
    /// has only the cheap listing row, before any description request is made.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub title_terms: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub defer_details: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WorkerEvent {
    pub protocol_version: u8,
    pub event: String,
    pub run_id: String,
    pub payload: serde_json::Value,
}
fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
fn between(start: Instant, end: Instant) -> u64 {
    millis(end.saturating_duration_since(start))
}
/// The five exclusive phases of one command, cut from consecutive instants so they sum to the
/// total without double-counting: nested work (`workMs`) overlaps these and is never added to them.
fn critical_path(marks: [Instant; 6]) -> serde_json::Value {
    let [app_started, spawned_at, startup_end, active_end, child_exited_at, snapshot_at] = marks;
    serde_json::json!({
        "prepare": between(app_started, spawned_at),
        "startup": between(spawned_at, startup_end),
        "active": between(startup_end, active_end),
        "shutdown": between(active_end, child_exited_at),
        "finalize": between(child_exited_at, snapshot_at)
    })
}
fn attach_app_performance(payload: &mut serde_json::Value, app: serde_json::Value) {
    let Some(payload) = payload.as_object_mut() else {
        return;
    };
    let performance = payload
        .entry("performance")
        .or_insert_with(|| serde_json::json!({ "version": 1 }));
    if !performance.is_object() {
        *performance = serde_json::json!({ "version": 1 });
    }
    let performance = performance.as_object_mut().unwrap();
    performance.insert("version".into(), serde_json::json!(1));
    performance.insert("app".into(), app);
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
        ("apple", &["direct-json","pagination"][..], &[][..]),
        ("workday", &["direct-json","pagination"][..], &[][..]),
        ("eightfold", &["direct-json","pagination"][..], &[][..]),
        ("icims", &["direct-json","pagination"][..], &[][..]),
        ("talentbrew-jibe", &["direct-json","pagination"][..], &[][..]),
        ("phenom", &["direct-json","pagination"][..], &[][..]),
    ].into_iter().map(|(id,modes,required_fields)| AdapterManifest { id,version:"1.1.0",modes,required_fields,config_schema:serde_json::json!({"type":"object","properties":{"urlTemplate":{"type":"string"},"query":{"type":"string"},"maxPages":{"type":"integer","minimum":1,"maximum":50}},"required":required_fields}) }).collect()
}
/// Tauri's `resource_dir()` returns a Windows verbatim path (`\\?\C:\...`). Rust and the OS accept
/// it, but Node reads `\\?\C:\...` as a UNC share and tries to `lstat "C:"`, so every sidecar
/// launch died before it could load worker.mjs. Strip the prefix before handing paths to Node.
pub fn simplified(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(match rest.strip_prefix(r"UNC\") {
            Some(share) => format!(r"\\{share}"),
            None => rest.to_owned(),
        }),
        None => path,
    }
}
fn worker_paths(app: &tauri::AppHandle) -> ApiResult<(PathBuf, PathBuf)> {
    let root = simplified(
        app.path()
            .resource_dir()
            .unwrap_or_else(|_| std::env::current_dir().unwrap()),
    );
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
    full_refresh: bool,
) -> ApiResult<Vec<WorkerEvent>> {
    let app_started = Instant::now();
    let mut config_load_ms = 0u64;
    let mut filter_load_ms = 0u64;
    let mut session_load_ms = 0u64;
    let mut known_hash_load_ms = 0u64;
    let run_id = Uuid::new_v4().to_string();
    let source_id = source
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let source_name = source
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let domain = source
        .get("baseUrl")
        .and_then(|value| value.as_str())
        .and_then(|value| url::Url::parse(value).ok())
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    if !state.sidecars.claim_domain(&domain) {
        return Err(format!(
            "Another operation is already reading {domain}; wait for it to finish."
        ));
    }
    let _domain_finalizer = DomainFinalizer {
        state: state.clone(),
        domain,
    };
    if !source_id.is_empty() {
        let started = Instant::now();
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
        config_load_ms = millis(started.elapsed());
    }
    if matches!(
        command,
        "scrape_source" | "capture_session" | "enrich_source"
    ) && source_id.is_empty()
    {
        return Err("A saved source is required for persistence".into());
    };
    // The scrape-time filter is read once per run: it decides what is written, so it is read
    // before anything is.
    let filter_terms = if matches!(command, "scrape_source" | "check_source") {
        let started = Instant::now();
        let terms = crate::db::scrape_filter_terms(
            &sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
                .bind(crate::db::SCRAPE_TITLE_FILTER)
                .fetch_optional(&state.db.pool)
                .await
                .map_err(|e| e.to_string())?
                .unwrap_or_default(),
        );
        filter_load_ms = millis(started.elapsed());
        terms
    } else {
        Vec::new()
    };
    let temp_session = if !source_id.is_empty() && command != "capture_session" {
        let started = Instant::now();
        if let Some(plain) = crate::sessions::load_browser_session(&state.db.root, &source_id)? {
            let path = state
                .db
                .root
                .join("sessions")
                .join(format!("{run_id}.playwright.json"));
            std::fs::write(&path, plain).map_err(|e| e.to_string())?;
            source["sessionStatePath"] = serde_json::json!(path.to_string_lossy());
            session_load_ms = millis(started.elapsed());
            Some(path)
        } else {
            session_load_ms = millis(started.elapsed());
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
    let mut run_finalizer = RunFinalizer::new(state.db.pool.clone(), run_id.clone(), track_run);
    let (node, script) = worker_paths(&app)?;
    // Node caps response headers at 16KB and fetch() rejects the whole response past that with
    // an opaque UND_ERR_HEADERS_OVERFLOW. www.u-blox.com alone sends a 19KB content-security-policy
    // header, so its pages were unreadable for reasons no adapter could see or report.
    let mut child = Command::new(node)
        .arg("--max-http-header-size=65536")
        .creation_flags(crate::notifications::CREATE_NO_WINDOW)
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start isolated scraper: {e}"))?;
    let spawned_at = Instant::now();
    let pid = child.id().ok_or("Worker did not return a PID")?;
    // Only a real scrape reuses what is already stored. A preview reads the board as it is, and a
    // full refresh re-reads every listing row. Detail pages still wait for their cards to open.
    let known: Vec<String> = if matches!(command, "scrape_source" | "check_source") && !full_refresh
    {
        let started = Instant::now();
        let hashes = sqlx::query_scalar(
            "SELECT listing_hash FROM jobs WHERE source_id=? AND listing_hash IS NOT NULL AND availability<>'archived'",
        )
        .bind(&source_id)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
        known_hash_load_ms = millis(started.elapsed());
        hashes
    } else {
        Vec::new()
    };
    let request = WorkerRequest {
        protocol_version: 1,
        url: source
            .get("captureUrl")
            .and_then(|value| value.as_str())
            .map(str::to_owned),
        command: command.into(),
        run_id: run_id.clone(),
        source,
        known,
        title_terms: filter_terms.clone(),
        defer_details: command == "scrape_source",
    };
    let mut stdin = child.stdin.take().ok_or("Worker stdin unavailable")?;
    let (control, mut control_rx) = mpsc::unbounded_channel::<String>();
    state
        .sidecars
        .add(run_id.clone(), pid, control, batch_id.map(str::to_owned));
    let mut worker_finalizer = WorkerFinalizer::new(state.clone(), run_id.clone());
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
    // stderr was piped and never read, so a worker that died before emitting an event (a
    // stale bundle, a missing module) surfaced only as "exited with exit code: 1". Drain it
    // concurrently: an unread pipe also blocks the child once its buffer fills.
    let mut worker_stderr = child.stderr.take().ok_or("Worker stderr unavailable")?;
    let stderr_task = tokio::spawn(async move {
        let mut text = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut worker_stderr, &mut text).await;
        text
    });
    let output = child.stdout.take().ok_or("Worker stdout unavailable")?;
    let mut lines = BufReader::new(output).lines();
    let mut events = Vec::new();
    let mut filtered_out: i64 = 0;
    let mut skipped: i64 = 0;
    let mut pending_jobs = Vec::with_capacity(200);
    let mut persisted_written: i64 = 0;
    let mut persistence_ms: u128 = 0;
    let mut sighting_persistence_ms: u128 = 0;
    let mut enrichment_persistence_ms: u128 = 0;
    let mut enriched_jobs = Vec::new();
    let mut enrichment_failures = Vec::new();
    let mut worker_started_at = None;
    let mut terminal_at = None;
    let mut terminal_event_id = None;
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let mut event: WorkerEvent = serde_json::from_str(&line)
            .map_err(|_| format!("Worker emitted invalid JSONL: {line}"))?;
        if event.event == "started" && worker_started_at.is_none() {
            worker_started_at = Some(Instant::now());
        }
        if command == "scrape_source" && event.event == "job" {
            let title = event
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if crate::db::title_passes_scrape_filter(title, &filter_terms) {
                pending_jobs.push(event.payload.clone());
                if pending_jobs.len() >= 200 {
                    let started = Instant::now();
                    let result = state
                        .db
                        .persist_worker_jobs(&run_id, &source_id, &pending_jobs)
                        .await?;
                    persistence_ms += started.elapsed().as_millis();
                    persisted_written += i64::from(result.written);
                    skipped += i64::from(result.unchanged);
                    pending_jobs.clear();
                }
            } else {
                filtered_out += 1;
            }
        }
        if command == "enrich_source" {
            match event.event.as_str() {
                "enriched_job" => enriched_jobs.push(event.payload.clone()),
                "enrichment_failed" => enrichment_failures.push(event.payload.clone()),
                _ => {}
            }
        }
        // Unchanged listings arrive once per worker, not once per job. They are persisted together
        // so a warm run pays one transaction and produces no database/UI event storm.
        if command == "scrape_source" && event.event == "seen_batch" {
            let started = Instant::now();
            let hashes = event
                .payload
                .get("listingHashes")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            skipped += state
                .db
                .record_jobs_seen(&run_id, &source_id, &hashes)
                .await? as i64;
            sighting_persistence_ms += started.elapsed().as_millis();
        }
        let finished = matches!(event.event.as_str(), "completed" | "failed" | "cancelled");
        if finished && !pending_jobs.is_empty() {
            let started = Instant::now();
            let result = state
                .db
                .persist_worker_jobs(&run_id, &source_id, &pending_jobs)
                .await?;
            persistence_ms += started.elapsed().as_millis();
            persisted_written += i64::from(result.written);
            skipped += i64::from(result.unchanged);
            pending_jobs.clear();
        }
        if finished && command == "scrape_source" {
            if let Some(payload) = event.payload.as_object_mut() {
                payload.insert("persisted".into(), serde_json::json!(persisted_written));
                payload.insert("persistenceMs".into(), serde_json::json!(persistence_ms));
                payload.insert("skipped".into(), serde_json::json!(skipped));
            }
        }
        if finished && command == "enrich_source" {
            let started = Instant::now();
            let written = state
                .db
                .persist_enriched_jobs(&source_id, &enriched_jobs)
                .await?;
            let failed = state
                .db
                .record_enrichment_failures(&source_id, &enrichment_failures)
                .await?;
            let elapsed = started.elapsed().as_millis();
            persistence_ms += elapsed;
            enrichment_persistence_ms += elapsed;
            if let Some(payload) = event.payload.as_object_mut() {
                payload.insert("persisted".into(), serde_json::json!(written));
                payload.insert("failed".into(), serde_json::json!(failed));
                payload.insert("persistenceMs".into(), serde_json::json!(persistence_ms));
            }
        }
        // "job" and "progress" fire once per discovered job and would dominate the event log
        // without adding diagnostic value beyond what jobs/job_occurrences already record.
        if track_run && !matches!(event.event.as_str(), "job" | "progress" | "seen_batch") {
            let level = match event.event.as_str() {
                "warning" => "warning",
                "failed" => "error",
                _ => "info",
            };
            let event_id = Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO scrape_run_events(id,run_id,level,event_type,payload_json,created_at) VALUES(?,?,?,?,?,?)")
                .bind(&event_id)
                .bind(&run_id)
                .bind(level)
                .bind(&event.event)
                .bind(event.payload.to_string())
                .bind(chrono::Utc::now().to_rfc3339())
                .execute(&state.db.pool)
                .await
                .map_err(|e| e.to_string())?;
            if finished {
                terminal_event_id = Some(event_id);
            }
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
        if event.event == "warning" {
            crate::db::log(
                &state.db.pool,
                "warning",
                opt(&source_id),
                opt(&source_name),
                command,
                event.payload.get("code").and_then(|v| v.as_str()),
                text(&event.payload, "message"),
                event.payload.clone(),
            )
            .await;
        }
        if event.event == "progress" {
            if let Some(batch_id) = batch_id {
                let _ = app.emit(
                    "scrape-all-progress",
                    serde_json::json!({
                        "runId": batch_id,
                        "phase": if command == "enrich_source" { "description-progress" } else { "source-progress" },
                        "source": &source_name,
                        "requests": event.payload.get("requests").and_then(|value| value.as_i64()).unwrap_or(0),
                        "current": event.payload.get("current").and_then(|value| value.as_i64()),
                        "total": event.payload.get("total").and_then(|value| value.as_i64()),
                        "elapsedMs": event.payload.get("elapsedMs").and_then(|value| value.as_i64())
                    }),
                );
            }
        }
        if event.event != "seen_batch"
            && !(command == "scrape_source" && event.event == "job")
            && !matches!(event.event.as_str(), "enriched_job" | "enrichment_failed")
        {
            app.emit("scrape-event", &event)
                .map_err(|e| e.to_string())?;
        }
        if event.event != "seen_batch"
            && !(command == "scrape_source" && event.event == "job")
            && !matches!(event.event.as_str(), "enriched_job" | "enrichment_failed")
        {
            events.push(event);
        }
        // The worker holds its stdin open so it can still receive resume/cancel, so it never
        // reaches EOF on its own and this loop would wait forever for a stream that stays open.
        // A run ends at its terminal event; stop there and let the worker go.
        if finished {
            terminal_at = Some(Instant::now());
            break;
        }
    }
    // Dropping the control sender ends the writer task, which closes the worker's stdin and lets
    // it exit. Killing after the grace period keeps a wedged worker from blocking the command.
    let status = match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(status) => status.map_err(|e| e.to_string())?,
        Err(_) => {
            let _ = child.kill().await;
            child.wait().await.map_err(|e| e.to_string())?
        }
    };
    let child_exited_at = Instant::now();
    state.sidecars.remove(&run_id);
    worker_finalizer.disarm();
    if let Some(path) = temp_session {
        let _ = std::fs::remove_file(path);
    }
    let mut run_record_ms = 0u64;
    let mut availability_ms = 0u64;
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
        // These four counters were declared when the table was created and never written, so there
        // was no record of what a run cost. requests_count against discovered_count is the whole
        // measure of whether skipping recognised listings is working.
        let count = |key: &str| {
            events
                .last()
                .and_then(|e| e.payload.get(key))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        };
        let discovered = count("discovered");
        let persisted = count("persisted").max(0);
        let record_started = Instant::now();
        sqlx::query("UPDATE scrape_runs SET status=?,complete=?,discovered_count=?,persisted_count=?,skipped_count=?,requests_count=?,finished_at=? WHERE id=?")
            .bind(state_name)
            .bind(complete)
            .bind(discovered)
            .bind(persisted)
            .bind(skipped)
            .bind(count("requests"))
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(&run_id)
            .execute(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?;
        run_record_ms = millis(record_started.elapsed());
        run_finalizer.disarm();
        if command == "scrape_source" && state_name == "completed" && complete {
            let availability_started = Instant::now();
            state
                .db
                .reconcile_availability(&run_id, &source_id, true)
                .await?;
            // A completed full read is the thing the skip budget exists to guarantee, so reaching
            // one spends nothing and starts the streak over.
            sqlx::query(
                "UPDATE sources SET last_success_at=?,updated_at=?,unchanged_checks=0 WHERE id=?",
            )
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(&source_id)
            .execute(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?;
            availability_ms = millis(availability_started.elapsed());
        }
    }
    let snapshot_at = Instant::now();
    let startup_end = worker_started_at.or(terminal_at).unwrap_or(child_exited_at);
    let active_end = terminal_at.unwrap_or(child_exited_at);
    let app_performance = serde_json::json!({
        "totalMs": between(app_started, snapshot_at),
        "criticalPathMs": critical_path([app_started, spawned_at, startup_end, active_end, child_exited_at, snapshot_at]),
        "workMs": {
            "configLoad": config_load_ms,
            "filterLoad": filter_load_ms,
            "sessionLoad": session_load_ms,
            "knownHashLoad": known_hash_load_ms,
            "jobPersistence": if command == "scrape_source" { persistence_ms as u64 } else { 0 },
            "sightingPersistence": sighting_persistence_ms as u64,
            "enrichmentPersistence": enrichment_persistence_ms as u64,
            "runRecord": run_record_ms,
            "availability": availability_ms
        }
    });
    if let Some(event) = events
        .last_mut()
        .filter(|event| matches!(event.event.as_str(), "completed" | "failed" | "cancelled"))
    {
        attach_app_performance(&mut event.payload, app_performance.clone());
        if let Some(event_id) = terminal_event_id.as_deref() {
            sqlx::query("UPDATE scrape_run_events SET payload_json=? WHERE id=?")
                .bind(event.payload.to_string())
                .bind(event_id)
                .execute(&state.db.pool)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    // A worker killed by a cancel exits non-zero without having emitted anything, which read as
    // "Scraper worker exited with exit code: 1" in the activity log — an error, in red, for doing
    // exactly what was asked of it.
    let cancelled_here = state.sidecars.was_cancelled(&run_id);
    if cancelled_here && !status.success() {
        crate::db::log(
            &state.db.pool,
            "info",
            opt(&source_id),
            opt(&source_name),
            command,
            Some("cancelled"),
            "Cancelled before the read finished.",
            serde_json::json!({}),
        )
        .await;
        return Err("cancelled".into());
    }
    if !status.success()
        && !events
            .iter()
            .any(|e| e.event == "failed" || e.event == "cancelled")
    {
        let stderr = stderr_task.await.unwrap_or_default();
        let detail = stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | ");
        let message = if detail.is_empty() {
            format!("Scraper worker exited with {status}")
        } else {
            format!("Scraper worker exited with {status}: {detail}")
        };
        crate::db::log(
            &state.db.pool,
            "error",
            opt(&source_id),
            opt(&source_name),
            command,
            Some("worker_exit"),
            &message,
            serde_json::json!({ "stderr": stderr, "performance": { "version": 1, "app": app_performance } }),
        )
        .await;
        return Err(message);
    }
    if let Some(final_event) = events.last() {
        let level = match final_event.event.as_str() {
            "completed" => "info",
            "cancelled" => "warning",
            _ => "error",
        };
        crate::db::log(
            &state.db.pool,
            level,
            opt(&source_id),
            opt(&source_name),
            command,
            final_event.payload.get("code").and_then(|v| v.as_str()),
            &outcome_message(final_event, command, filtered_out),
            final_event.payload.clone(),
        )
        .await;
    }
    Ok(events)
}
fn opt(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
fn text<'a>(payload: &'a serde_json::Value, key: &str) -> &'a str {
    payload.get(key).and_then(|v| v.as_str()).unwrap_or("")
}
fn duration_text(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        format!("{milliseconds}ms")
    } else if milliseconds < 60_000 {
        format!("{:.1}s", milliseconds as f64 / 1_000.0)
    } else {
        format!(
            "{}m {}s",
            milliseconds / 60_000,
            milliseconds % 60_000 / 1_000
        )
    }
}
fn performance_suffix(payload: &serde_json::Value) -> String {
    let candidates = [
        ("/performance/worker/bucketsMs/response", "site responses"),
        ("/performance/worker/bucketsMs/pacing", "request pacing"),
        ("/performance/worker/bucketsMs/backoff", "retry waits"),
        ("/performance/worker/bucketsMs/body", "response bodies"),
        ("/performance/worker/bucketsMs/urlGuard", "address checks"),
        (
            "/performance/worker/bucketsMs/browser",
            "browser automation",
        ),
        (
            "/performance/worker/bucketsMs/adapterOther",
            "parsing and transformation",
        ),
        ("/performance/worker/bucketsMs/emit", "worker output"),
        ("/performance/worker/bucketsMs/setup", "worker setup"),
        ("/performance/app/workMs/jobPersistence", "database writes"),
        (
            "/performance/app/workMs/sightingPersistence",
            "sighting writes",
        ),
        (
            "/performance/app/workMs/enrichmentPersistence",
            "description writes",
        ),
        ("/performance/app/workMs/knownHashLoad", "known-job lookup"),
        (
            "/performance/app/workMs/availability",
            "availability update",
        ),
        ("/performance/app/criticalPathMs/startup", "worker startup"),
        (
            "/performance/app/criticalPathMs/shutdown",
            "worker shutdown",
        ),
        ("/performance/app/criticalPathMs/prepare", "preparation"),
        ("/performance/app/criticalPathMs/finalize", "finalization"),
    ];
    let total = payload
        .pointer("/performance/app/totalMs")
        .or_else(|| payload.pointer("/performance/worker/totalMs"))
        .and_then(|value| value.as_u64());
    let slowest = candidates
        .into_iter()
        .filter_map(|(path, label)| {
            payload
                .pointer(path)
                .and_then(|value| value.as_u64())
                .map(|value| (label, value))
        })
        .filter(|(_, value)| *value > 0)
        .max_by_key(|(_, value)| *value);
    match (total, slowest) {
        (Some(total), Some((label, value))) => format!(
            " · {} · slowest: {label} ({})",
            duration_text(total),
            duration_text(value)
        ),
        (Some(total), None) => format!(" · {}", duration_text(total)),
        _ => String::new(),
    }
}
/// One readable line per run. The full payload is kept alongside it in detail_json, so the log
/// table stays scannable while nothing diagnostic is lost.
fn outcome_message(event: &WorkerEvent, command: &str, filtered_out: i64) -> String {
    match event.event.as_str() {
        "completed" if command == "probe_source" => match event.payload.get("recommendedAdapter") {
            Some(serde_json::Value::String(adapter)) => {
                format!("Configured automatically as {adapter}.")
            }
            _ => "No job listings could be read from this page.".into(),
        },
        "completed" if command == "enrich_source" => {
            let saved = event
                .payload
                .get("persisted")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let failed = event
                .payload
                .get("failed")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let requests = event
                .payload
                .get("requests")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            format!(
                "Descriptions finished: {saved} saved, {failed} failed · {requests} requests{}.",
                performance_suffix(&event.payload)
            )
        }
        // A run that stores far fewer jobs than it found now says which of the two filter layers
        // did it, so a too-narrow title filter never looks like a broken scraper.
        "completed" => {
            let found = event
                .payload
                .get("discovered")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let saved = event
                .payload
                .get("persisted")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let unchanged = event
                .payload
                .get("skipped")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let requests = event
                .payload
                .get("requests")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let worker_filtered = event
                .payload
                .get("filtered")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let filtered = worker_filtered + filtered_out;
            let mut parts = vec![format!("{found} listings found")];
            parts.push(format!("{saved} saved"));
            if unchanged > 0 {
                parts.push(format!("{unchanged} already up to date"));
            }
            if filtered > 0 {
                parts.push(format!("{filtered} skipped by the title filter"));
            }
            format!(
                "Finished: {} · {requests} requests{}.",
                parts.join(", "),
                performance_suffix(&event.payload)
            )
        }
        "cancelled" => "Cancelled.".into(),
        _ => {
            let message = text(&event.payload, "message");
            if message.is_empty() {
                event.event.clone()
            } else {
                message.to_owned()
            }
        }
    }
}
#[tauri::command]
pub async fn test_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(
        app,
        state.inner().clone(),
        "test_source",
        source,
        None,
        false,
    )
    .await
}
/// Reads one vacancy from its own page. A referral or a recruiter's link is a single opening on a
/// board nobody has configured, and configuring a whole source for one job is the wrong trade —
/// so this reads the page as a job. Nothing is stored yet: what comes back is a draft the person
/// pasting the link can correct before saving, because a page without the standard markup gives a
/// title and little else.
#[tauri::command]
pub async fn capture_job(
    url: String,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(
        app,
        state.inner().clone(),
        "capture_job",
        serde_json::json!({
            "name": "Captured link",
            "baseUrl": url,
            "captureUrl": url,
            "adapterId": "static-css",
            "kind": "active",
            // A link someone chose to paste is a page they want read; the board's crawl policy is
            // about crawling it, not about opening one vacancy a person is already looking at.
            "robotsOverride": true,
            "configJson": { "maxPages": 1 }
        }),
        None,
        false,
    )
    .await
}
#[tauri::command]
pub async fn probe_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(
        app,
        state.inner().clone(),
        "probe_source",
        source,
        None,
        false,
    )
    .await
}
#[tauri::command]
pub async fn scrape_source(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    full_refresh: Option<bool>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(
        app,
        state.inner().clone(),
        "scrape_source",
        source,
        None,
        full_refresh.unwrap_or(false),
    )
    .await
}
/// How many hosts are read at once during a batch, and the setting that overrides it. Five keeps
/// eighteen sources moving without pointing more than a handful of requests at the network at any
/// moment; a single host is always read serially regardless of this.
const DEFAULT_SCRAPE_LANES: usize = 5;
pub const SCRAPE_LANES: &str = "scrape.lanes";
/// What one source's change check found. `fresh` is the count of listings on the board's first
/// page that are not already stored; `board_total` is an exact vendor count. The count is compared
/// with the latest complete scrape's discovered count, never with the title-filtered jobs table.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCheck {
    pub source_id: String,
    pub name: String,
    pub fresh: i64,
    pub board_total: Option<i64>,
    pub stored: i64,
    pub changed: bool,
    pub conclusive: bool,
    pub requests: i64,
    pub error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeAllResult {
    pub run_id: String,
    pub completed_sources: usize,
    pub unchanged_sources: usize,
    pub failed_sources: usize,
    pub cancelled_sources: usize,
}

#[tauri::command]
pub async fn job_description(
    job_id: String,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<crate::db::JobDescription> {
    let current = crate::db::load_job_description(&state.db.pool, &job_id).await?;
    if current.status == "complete" {
        return Ok(current);
    }
    let Some((source_id, job)) = state.db.pending_enrichment_job(&job_id).await? else {
        return Ok(current);
    };
    let mut source = saved_sources(&state, &[source_id])
        .await?
        .into_iter()
        .next()
        .ok_or("This job's source is not available")?
        .source;
    source["enrichmentJobs"] = serde_json::json!([job]);
    run(
        app,
        state.inner().clone(),
        "enrich_source",
        source,
        None,
        false,
    )
    .await?;
    crate::db::load_job_description(&state.db.pool, &job_id).await
}

#[derive(Clone)]
struct SavedSource {
    order: usize,
    source_id: String,
    name: String,
    domain: String,
    source: serde_json::Value,
    stored: i64,
    baseline: Option<i64>,
    attempted: bool,
    /// Consecutive times this source's full read was skipped because its total had not moved.
    unchanged_checks: i64,
}

fn comparison_baseline(attempted: bool, stored: i64, complete: Option<i64>) -> Option<i64> {
    complete.or_else(|| attempted.then_some(stored))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckDecision {
    Unchanged,
    Changed,
    Indeterminate,
    Failed,
}

fn classify_check(
    fresh: i64,
    board_total: Option<i64>,
    baseline: Option<i64>,
    failed: bool,
) -> CheckDecision {
    if failed {
        CheckDecision::Failed
    } else if fresh > 0 {
        CheckDecision::Changed
    } else if let (Some(total), Some(baseline)) = (board_total, baseline) {
        if total == baseline {
            CheckDecision::Unchanged
        } else {
            CheckDecision::Changed
        }
    } else if board_total.is_some() {
        CheckDecision::Changed
    } else {
        CheckDecision::Indeterminate
    }
}

/// How many times running a source may be skipped on the strength of "the total did not move"
/// before it is read in full regardless.
///
/// ponytail: a streak counter bounds the damage, it does not remove it. The real fix is to make
/// the check page date-ordered so `fresh` alone answers the question — Apple already asks for
/// `sort:"newest"` and MediaTek for `publishedDate DESC`, and every other adapter takes whatever
/// the board's default sort is. Adding a sort parameter that a tenant silently ignores would make
/// the check look sound while staying unsound, so that needs verification against live tenants
/// rather than a guess here.
const MAX_UNCHANGED_SKIPS: i64 = 3;

/// Whether a check result may stand in for a full read.
///
/// Two things this deliberately refuses to do. It never skips on a failed check: a timeout or a
/// 429 on the cheapest request the app makes is not evidence about the board, and letting it veto
/// the full read means the one request with retries and backoff behind it never happens. And it
/// never skips a source whose "unchanged" streak has run out, however confident the check is.
fn may_skip(decision: CheckDecision, unchanged_checks: i64, limit: i64) -> bool {
    decision == CheckDecision::Unchanged && unchanged_checks < limit
}

/// `only` narrows a run to the sources the Jobs page is currently showing. An empty list is not a
/// narrowing to nothing — it is the default, meaning every source the user still follows. Which
/// sources are followed at all stays `sources.enabled`, owned by the Sources page; this is the
/// separate, throwaway question of what is being looked at right now.
async fn saved_sources(state: &AppState, only: &[String]) -> ApiResult<Vec<SavedSource>> {
    let rows = sqlx::query(
        "SELECT s.id,s.name,s.base_url,s.adapter_id,s.adapter_version,s.robots_override,s.unchanged_checks,\
         (SELECT count(*) FROM jobs j WHERE j.source_id=s.id AND j.availability<>'archived') AS stored,\
         (SELECT r.discovered_count FROM scrape_runs r WHERE r.source_id=s.id AND r.status='completed' AND r.complete=1 ORDER BY r.started_at DESC LIMIT 1) AS baseline,\
         EXISTS(SELECT 1 FROM scrape_runs r WHERE r.source_id=s.id AND r.mode='scrape') AS attempted \
         FROM sources s WHERE s.enabled=1 AND s.kind='active' AND s.deleted_at IS NULL ORDER BY s.name,s.id",
    )
    .fetch_all(&state.db.pool)
    .await
    .map_err(|e| e.to_string())?;
    rows.into_iter()
        .filter(|row| only.is_empty() || only.iter().any(|id| *id == row.get::<String, _>("id")))
        .enumerate()
        .map(|(order, row)| {
            let source_id: String = row.get("id");
            let name: String = row.get("name");
            let base_url: String = row.get("base_url");
            let domain = url::Url::parse(&base_url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .ok_or_else(|| format!("Saved source {name} has an invalid URL"))?;
            let stored: i64 = row.get("stored");
            let baseline: Option<i64> = row.get("baseline");
            // Old runs never populated discovered_count. Zero is therefore not evidence that a
            // non-empty source has a valid baseline.
            let baseline = baseline.filter(|count| *count > 0 || stored == 0);
            let source = serde_json::json!({
                "id": source_id,
                "name": name,
                "baseUrl": base_url,
                "adapterId": row.get::<String,_>("adapter_id"),
                "adapterVersion": row.get::<String,_>("adapter_version"),
                "enabled": true,
                "kind": "active",
                "robotsOverride": row.get::<bool,_>("robots_override")
            });
            Ok(SavedSource {
                order,
                source_id,
                name,
                domain,
                source,
                stored,
                baseline,
                attempted: row.get::<bool, _>("attempted"),
                unchanged_checks: row.get::<i64, _>("unchanged_checks"),
            })
        })
        .collect()
}

fn domain_groups(sources: Vec<SavedSource>) -> Vec<Vec<SavedSource>> {
    let mut groups = BTreeMap::<String, Vec<SavedSource>>::new();
    for source in sources {
        groups
            .entry(source.domain.clone())
            .or_default()
            .push(source);
    }
    groups.into_values().collect()
}

async fn lane_count(state: &AppState, domains: usize) -> ApiResult<usize> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
            .bind(SCRAPE_LANES)
            .fetch_optional(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_SCRAPE_LANES)
            .clamp(1, 12)
            .min(domains.max(1)),
    )
}

async fn check_one(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    source: &SavedSource,
    batch_id: Option<&str>,
) -> SourceCheck {
    let events = run(
        app,
        state,
        "check_source",
        source.source.clone(),
        batch_id,
        false,
    )
    .await;
    let (fresh, board_total, requests, error) = match events {
        Ok(events) => match events.last() {
            Some(event) if event.event == "completed" => {
                let read = |key: &str| event.payload.get(key).and_then(|value| value.as_i64());
                let exact = event
                    .payload
                    .get("boardTotalExact")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                (
                    read("fresh").unwrap_or(0),
                    exact.then(|| read("boardTotal")).flatten(),
                    read("requests").unwrap_or(0),
                    None,
                )
            }
            Some(event) => (
                0,
                None,
                0,
                Some(
                    event
                        .payload
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("This source could not be read.")
                        .to_owned(),
                ),
            ),
            None => (0, None, 0, Some("The scraper returned nothing.".into())),
        },
        Err(message) => (0, None, 0, Some(message)),
    };
    let decision = classify_check(
        fresh,
        board_total,
        comparison_baseline(source.attempted, source.stored, source.baseline),
        error.is_some(),
    );
    SourceCheck {
        source_id: source.source_id.clone(),
        name: source.name.clone(),
        fresh,
        board_total,
        stored: source.stored,
        changed: decision == CheckDecision::Changed,
        conclusive: matches!(decision, CheckDecision::Changed | CheckDecision::Unchanged),
        requests,
        error,
    }
}
/// Answers "is there anything new?" for every enabled source without reading any of them in
/// full: one page each, compared against what is stored. A source that reports no change can be
/// left alone, which is the difference between a minute and an afternoon.
#[tauri::command]
pub async fn check_sources(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    source_ids: Option<Vec<String>>,
) -> ApiResult<Vec<SourceCheck>> {
    let started = Instant::now();
    let sources = saved_sources(&state, &source_ids.unwrap_or_default()).await?;
    let source_count = sources.len();
    let groups = domain_groups(sources);
    let lanes = lane_count(&state, groups.len()).await?;
    let active_started = Instant::now();
    let queue = Arc::new(Mutex::new(VecDeque::from(groups)));
    let mut tasks = Vec::new();
    for _ in 0..lanes {
        let (app, state, queue) = (app.clone(), state.inner().clone(), queue.clone());
        tasks.push(tokio::spawn(async move {
            let mut checks = Vec::new();
            while let Some(sources) = {
                let next = queue.lock().unwrap().pop_front();
                next
            } {
                for source in sources {
                    let check = check_one(app.clone(), state.clone(), &source, None).await;
                    checks.push((source.order, check));
                }
            }
            checks
        }));
    }
    let mut checks = Vec::new();
    for task in futures::future::join_all(tasks).await {
        checks.extend(task.map_err(|e| e.to_string())?);
    }
    let active_finished = Instant::now();
    checks.sort_by_key(|(order, _)| *order);
    let checks = checks
        .into_iter()
        .map(|(_, check)| check)
        .collect::<Vec<_>>();
    let requests = checks
        .iter()
        .map(|check| check.requests.max(0) as u64)
        .sum::<u64>();
    let failed = checks.iter().filter(|check| check.error.is_some()).count();
    let changed = checks.iter().filter(|check| check.changed).count();
    let conclusive = checks.iter().filter(|check| check.conclusive).count();
    let finished = Instant::now();
    crate::db::log(
        &state.db.pool,
        "info",
        None,
        None,
        "check_sources",
        None,
        &format!("Checked {source_count} sources in {:.1}s · {requests} requests · {failed} failed.", finished.duration_since(started).as_secs_f64()),
        serde_json::json!({
            "sources": source_count,
            "changedSources": changed,
            "conclusiveSources": conclusive,
            "failedSources": failed,
            "requests": requests,
            "peakDomains": lanes,
            "performance": {"version":1,"app":{
                "totalMs": between(started,finished),
                "criticalPathMs": critical_path([started,active_started,active_started,active_finished,active_finished,finished]),
                "workMs": {"sourceChecks": between(active_started,active_finished)}
            }}
        }),
    ).await;
    Ok(checks)
}
#[tauri::command]
pub async fn scrape_all(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    run_id: Option<String>,
    source_ids: Option<Vec<String>>,
    force_refresh: Option<bool>,
) -> ApiResult<ScrapeAllResult> {
    scrape_all_inner(
        app,
        state.inner().clone(),
        run_id,
        source_ids.unwrap_or_default(),
        force_refresh.unwrap_or(false),
    )
    .await
}

/// Hosts are read by a fixed number of lanes pulling from a shared queue. Each host remains
/// sequential, enforcing one worker stream per domain while distinct companies run in parallel.
async fn scrape_all_inner(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    run_id: Option<String>,
    source_ids: Vec<String>,
    force_refresh: bool,
) -> ApiResult<ScrapeAllResult> {
    let started = Instant::now();
    let batch_id = run_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let sources = saved_sources(&state, &source_ids).await?;
    let total = sources.len();
    let groups = domain_groups(sources);
    // Sources used to run in fixed pairs, and each pair had to finish before the next began, so a
    // two-second board sat waiting on a two-hour one and eighteen sources meant nine such waits.
    // Lanes now pull from a shared queue: a lane that finishes a host immediately takes the next.
    // One host is still read strictly serially, by itself, which is what politeness requires.
    let lanes = lane_count(&state, groups.len()).await?;
    if !state.sidecars.start_batch(&batch_id) {
        return Err("Another source update is already running".into());
    }
    let active_started = Instant::now();
    let queue = Arc::new(Mutex::new(VecDeque::from(groups)));
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let unchanged = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let failed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cancelled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let preflight_ms = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let scrape_ms = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let active_domains = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak_domains = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..lanes {
        let (app, state, batch_id) = (app.clone(), state.clone(), batch_id.clone());
        let (
            queue,
            done,
            unchanged,
            failed,
            cancelled,
            requests,
            preflight_ms,
            scrape_ms,
            active_domains,
            peak_domains,
        ) = (
            queue.clone(),
            done.clone(),
            unchanged.clone(),
            failed.clone(),
            cancelled.clone(),
            requests.clone(),
            preflight_ms.clone(),
            scrape_ms.clone(),
            active_domains.clone(),
            peak_domains.clone(),
        );
        tasks.push(tokio::spawn(async move {
            // The guard is dropped before the await below; holding it across one would stall
            // every other lane for the length of a scrape.
            while let Some(sources) = {
                let next = queue.lock().unwrap().pop_front();
                next
            } {
                let active = active_domains.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                peak_domains.fetch_max(active, std::sync::atomic::Ordering::Relaxed);
                for source in sources {
                    let outcome = if state.sidecars.batch_cancelled(&batch_id) {
                        CheckDecision::Failed
                    } else {
                        // New sources bootstrap once. Afterwards even an empty or partial source
                        // gets a cheap check, so a legitimately empty board is not rebuilt every
                        // time the app opens. Complete baselines retain the bounded skip budget.
                        let decision = if force_refresh
                            || !source.attempted
                            || (source.baseline.is_some()
                                && source.unchanged_checks >= MAX_UNCHANGED_SKIPS)
                        {
                            CheckDecision::Changed
                        } else {
                            let check_started = Instant::now();
                            let check = check_one(
                                app.clone(),
                                state.clone(),
                                &source,
                                Some(&batch_id),
                            )
                            .await;
                            preflight_ms.fetch_add(millis(check_started.elapsed()) as usize, std::sync::atomic::Ordering::Relaxed);
                            requests.fetch_add(check.requests.max(0) as usize, std::sync::atomic::Ordering::Relaxed);
                            classify_check(
                                check.fresh,
                                check.board_total,
                                comparison_baseline(
                                    source.attempted,
                                    source.stored,
                                    source.baseline,
                                ),
                                check.error.is_some(),
                            )
                        };
                        if state.sidecars.batch_cancelled(&batch_id) {
                            CheckDecision::Failed
                        } else if may_skip(
                            decision,
                            if source.baseline.is_some() {
                                source.unchanged_checks
                            } else {
                                0
                            },
                            MAX_UNCHANGED_SKIPS,
                        ) {
                            // Spend one skip. The next check compares the same board total
                            // against the same baseline, so without a counter that advances,
                            // a source whose total happens to sit still is never read again.
                            if source.baseline.is_some() {
                                let _ = sqlx::query(
                                    "UPDATE sources SET unchanged_checks=unchanged_checks+1 WHERE id=?",
                                )
                                .bind(&source.source_id)
                                .execute(&state.db.pool)
                                .await;
                            }
                            CheckDecision::Unchanged
                        } else {
                            // Everything else reads the board, a failed check included: a timeout
                            // on the cheapest request the app makes says nothing about the board,
                            // and the full read is the one with retries and backoff behind it.
                            let scrape_started = Instant::now();
                            let result = run(
                                app.clone(),
                                state.clone(),
                                "scrape_source",
                                source.source.clone(),
                                Some(&batch_id),
                                force_refresh || !source.attempted,
                            )
                            .await;
                            scrape_ms.fetch_add(millis(scrape_started.elapsed()) as usize, std::sync::atomic::Ordering::Relaxed);
                            if let Ok(events) = &result {
                                let count = events.last().and_then(|event| event.payload.get("requests")).and_then(|value| value.as_u64()).unwrap_or(0);
                                requests.fetch_add(count as usize, std::sync::atomic::Ordering::Relaxed);
                            }
                            match result {
                                Ok(events)
                                    if events.last().is_some_and(|event| {
                                        event.event == "completed"
                                            && event
                                                .payload
                                                .get("complete")
                                                .and_then(|value| value.as_bool())
                                                .unwrap_or(false)
                                    }) =>
                                {
                                    CheckDecision::Changed
                                }
                                _ => CheckDecision::Failed,
                            }
                        }
                    };
                    if state.sidecars.batch_cancelled(&batch_id) {
                        cancelled.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        match outcome {
                            CheckDecision::Unchanged => {
                                unchanged.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            CheckDecision::Changed => {
                                done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            CheckDecision::Failed | CheckDecision::Indeterminate => {
                                failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                    let read = |c: &std::sync::atomic::AtomicUsize| {
                        c.load(std::sync::atomic::Ordering::Relaxed)
                    };
                    // Progress is now reported the moment a source finishes rather than when its
                    // pair does, so the count moves steadily instead of in jumps.
                    let _=app.emit("scrape-all-progress",serde_json::json!({"runId":batch_id,"completed":read(&done)+read(&unchanged)+read(&failed)+read(&cancelled),"total":total,"unchanged":read(&unchanged),"failed":read(&failed),"cancelled":read(&cancelled)}));
                }
                active_domains.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            }
        }));
    }
    for task in futures::future::join_all(tasks).await {
        task.map_err(|e| e.to_string())?;
    }
    let active_finished = Instant::now();
    let load =
        |c: &Arc<std::sync::atomic::AtomicUsize>| c.load(std::sync::atomic::Ordering::Relaxed);
    let (done, unchanged, failed, cancelled) = (
        load(&done),
        load(&unchanged),
        load(&failed),
        load(&cancelled),
    );
    let batch_finished = Instant::now();
    let batch_requests = load(&requests);
    let batch_preflight_ms = load(&preflight_ms);
    let batch_scrape_ms = load(&scrape_ms);
    let batch_peak_domains = load(&peak_domains);
    crate::db::log(
        &state.db.pool,
        if failed > 0 { "warning" } else { "info" },
        None,
        None,
        "scrape_all",
        None,
        &format!("Updated {total} sources in {:.1}s · {batch_requests} requests · {unchanged} unchanged · {failed} failed.", batch_finished.duration_since(started).as_secs_f64()),
        serde_json::json!({
            "sources": total,
            "completedSources": done,
            "unchangedSources": unchanged,
            "failedSources": failed,
            "cancelledSources": cancelled,
            "requests": batch_requests,
            "peakDomains": batch_peak_domains,
            "performance": {"version":1,"app":{
                "totalMs": between(started,batch_finished),
                "criticalPathMs": critical_path([started,active_started,active_started,active_finished,active_finished,batch_finished]),
                "workMs": {"preflight":batch_preflight_ms,"scrape":batch_scrape_ms}
            }}
        }),
    ).await;
    state.sidecars.finish_batch(&batch_id);
    Ok(ScrapeAllResult {
        run_id: batch_id,
        completed_sources: done,
        unchanged_sources: unchanged,
        failed_sources: failed,
        cancelled_sources: cancelled,
    })
}

/// Builds one compact toast, or stays silent when the batch stored nothing new.
pub fn new_jobs_summary(count: i64, samples: &[crate::db::NewJob]) -> Option<(String, String)> {
    if count <= 0 {
        return None;
    }
    let title = if count == 1 {
        "1 new job".to_string()
    } else {
        format!("{count} new jobs")
    };
    let mut lines: Vec<String> = samples
        .iter()
        .take(3)
        .map(|job| format!("{} — {}", job.title, job.company))
        .collect();
    let remaining = count - lines.len() as i64;
    if remaining > 0 {
        lines.push(format!("and {remaining} more"));
    }
    Some((title, lines.join("\n")))
}
pub async fn notify_new_jobs(
    app: &tauri::AppHandle,
    pool: &sqlx::SqlitePool,
    since: &str,
) -> ApiResult<()> {
    let (count, jobs) = crate::db::new_jobs_since(pool, since, 3).await?;
    let Some((title, body)) = new_jobs_summary(count, &jobs) else {
        crate::db::log(
            pool,
            "info",
            None,
            None,
            "new_jobs_alert",
            None,
            "No new jobs found; no notification shown.",
            serde_json::json!({"count": 0}),
        )
        .await;
        return Ok(());
    };
    match app
        .notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
    {
        Ok(()) => {
            crate::db::log(
                pool,
                "info",
                None,
                None,
                "new_jobs_alert",
                None,
                &format!(
                    "Announced {count} new job{}.",
                    if count == 1 { "" } else { "s" }
                ),
                serde_json::json!({"count": count, "samples": jobs}),
            )
            .await;
            Ok(())
        }
        Err(error) => {
            let error = error.to_string();
            crate::db::log(
                pool,
                "error",
                None,
                None,
                "new_jobs_alert",
                None,
                &error,
                serde_json::json!({"count": count}),
            )
            .await;
            Err(error)
        }
    }
}
pub async fn run_automatic_sync(app: tauri::AppHandle, state: Arc<AppState>) -> ApiResult<bool> {
    if crate::db::sync_in_progress(&state.db.pool).await {
        return Ok(false);
    }
    let since = chrono::Utc::now().to_rfc3339();
    crate::db::write_sync_heartbeat(&state.db.pool).await;
    let heartbeat_pool = state.db.pool.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            crate::db::write_sync_heartbeat(&heartbeat_pool).await;
        }
    });
    let result = scrape_all_inner(
        app.clone(),
        state.clone(),
        Some("automatic-startup".into()),
        Vec::new(),
        false,
    )
    .await;
    heartbeat.abort();
    let _ = heartbeat.await;
    crate::db::clear_sync_heartbeat(&state.db.pool).await;
    let alert = notify_new_jobs(&app, &state.db.pool, &since).await;
    result?;
    alert?;
    Ok(true)
}

#[tauri::command]
pub async fn start_automatic_sync(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<bool> {
    if state.headless || crate::db::sync_in_progress(&state.db.pool).await {
        return Ok(false);
    }
    if state
        .automatic_sync_started
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(false);
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_automatic_sync(app, state.clone()).await {
            crate::db::log(
                &state.db.pool,
                "error",
                None,
                None,
                "automatic_sync",
                None,
                &error,
                serde_json::json!({}),
            )
            .await;
        }
    });
    Ok(true)
}
#[tauri::command]
pub async fn capture_session(
    source: serde_json::Value,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<WorkerEvent>> {
    run(
        app,
        state.inner().clone(),
        "capture_session",
        source,
        None,
        false,
    )
    .await
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
    fn new_jobs_summary_stays_silent_at_zero() {
        let samples = [
            crate::db::NewJob {
                title: "Firmware Engineer".into(),
                company: "Chip Co".into(),
            },
            crate::db::NewJob {
                title: "Embedded Engineer".into(),
                company: "Board Co".into(),
            },
            crate::db::NewJob {
                title: "Verification Engineer".into(),
                company: "Core Co".into(),
            },
        ];
        assert!(new_jobs_summary(0, &samples).is_none());
        let one = new_jobs_summary(1, &samples[..1]).unwrap();
        assert_eq!(one.0, "1 new job");
        let seven = new_jobs_summary(7, &samples).unwrap();
        assert_eq!(seven.0, "7 new jobs");
        assert!(seven.1.ends_with("and 4 more"));
    }

    #[test]
    fn batch_cancellation_during_preflight_prevents_dequeue_and_signals_the_check_worker() {
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
    fn checks_classify_unchanged_changed_indeterminate_and_failed_separately() {
        assert_eq!(
            classify_check(0, Some(12), Some(12), false),
            CheckDecision::Unchanged
        );
        assert_eq!(
            classify_check(1, Some(12), Some(12), false),
            CheckDecision::Changed
        );
        assert_eq!(
            classify_check(0, Some(11), Some(12), false),
            CheckDecision::Changed
        );
        assert_eq!(
            classify_check(0, None, Some(12), false),
            CheckDecision::Indeterminate
        );
        assert_eq!(
            classify_check(0, Some(12), Some(12), true),
            CheckDecision::Failed
        );
        assert_eq!(
            classify_check(0, Some(0), comparison_baseline(true, 0, None), false),
            CheckDecision::Unchanged,
            "an attempted empty source is checked against zero instead of bootstrapped again"
        );
        assert_eq!(comparison_baseline(false, 0, None), None);
    }

    fn saved(order: usize, domain: &str) -> SavedSource {
        SavedSource {
            order,
            source_id: order.to_string(),
            name: order.to_string(),
            domain: domain.into(),
            source: serde_json::json!({}),
            stored: 0,
            baseline: Some(0),
            attempted: true,
            unchanged_checks: 0,
        }
    }

    #[test]
    fn a_failed_check_never_stands_in_for_the_full_read() {
        // The cheapest request the app makes timing out is not evidence about the board. It used
        // to veto the full read, so one 429 on a preflight skipped that source for the whole batch.
        assert!(!may_skip(CheckDecision::Failed, 0, MAX_UNCHANGED_SKIPS));
        assert!(!may_skip(
            CheckDecision::Indeterminate,
            0,
            MAX_UNCHANGED_SKIPS
        ));
        assert!(!may_skip(CheckDecision::Changed, 0, MAX_UNCHANGED_SKIPS));
    }

    #[test]
    fn an_unchanged_streak_is_spent_and_then_forces_a_full_read() {
        // One opening closing while another opens leaves the total identical, so a board sorted by
        // anything but date reports "unchanged" forever. The budget is what ends that.
        for streak in 0..MAX_UNCHANGED_SKIPS {
            assert!(
                may_skip(CheckDecision::Unchanged, streak, MAX_UNCHANGED_SKIPS),
                "skip {streak} is still within budget"
            );
        }
        assert!(!may_skip(
            CheckDecision::Unchanged,
            MAX_UNCHANGED_SKIPS,
            MAX_UNCHANGED_SKIPS
        ));
        assert!(!may_skip(
            CheckDecision::Unchanged,
            MAX_UNCHANGED_SKIPS + 5,
            MAX_UNCHANGED_SKIPS
        ));
    }

    #[test]
    fn domain_grouping_is_stable_and_keeps_each_domain_serial() {
        let groups = domain_groups(vec![
            saved(0, "b.test"),
            saved(1, "a.test"),
            saved(2, "b.test"),
        ]);
        assert_eq!(
            groups
                .iter()
                .map(|group| group.iter().map(|source| source.order).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![vec![1], vec![0, 2]]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn five_lanes_run_concurrently_with_only_one_stream_per_domain() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let groups = (0..9)
            .map(|domain| vec![(domain, domain * 2), (domain, domain * 2 + 1)])
            .collect::<Vec<_>>();
        let queue = Arc::new(Mutex::new(VecDeque::from(groups)));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let overlap = Arc::new(AtomicBool::new(false));
        let active_domains = Arc::new(Mutex::new(HashSet::new()));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let (queue, active, maximum, overlap, active_domains, completed) = (
                queue.clone(),
                active.clone(),
                maximum.clone(),
                overlap.clone(),
                active_domains.clone(),
                completed.clone(),
            );
            tasks.push(tokio::spawn(async move {
                while let Some(group) = {
                    let next = queue.lock().unwrap().pop_front();
                    next
                } {
                    let domain = group[0].0;
                    if !active_domains.lock().unwrap().insert(domain) {
                        overlap.store(true, Ordering::SeqCst);
                    }
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(now, Ordering::SeqCst);
                    for (_, order) in group {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        completed.lock().unwrap().push(order);
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                    active_domains.lock().unwrap().remove(&domain);
                }
            }));
        }
        futures::future::join_all(tasks).await;
        assert_eq!(maximum.load(Ordering::SeqCst), 5);
        assert!(!overlap.load(Ordering::SeqCst));
        let mut ordered = completed.lock().unwrap().clone();
        ordered.sort_unstable();
        assert_eq!(ordered, (0..18).collect::<Vec<_>>());
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

    #[test]
    fn domain_claims_prevent_manual_and_batch_workers_from_overlapping() {
        let manager = SidecarManager::default();
        assert!(manager.claim_domain("jobs.example.test"));
        assert!(!manager.claim_domain("jobs.example.test"));
        assert!(manager.claim_domain("other.example.test"));
        manager.release_domain("jobs.example.test");
        assert!(manager.claim_domain("jobs.example.test"));
    }
}

#[cfg(test)]
mod worker_path_tests {
    use super::*;
    #[test]
    fn verbatim_resource_paths_are_simplified_for_node() {
        // Node reads a verbatim path as a UNC share and lstats "C:", which killed every sidecar
        // launch with an EISDIR before worker.mjs ran.
        assert_eq!(
            simplified(PathBuf::from(r"\\?\C:\Program Files\JobScraper")),
            PathBuf::from(r"C:\Program Files\JobScraper")
        );
        assert_eq!(
            simplified(PathBuf::from(r"\\?\UNC\server\share\JobScraper")),
            PathBuf::from(r"\\server\share\JobScraper")
        );
        // Ordinary paths pass through untouched.
        assert_eq!(
            simplified(PathBuf::from(r"C:\JobScraper")),
            PathBuf::from(r"C:\JobScraper")
        );
    }
}

#[cfg(test)]
mod performance_metric_tests {
    use super::*;

    fn marks(offsets: [u64; 6]) -> [Instant; 6] {
        let origin = Instant::now();
        offsets.map(|milliseconds| origin + Duration::from_millis(milliseconds))
    }

    #[test]
    fn the_five_critical_path_phases_sum_to_the_total() {
        let all = marks([0, 40, 340, 12_240, 12_300, 12_460]);
        let path = critical_path(all);
        let total = between(all[0], all[5]);
        let sum: u64 = ["prepare", "startup", "active", "shutdown", "finalize"]
            .iter()
            .map(|key| path[key].as_u64().unwrap())
            .sum();
        assert_eq!(sum, total, "no phase is double counted or lost");
        assert_eq!(path["prepare"], 40);
        assert_eq!(path["startup"], 300);
        assert_eq!(path["active"], 11_900);
        assert_eq!(path["shutdown"], 60);
        assert_eq!(path["finalize"], 160);
    }

    #[test]
    fn a_batch_reports_its_own_phases_without_a_worker_lifecycle() {
        let all = marks([0, 500, 500, 9_500, 9_500, 9_800]);
        let path = critical_path(all);
        assert_eq!(path["startup"], 0);
        assert_eq!(path["shutdown"], 0);
        assert_eq!(path["prepare"], 500);
        assert_eq!(path["active"], 9_000);
        assert_eq!(path["finalize"], 300);
    }

    #[test]
    fn app_metrics_join_the_worker_metrics_on_the_terminal_event() {
        let mut payload = serde_json::json!({
            "complete": true,
            "performance": { "version": 1, "worker": { "totalMs": 900 } }
        });
        attach_app_performance(&mut payload, serde_json::json!({ "totalMs": 1_100 }));
        assert_eq!(
            payload["performance"]["worker"]["totalMs"], 900,
            "worker metrics survive"
        );
        assert_eq!(payload["performance"]["app"]["totalMs"], 1_100);
        assert_eq!(payload["performance"]["version"], 1);
    }

    #[test]
    fn a_crash_with_no_worker_metrics_still_records_the_app_side() {
        let mut payload = serde_json::json!({ "code": "worker_crashed" });
        attach_app_performance(&mut payload, serde_json::json!({ "totalMs": 300 }));
        assert_eq!(payload["performance"]["version"], 1);
        assert_eq!(payload["performance"]["app"]["totalMs"], 300);
        assert!(payload["performance"].get("worker").is_none());
        // A payload that is not an object cannot carry metrics and must not panic.
        let mut broken = serde_json::json!("crashed");
        attach_app_performance(&mut broken, serde_json::json!({}));
        assert_eq!(broken, serde_json::json!("crashed"));
    }

    #[test]
    fn the_activity_line_names_the_slowest_measured_step() {
        let payload = serde_json::json!({
            "performance": { "version": 1,
                "worker": { "totalMs": 12_000, "bucketsMs": { "response": 6_100, "pacing": 3_000, "body": 400 } },
                "app": { "totalMs": 12_400, "criticalPathMs": { "prepare": 40, "startup": 300, "active": 11_900, "shutdown": 60, "finalize": 100 },
                         "workMs": { "jobPersistence": 120 } } }
        });
        let suffix = performance_suffix(&payload);
        assert!(
            suffix.contains("12.4s"),
            "the total elapsed time is reported: {suffix}"
        );
        assert!(
            suffix.contains("site responses"),
            "the slowest measured step is named: {suffix}"
        );
        assert!(
            !suffix.contains("worker running"),
            "the parent phase is never the answer: {suffix}"
        );
        // A legacy payload has nothing to rank and says nothing rather than "0ms".
        assert_eq!(
            performance_suffix(&serde_json::json!({ "complete": true })),
            ""
        );
    }
}
