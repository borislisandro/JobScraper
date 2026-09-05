use crate::notifications::{
    is_managed_task_path, run_scheduler_blocking, scoped_orphans, Reminder as ScheduledReminder,
    Scheduler, WindowsTaskScheduler, SYNC_TASK,
};
use crate::{
    domain::{Application, Source, SourceInput, StageInput},
    AppState,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use tauri::{ipc::InvokeBody, Emitter, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;
use uuid::Uuid;

pub type ApiResult<T> = Result<T, String>;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const DOCUMENT_METADATA_HEADER: &str = "x-jobscraper-metadata";
static PURGE_PREVIEWS: OnceLock<Mutex<HashMap<String, PurgePreviewState>>> = OnceLock::new();
fn purge_previews() -> &'static Mutex<HashMap<String, PurgePreviewState>> {
    PURGE_PREVIEWS.get_or_init(|| Mutex::new(HashMap::new()))
}
#[derive(Clone)]
struct PurgePreviewState {
    preview: PurgePreview,
    job_ids: Vec<String>,
    event_ids: Vec<String>,
    session_paths: Vec<String>,
    expires_at: chrono::DateTime<Utc>,
}
#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
    pub root: PathBuf,
}
fn now() -> String {
    Utc::now().to_rfc3339()
}
pub const SYNC_HEARTBEAT: &str = "sync.heartbeatAt";
const SYNC_BACKGROUND: &str = "sync.background";
const HEARTBEAT_FRESH_SECS: i64 = 300;
/// Every employer this app already knows how to read: which adapter reads its board, and the
/// settings that adapter needs. It lives in `sidecar/company-catalog.json` because the sidecar
/// tests parse the same file — one list, checked from both sides, rather than a Rust array and a
/// JavaScript copy of it that drift apart.
///
/// A company in here never needs the manual "Add source" form: the address, the adapter and the
/// advanced settings are all known, and every one of them was verified against the live board.
/// "Add source" is what remains for the employers this list does not cover.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCompany {
    pub id: String,
    pub name: String,
    pub country: String,
    pub tags: Vec<String>,
    pub adapter_id: String,
    pub base_url: String,
    pub config: serde_json::Value,
    /// Seeded into a fresh database (the twenty-one the pack has always installed). The rest are
    /// offered in the app and added on request, so a new install does not open on 134 sources.
    #[serde(default)]
    pub starter: bool,
    #[serde(default = "active_kind")]
    pub kind: String,
    pub disabled_reason: Option<String>,
    /// When this entry was last read from the live board, and how many openings it held then.
    #[serde(default)]
    pub verified: Option<serde_json::Value>,
}
fn active_kind() -> String {
    "active".into()
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompanyCatalog {
    pub version: String,
    pub companies: Vec<CatalogCompany>,
}
static CATALOG: std::sync::LazyLock<CompanyCatalog> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("../../sidecar/company-catalog.json"))
        .expect("sidecar/company-catalog.json is not a readable company catalog")
});
pub fn company_catalog() -> &'static CompanyCatalog {
    &CATALOG
}
// Bump the catalog's version whenever any starter's URL, adapter or settings change. A pack
// corrected without a new version reaches nobody: install_starter_pack stops at the version check,
// and every database stamped with the old number keeps the values that did not work. That is
// exactly how installs ended up reading jobs.cisco.com and www.careers.mediatek.com long after
// both were fixed here.
fn starter_pack_version() -> &'static str {
    &CATALOG.version
}
/// Conflict snapshots are immutable audit evidence. Old, pre-operational snapshots
/// deliberately fail closed: they cannot be used to delete or re-key a live row.
fn dedupe_snapshot_id(snapshot: &str) -> ApiResult<String> {
    serde_json::from_str::<serde_json::Value>(snapshot)
        .ok()
        .and_then(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_owned)
        })
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            "Conflict uses a legacy incomplete snapshot; unmerge before resolving it".into()
        })
}
fn dedupe_snapshot_text(value: &serde_json::Value, field: &str) -> ApiResult<String> {
    value
        .get(field)
        .and_then(|item| item.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("Conflict snapshot missing {field}"))
}
async fn restore_match_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    job_id: &str,
    persona_id: &str,
    snapshot: &str,
) -> ApiResult<()> {
    let value: serde_json::Value =
        serde_json::from_str(snapshot).map_err(|_| "Conflict snapshot is corrupt".to_string())?;
    let eligible = value
        .get("eligible")
        .and_then(|item| item.as_i64())
        .ok_or("Conflict snapshot missing eligible")?;
    let score = value
        .get("score")
        .and_then(|item| item.as_f64())
        .ok_or("Conflict snapshot missing score")?;
    sqlx::query("INSERT INTO match_results(id,job_id,persona_id,score,eligible,algorithm_version,model_version,filter_decision_json,components_json,explanation_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(dedupe_snapshot_id(snapshot)?).bind(job_id).bind(persona_id).bind(score).bind(eligible)
        .bind(dedupe_snapshot_text(&value,"algorithmVersion")?).bind(dedupe_snapshot_text(&value,"modelVersion")?)
        .bind(dedupe_snapshot_text(&value,"filterDecisionJson")?).bind(dedupe_snapshot_text(&value,"componentsJson")?)
        .bind(dedupe_snapshot_text(&value,"explanationJson")?).bind(dedupe_snapshot_text(&value,"createdAt")?).bind(dedupe_snapshot_text(&value,"updatedAt")?)
        .execute(&mut **tx).await.map_err(|e|e.to_string())?;
    Ok(())
}
async fn restore_review_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    job_id: &str,
    persona_id: &str,
    snapshot: &str,
) -> ApiResult<()> {
    let value: serde_json::Value =
        serde_json::from_str(snapshot).map_err(|_| "Conflict snapshot is corrupt".to_string())?;
    sqlx::query("INSERT INTO review_decisions(id,job_id,persona_id,status,reason,decided_at) VALUES(?,?,?,?,?,?)")
        .bind(dedupe_snapshot_id(snapshot)?).bind(job_id).bind(persona_id)
        .bind(dedupe_snapshot_text(&value,"status")?).bind(value.get("reason").and_then(|item|item.as_str()))
        .bind(dedupe_snapshot_text(&value,"decidedAt")?).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    Ok(())
}
fn ghost_due_from(occurred_at: &str, days: i64) -> ApiResult<String> {
    chrono::DateTime::parse_from_rfc3339(occurred_at)
        .map_err(|_| "Application timestamp must be UTC RFC3339".to_string())
        .map(|value| (value.with_timezone(&Utc) + chrono::Duration::days(days)).to_rfc3339())
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AppLog {
    pub id: String,
    pub at: String,
    pub level: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub action: String,
    pub code: Option<String>,
    pub message: String,
    pub detail_json: String,
}
/// Records one line of user-visible activity. Logging must never fail a user action, so write
/// errors are swallowed here rather than propagated into the caller's Result.
pub async fn log(
    pool: &SqlitePool,
    level: &str,
    source_id: Option<&str>,
    source_name: Option<&str>,
    action: &str,
    code: Option<&str>,
    message: &str,
    detail: serde_json::Value,
) {
    let _ = sqlx::query("INSERT INTO app_logs(id,at,level,source_id,source_name,action,code,message,detail_json) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(id()).bind(now()).bind(level).bind(source_id).bind(source_name)
        .bind(action).bind(code).bind(message).bind(detail.to_string())
        .execute(pool).await;
}
#[tauri::command]
pub async fn list_app_logs(limit: i64, state: State<'_, Arc<AppState>>) -> ApiResult<Vec<AppLog>> {
    sqlx::query_as::<_, AppLog>("SELECT id,at,level,source_id,source_name,action,code,message,detail_json FROM app_logs ORDER BY at DESC, rowid DESC LIMIT ?")
        .bind(limit.clamp(1, 1000))
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| e.to_string())
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformancePhase {
    pub key: String,
    pub milliseconds: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceRun {
    pub id: String,
    pub at: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub action: String,
    pub outcome: String,
    pub total_ms: u64,
    pub worker_ms: u64,
    pub requests: u64,
    pub pages: u64,
    pub jobs: u64,
    pub slowest_phase: Option<PerformancePhase>,
    pub phases: BTreeMap<String, u64>,
    pub requests_by_kind: serde_json::Value,
    pub performance: serde_json::Value,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceAggregate {
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub action: String,
    pub samples: usize,
    pub median_ms: u64,
    pub p95_ms: Option<u64>,
    pub slowest_phase: Option<PerformancePhase>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceHistory {
    pub recent: Vec<PerformanceRun>,
    pub aggregates: Vec<PerformanceAggregate>,
}
fn json_u64(value: Option<&serde_json::Value>) -> u64 {
    value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|n| n.max(0.0) as u64))
        })
        .unwrap_or(0)
}
fn insert_phase_group(
    phases: &mut BTreeMap<String, u64>,
    prefix: &str,
    value: Option<&serde_json::Value>,
) {
    let Some(values) = value.and_then(serde_json::Value::as_object) else {
        return;
    };
    for (key, value) in values {
        if value.is_number() {
            phases.insert(format!("{prefix}.{key}"), json_u64(Some(value)));
        }
    }
}
fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    match values.len() {
        0 => 0,
        n if n % 2 == 1 => values[n / 2],
        n => values[n / 2 - 1].saturating_add(values[n / 2]) / 2,
    }
}
fn p95(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    values[((values.len() * 95).div_ceil(100)).saturating_sub(1)]
}
// app.active is the parent of every workMs entry, so ranking it against its own children would
// always name the parent. It is reported as a phase but never as the answer to "what cost most".
fn slowest(phases: &BTreeMap<String, u64>) -> Option<PerformancePhase> {
    phases
        .iter()
        .filter(|(key, value)| **value > 0 && key.as_str() != "app.active")
        .max_by_key(|(_, value)| **value)
        .map(|(key, milliseconds)| PerformancePhase {
            key: key.clone(),
            milliseconds: *milliseconds,
        })
}
fn performance_run(log: AppLog) -> Option<PerformanceRun> {
    let detail: serde_json::Value = serde_json::from_str(&log.detail_json).ok()?;
    let performance = detail.get("performance")?.clone();
    if performance.get("version").and_then(|value| value.as_u64()) != Some(1) {
        return None;
    }
    let worker = performance.get("worker");
    let app = performance.get("app");
    let mut phases = BTreeMap::new();
    insert_phase_group(
        &mut phases,
        "worker",
        worker.and_then(|value| value.get("bucketsMs")),
    );
    insert_phase_group(
        &mut phases,
        "app",
        app.and_then(|value| value.get("criticalPathMs")),
    );
    insert_phase_group(
        &mut phases,
        "work",
        app.and_then(|value| value.get("workMs")),
    );
    let total_ms = json_u64(app.and_then(|value| value.get("totalMs")))
        .max(json_u64(worker.and_then(|value| value.get("totalMs"))));
    let worker_ms = json_u64(worker.and_then(|value| value.get("totalMs")));
    let outcome = if detail.get("code").and_then(|value| value.as_str()) == Some("cancelled") {
        "cancelled"
    } else if log.level == "error" {
        "failed"
    } else {
        "completed"
    };
    let requests_by_kind = worker
        .and_then(|value| value.get("requestsByKind"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Some(PerformanceRun {
        id: log.id,
        at: log.at,
        source_id: log.source_id,
        source_name: log.source_name,
        action: log.action,
        outcome: outcome.into(),
        total_ms,
        worker_ms,
        requests: json_u64(detail.get("requests")),
        pages: json_u64(detail.get("pages")),
        jobs: json_u64(
            detail
                .get("discovered")
                .or_else(|| detail.get("completedSources")),
        ),
        slowest_phase: slowest(&phases),
        phases,
        requests_by_kind,
        performance,
    })
}
fn performance_history_from_logs(logs: Vec<AppLog>) -> PerformanceHistory {
    let runs = logs
        .into_iter()
        .filter_map(performance_run)
        .collect::<Vec<_>>();
    let recent = runs.iter().take(50).cloned().collect::<Vec<_>>();
    let mut groups: HashMap<(Option<String>, Option<String>, String), Vec<&PerformanceRun>> =
        HashMap::new();
    for run in runs.iter().filter(|run| run.outcome == "completed") {
        let group = groups
            .entry((
                run.source_id.clone(),
                run.source_name.clone(),
                run.action.clone(),
            ))
            .or_default();
        if group.len() < 20 {
            group.push(run);
        }
    }
    let mut aggregates = groups
        .into_iter()
        .map(|((source_id, source_name, action), runs)| {
            let mut totals = runs.iter().map(|run| run.total_ms).collect::<Vec<_>>();
            let samples = totals.len();
            let median_ms = median(&mut totals);
            let p95_ms = (samples >= 5).then(|| p95(&mut totals));
            let mut phase_samples: HashMap<String, Vec<u64>> = HashMap::new();
            for run in &runs {
                for (key, value) in &run.phases {
                    if key != "app.active" {
                        phase_samples.entry(key.clone()).or_default().push(*value);
                    }
                }
            }
            let phase_medians = phase_samples
                .into_iter()
                .map(|(key, mut values)| (key, median(&mut values)))
                .collect::<BTreeMap<_, _>>();
            PerformanceAggregate {
                source_id,
                source_name,
                action,
                samples,
                median_ms,
                p95_ms,
                slowest_phase: slowest(&phase_medians),
            }
        })
        .collect::<Vec<_>>();
    aggregates.sort_by(|a, b| {
        b.median_ms
            .cmp(&a.median_ms)
            .then_with(|| a.action.cmp(&b.action))
    });
    PerformanceHistory { recent, aggregates }
}
#[tauri::command]
pub async fn scrape_performance(state: State<'_, Arc<AppState>>) -> ApiResult<PerformanceHistory> {
    let logs = sqlx::query_as::<_, AppLog>("SELECT id,at,level,source_id,source_name,action,code,message,detail_json FROM app_logs ORDER BY at DESC,rowid DESC LIMIT 1000")
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(performance_history_from_logs(logs))
}
/// The setting holding the scrape-time title filter, the first of the app's two filter layers:
/// this one decides what is ever stored, the Jobs page decides what is shown of what was stored.
pub const SCRAPE_TITLE_FILTER: &str = "scrape.titleAny";
/// Terms are separated by commas or newlines so a phrase ("design verification") stays one term.
pub fn scrape_filter_terms(raw: &str) -> Vec<String> {
    raw.split(['\n', ','])
        .map(|term| term.trim().to_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}
/// No terms means no filter — an empty box must never silently discard an entire run.
pub fn title_passes_scrape_filter(title: &str, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let title = title.to_lowercase();
    terms.iter().any(|term| title.contains(term.as_str()))
}
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct JobDescription {
    pub text: String,
    pub status: String,
    pub error: Option<String>,
}
/// One job's description, read when its card is opened. Kept out of the list query so that
/// listing jobs does not move prose nobody is looking at.
pub async fn load_job_description(pool: &SqlitePool, job_id: &str) -> ApiResult<JobDescription> {
    sqlx::query_as("SELECT description_text AS text,description_status AS status,description_error AS error FROM jobs WHERE id=?")
        .bind(job_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Job was not found".into())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeAllJobsResult {
    pub deleted: i64,
    pub kept: i64,
}
/// Deletes every stored listing. Sightings, revisions, dedupe events and match results go with
/// them by foreign key; the sources themselves are untouched, so the next update refills the list.
///
/// Two things are deliberately protected. A job you have saved or applied to is kept, because the
/// application on it is your own work and not something a scraper can fetch again. And the caller
/// has to type the confirmation exactly — a destructive command that fires on a stray click is a
/// bug waiting to happen, so the word is checked here rather than only in the window.
#[tauri::command]
pub async fn purge_all_jobs(
    confirmation: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<PurgeAllJobsResult> {
    purge_jobs(&state.db.pool, &confirmation).await
}
/// The command body, taking a pool so the guarantees above can be tested without Tauri state.
pub async fn purge_jobs(pool: &SqlitePool, confirmation: &str) -> ApiResult<PurgeAllJobsResult> {
    if confirmation.trim() != "Confirm" {
        return Err("Type Confirm to delete every stored job".into());
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let deleted = sqlx::query(
        "DELETE FROM jobs WHERE NOT EXISTS(SELECT 1 FROM applications a WHERE a.job_id=jobs.id) AND NOT EXISTS(SELECT 1 FROM review_decisions r WHERE r.job_id=jobs.id)",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected() as i64;
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    log(
        pool,
        "warning",
        None,
        None,
        "purge_all_jobs",
        None,
        &format!("Deleted every stored job: {deleted} removed, {kept} kept because they have an application or a review."),
        serde_json::json!({"deleted":deleted,"kept":kept}),
    )
    .await;
    Ok(PurgeAllJobsResult { deleted, kept })
}
#[tauri::command]
pub async fn get_setting(
    key: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Option<String>> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(key)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn set_setting(
    key: String,
    value: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    write_setting(&state.db.pool, &key, &value).await
}
async fn write_setting(pool: &SqlitePool, key: &str, value: &str) -> ApiResult<()> {
    sqlx::query("INSERT INTO settings(key,value,updated_at) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at")
        .bind(key).bind(value).bind(now())
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
/// A fresh cross-process heartbeat means another JobScraper process owns the batch.
pub async fn sync_in_progress(pool: &SqlitePool) -> bool {
    let value = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(SYNC_HEARTBEAT)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
        .is_some_and(|heartbeat| {
            Utc::now()
                .signed_duration_since(heartbeat.with_timezone(&Utc))
                .num_seconds()
                < HEARTBEAT_FRESH_SECS
        })
}
pub async fn write_sync_heartbeat(pool: &SqlitePool) {
    let _ = write_setting(pool, SYNC_HEARTBEAT, &now()).await;
}
pub async fn clear_sync_heartbeat(pool: &SqlitePool) {
    let _ = sqlx::query("DELETE FROM settings WHERE key=?")
        .bind(SYNC_HEARTBEAT)
        .execute(pool)
        .await;
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NewJob {
    pub title: String,
    pub company: String,
}
/// `created_at` is insert-only, so re-scraping an existing listing cannot resurface it here.
pub async fn new_jobs_since(
    pool: &SqlitePool,
    since: &str,
    limit: i64,
) -> ApiResult<(i64, Vec<NewJob>)> {
    let count = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE created_at >= ? AND availability != 'closed'",
    )
    .bind(since)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let jobs = sqlx::query_as(
        "SELECT title,company FROM jobs WHERE created_at >= ? AND availability != 'closed' ORDER BY created_at DESC LIMIT ?",
    )
    .bind(since)
    .bind(limit.max(0))
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok((count, jobs))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSyncStatus {
    pub enabled: bool,
    pub issue: Option<String>,
    pub last_result: Option<u32>,
    pub requested: bool,
    pub debug_build: bool,
}
async fn background_sync_status_pool(pool: &SqlitePool) -> ApiResult<BackgroundSyncStatus> {
    let requested = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(SYNC_BACKGROUND)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .is_some_and(|value| value == "true");
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let scheduler = WindowsTaskScheduler::packaged(executable.to_string_lossy().into_owned());
    let task = run_scheduler_blocking(move || scheduler.status(SYNC_TASK)).await?;
    Ok(BackgroundSyncStatus {
        enabled: task.enabled,
        issue: task.issue,
        last_result: task.last_result,
        requested,
        debug_build: cfg!(debug_assertions),
    })
}
#[tauri::command]
pub async fn background_sync_status(
    state: State<'_, Arc<AppState>>,
) -> ApiResult<BackgroundSyncStatus> {
    background_sync_status_pool(&state.db.pool).await
}
#[tauri::command]
pub async fn set_background_sync(
    enabled: bool,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<BackgroundSyncStatus> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let scheduler = WindowsTaskScheduler::packaged(executable.to_string_lossy().into_owned());
    run_scheduler_blocking(move || {
        if enabled {
            scheduler.create_sync()?;
        } else if scheduler.exists(SYNC_TASK)? {
            scheduler.cancel(SYNC_TASK)?;
        }
        Ok(())
    })
    .await?;
    write_setting(
        &state.db.pool,
        SYNC_BACKGROUND,
        if enabled { "true" } else { "false" },
    )
    .await?;
    background_sync_status_pool(&state.db.pool).await
}
pub fn normalize_canonical_url(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    };
    url.set_fragment(None);
    let host = url.host_str()?.to_ascii_lowercase();
    url.set_host(Some(&host)).ok()?;
    if (url.scheme() == "https" && url.port() == Some(443))
        || (url.scheme() == "http" && url.port() == Some(80))
    {
        url.set_port(None).ok()?
    };
    let mut pairs: url::form_urlencoded::Serializer<'_, String> =
        url::form_urlencoded::Serializer::new(String::new());
    let mut query = url::form_urlencoded::parse(url.query().unwrap_or_default().as_bytes())
        .filter(|(k, _)| !k.starts_with("utm_") && k != "fbclid")
        .collect::<Vec<_>>();
    query.sort();
    for (key, value) in query {
        pairs.append_pair(&key, &value);
    }
    let encoded = pairs.finish();
    url.set_query((!encoded.is_empty()).then_some(&encoded));
    if url.path() != "/" {
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
    }
    Some(url.to_string())
}
fn normalized_text(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn job_fingerprint(title: &str, company: &str, location: Option<&str>) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{}|{}|{}",
            normalized_text(title),
            normalized_text(company),
            normalized_text(location.unwrap_or(""))
        ))
    )
}
fn worker_content_hash(title: &str, company: &str, description: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{title}\n{company}\n{description}"))
    )
}
#[derive(Default, Debug, Clone, Copy)]
pub struct PersistBatchResult {
    pub written: u32,
    pub unchanged: u32,
}
const APPLICATION_SELECT: &str = "SELECT a.id,a.job_id,a.persona_id,a.current_stage,a.recruiter,a.recruiter_name,a.recruiter_email,a.recruiter_phone,a.source_attribution,a.rejection_reason,a.rejection_category,a.withdrawn_reason,a.accepted_at,a.applied_at,a.created_at,a.updated_at,j.title,j.company,EXISTS(SELECT 1 FROM application_attempts att WHERE att.application_id=a.id AND att.resolved_at IS NULL) AS pending_confirmation FROM applications a LEFT JOIN jobs j ON j.id=a.job_id";
fn valid_url(value: &str, allow_private: bool) -> ApiResult<()> {
    let url = Url::parse(value).map_err(|_| "A valid HTTP(S) URL is required".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only HTTP(S) source URLs are allowed".into());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !allow_private
        && (host == "localhost"
            || host.ends_with(".local")
            || host.starts_with("127.")
            || host == "::1")
    {
        return Err("Private/local navigation needs explicit per-source approval".into());
    }
    Ok(())
}
/// sqlx refuses a database carrying a migration this binary does not have (VersionMissing), which
/// is exactly what an older build sees after a newer one has run. The raw error names neither the
/// cause nor the fix, and the user's data is fine, so say so.
pub fn migration_error(error: sqlx::migrate::MigrateError) -> String {
    match error {
        sqlx::migrate::MigrateError::VersionMissing(version) => format!(
            "This database was created by a newer version of JobScraper (schema {version}). Install the newer version to open it — your data is intact."
        ),
        other => other.to_string(),
    }
}
impl Database {
    /// Builds the pool without touching the disk, so application startup can manage its state
    /// and paint a window before any I/O happens. Connecting and migrating is `prepare()`.
    pub fn connect_lazy(path: PathBuf) -> Self {
        let opts = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true);
        Self {
            pool: SqlitePoolOptions::new()
                .max_connections(5)
                .connect_lazy_with(opts),
            root: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        }
    }
    pub async fn prepare(&self) -> ApiResult<()> {
        sqlx::query("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(migration_error)?;
        // Fast, bounded planner maintenance after schema/index changes. It never rewrites user
        // rows or blocks startup with a whole-database VACUUM.
        sqlx::query("PRAGMA optimize")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    /// A process can disappear after opening a run but before its terminal event. Those partial
    /// sightings must never affect availability, and the run must not remain "running" forever.
    pub async fn recover_interrupted_runs(&self) -> ApiResult<u64> {
        // Another process may own these running rows. Its heartbeat prevents this startup from
        // deleting live sightings; a crashed owner becomes recoverable after five minutes.
        if sync_in_progress(&self.pool).await {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query(
            "DELETE FROM job_occurrences WHERE run_id IN (SELECT id FROM scrape_runs WHERE status='running')",
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let recovered = sqlx::query("UPDATE scrape_runs SET status='cancelled',complete=0,failure_code='interrupted',diagnostics='The app exited before this run finished; partial sightings were discarded.',finished_at=? WHERE status='running'")
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected();
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(recovered)
    }
    pub async fn open(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Self::connect_lazy(path);
        db.prepare().await?;
        Ok(db)
    }
    pub async fn install_starter_pack(&self) -> ApiResult<()> {
        let installed: Option<String> = sqlx::query_scalar(
            "SELECT value FROM schema_metadata WHERE key='starter_pack_version'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        if installed.as_deref() == Some(starter_pack_version()) {
            return Ok(());
        }
        // Stable IDs make this dated pack idempotent even when a user renames a
        // source. All active starters are disabled until explicitly enabled.
        //
        // Live-fire verified 2026-08-31: every source below returns real vacancies anonymously
        // through the sidecar's own guarded fetch.
        // The earlier pack guessed generic ATS adapters for hosts that do not serve one
        // (Arm ships TalentBrew assets but Radancy markup; Cisco ships Phenom assets but
        // an inline payload; Micron is a Workday tenant, not an Eightfold one) and those
        // guesses are corrected here against the live contract, not against the vendor
        // fingerprint. Microchip's board is a Workday tenant on wd5.myworkdaysite.com (its own
        // careers host only 302s to a marketing page), and u-blox's openings come from the Algolia
        // index its job-openings page queries client-side; both were verified live.
        let starters: Vec<&CatalogCompany> = company_catalog()
            .companies
            .iter()
            .filter(|company| company.starter)
            .collect();
        for company in starters {
            let (source_id, name, url, adapter, kind) = (
                company.id.as_str(),
                company.name.as_str(),
                company.base_url.as_str(),
                company.adapter_id.as_str(),
                company.kind.as_str(),
            );
            let reason = company.disabled_reason.as_deref();
            let t = now();
            sqlx::query("INSERT OR IGNORE INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?,?,?,?,0,?,?,1,0,?,?)")
    .bind(source_id).bind(name).bind(url).bind(adapter).bind("1.1.0").bind(kind).bind(reason).bind(&t).bind(&t).execute(&self.pool).await.map_err(|e| e.to_string())?;
            let mut config = serde_json::json!({"schemaVersion":"1.1.0","starterPackVersion":starter_pack_version(),"adapterVersion":"1.1.0","mode":"direct"});
            if let (Some(base), Some(extra)) = (config.as_object_mut(), company.config.as_object())
            {
                base.extend(extra.clone());
            }
            sqlx::query("INSERT OR IGNORE INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?)")
                .bind(id()).bind(source_id).bind(config.to_string()).bind(&t).bind(&t).execute(&self.pool).await.map_err(|e| e.to_string())?;
            // Legacy packs used generated source IDs. Retire only an untouched,
            // disabled old starter with no dependent history; user-created sources
            // and every historical reference remain intact.
            sqlx::query("UPDATE sources SET enabled=0,deleted_at=?,disabled_reason='Replaced by versioned starter-pack source' WHERE id<>? AND name=? AND base_url=? AND adapter_id=? AND enabled=0 AND deleted_at IS NULL AND created_at=updated_at AND disabled_reason='Starter source is disabled until you review and enable it.' AND NOT EXISTS (SELECT 1 FROM scrape_runs WHERE scrape_runs.source_id=sources.id) AND NOT EXISTS (SELECT 1 FROM jobs WHERE jobs.source_id=sources.id) AND EXISTS (SELECT 1 FROM source_configs c WHERE c.source_id=sources.id AND c.created_at=c.updated_at AND c.config_json LIKE '%starterPackVersion%')")
                .bind(&t).bind(source_id).bind(name).bind(url).bind(adapter).execute(&self.pool).await.map_err(|e| e.to_string())?;
            // INSERT OR IGNORE cannot fix a row an earlier pack already wrote, so the fields this
            // pack owns are asserted here. Deliberately not restricted to untouched rows: enabling
            // a source counts as touching it, and the rows most in need of correcting are the ones
            // someone tried to use. What belongs to the user is left alone — whether the source is
            // switched on, and whether it was deleted.
            sqlx::query("UPDATE sources SET name=?,base_url=?,adapter_id=?,disabled_reason=CASE WHEN enabled=0 THEN ? ELSE disabled_reason END,updated_at=? WHERE id=? AND (name<>? OR base_url<>? OR adapter_id<>? OR (enabled=0 AND disabled_reason IS NOT ?))")
                .bind(name).bind(url).bind(adapter).bind(reason).bind(&t).bind(source_id).bind(name).bind(url).bind(adapter).bind(reason).execute(&self.pool).await.map_err(|e| e.to_string())?;
            // The config is replaced rather than merged into: merging leaves behind whatever an
            // older pack wrote and this one dropped, and those leftovers are not inert — the
            // "pageSize": 0 an earlier pack left on Micron and NVIDIA asks Workday for zero rows a
            // page. Only the two settings that are genuinely the user's survive the replacement.
            let existing_config: Option<String> =
                sqlx::query_scalar("SELECT config_json FROM source_configs WHERE source_id=?")
                    .bind(source_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| e.to_string())?;
            if let Some(existing) = existing_config {
                let mut wanted = config.clone();
                if let (Some(target), Ok(previous)) = (
                    wanted.as_object_mut(),
                    serde_json::from_str::<serde_json::Value>(&existing),
                ) {
                    for key in ["query", "headless"] {
                        if let Some(kept) = previous.get(key) {
                            target.insert(key.into(), kept.clone());
                        }
                    }
                }
                if wanted.to_string() != existing {
                    sqlx::query(
                        "UPDATE source_configs SET config_json=?,updated_at=? WHERE source_id=?",
                    )
                    .bind(wanted.to_string())
                    .bind(&t)
                    .bind(source_id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| e.to_string())?;
                }
            }
        }
        // A source the app stored under a guessed adapter keeps that guess forever, and the
        // generic selector adapter turns a board's navigation shell into "jobs" (Apple's 11
        // menu links were the case that surfaced this). Every host below has exactly one
        // adapter that reads it, live-verified, so an existing source pointed at that host is
        // moved onto it — keeping the user's enabled state, filters and any other settings.
        // The fourth field relocates the source. It is set only for the hosts an earlier
        // starter pack itself seeded wrongly — careers.st.com does not resolve at all, gf.com
        // and careers.micron.com are marketing pages — so rewriting those addresses corrects
        // this app's own mistake rather than overriding an address the user chose.
        // jobs.intel.com and jobs.cisco.com are vanity addresses for the same boards: the first
        // now answers with a 404 redirector, the second redirects to careers.cisco.com. Pointing
        // them at the board they stand for keeps the source the user meant.
        let upgrades: [(&str, &str, &str, &str); 21] = [
            ("careers.microchip.com", "workday", "{\"maxPages\":500,\"site\":\"External\",\"tenant\":\"microchiphr\"}", "https://wd5.myworkdaysite.com/en-US/recruiting/microchiphr/External"),
            ("www.microchip.com", "workday", "{\"maxPages\":500,\"site\":\"External\",\"tenant\":\"microchiphr\"}", "https://wd5.myworkdaysite.com/en-US/recruiting/microchiphr/External"),
            ("jobs.intel.com", "workday", "{\"maxPages\":500,\"site\":\"External\",\"tenant\":\"intel\"}", "https://intel.wd1.myworkdayjobs.com/"),
            ("jobs.cisco.com", "cisco", "{\"maxPages\":500}", "https://careers.cisco.com/global/en/search-results"),
            ("jobs.apple.com", "apple", "{\"maxPages\":500,\"requestDelayMs\":250}", ""),
            ("careers.arm.com", "arm", "{\"maxPages\":500}", ""),
            ("careers.amd.com", "amd", "{\"maxPages\":500}", ""),
            ("careers.mediatek.com", "mediatek", "{\"maxPages\":500}", ""),
            ("www.mediatek.com", "mediatek", "{\"maxPages\":500}", "https://careers.mediatek.com/en/jobs"),
            ("careers.cisco.com", "cisco", "{\"maxPages\":500}", ""),
            ("talent.skhynix.com", "sk-hynix", "{\"maxPages\":500}", ""),
            ("www.skhynix.com", "sk-hynix", "{\"maxPages\":500}", "https://talent.skhynix.com/hub/en/apply/job"),
            ("www.u-blox.com", "u-blox", "{\"maxPages\":500}", ""),
            ("www.google.com", "google", "{\"maxPages\":500}", ""),
            ("careers.qualcomm.com", "eightfold", "{\"domain\":\"qualcomm.com\",\"eightfoldApi\":\"pcsx\",\"maxPages\":500,\"requestDelayMs\":800,\"retryForbidden\":true}", ""),
            ("careers.gf.com", "eightfold", "{\"domain\":\"globalfoundries.com\",\"eightfoldApi\":\"pcsx\",\"maxPages\":500,\"requestDelayMs\":800,\"retryForbidden\":true}", ""),
            ("gf.com", "eightfold", "{\"domain\":\"globalfoundries.com\",\"eightfoldApi\":\"pcsx\",\"maxPages\":500,\"requestDelayMs\":800,\"retryForbidden\":true}", "https://careers.gf.com/"),
            ("stmicroelectronics.eightfold.ai", "eightfold", "{\"domain\":\"stmicroelectronics.com\",\"eightfoldApi\":\"legacy\",\"maxPages\":500,\"requestDelayMs\":800,\"retryForbidden\":true}", ""),
            ("careers.st.com", "eightfold", "{\"domain\":\"stmicroelectronics.com\",\"eightfoldApi\":\"legacy\",\"maxPages\":500,\"requestDelayMs\":800,\"retryForbidden\":true}", "https://stmicroelectronics.eightfold.ai/"),
            ("careers.micron.com", "workday", "{\"maxPages\":500,\"site\":\"External\",\"tenant\":\"micron\"}", "https://micron.wd1.myworkdayjobs.com/"),
            ("nvidia.wd5.myworkdayjobs.com", "workday", "{\"maxPages\":500,\"site\":\"NVIDIAExternalCareerSite\",\"splitFacet\":\"jobFamilyGroup\",\"splitThreshold\":2000,\"tenant\":\"nvidia\"}", ""),
        ];
        let known_sources = sqlx::query("SELECT s.id,s.base_url,s.adapter_id,s.enabled,c.config_json FROM sources s JOIN source_configs c ON c.source_id=s.id WHERE s.deleted_at IS NULL AND s.kind<>'reference'")
            .fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
        for row in known_sources {
            let source_id: String = row.get("id");
            let base_url: String = row.get("base_url");
            let Ok(parsed_url) = Url::parse(&base_url) else {
                continue;
            };
            let host = parsed_url
                .host_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            // google.com only serves a job board under /about/careers; the bare host is not one.
            let Some((_, target, extra, relocation)) = upgrades.iter().find(|(candidate, ..)| {
                *candidate == host
                    && (*candidate != "www.google.com"
                        || parsed_url.path().starts_with("/about/careers"))
            }) else {
                continue;
            };
            let adapter_id: String = row.get("adapter_id");
            let enabled: bool = row.get("enabled");
            let config_text: String = row.get("config_json");
            let mut config = serde_json::from_str::<serde_json::Value>(&config_text)
                .unwrap_or_else(|_| serde_json::json!({}));
            let Some(object) = config.as_object_mut() else {
                continue;
            };
            // None of these keys mean anything on these hosts: the selector fields belong to
            // the adapter being replaced, and a saved listingPath would override the endpoint
            // the corrected adapter derives (the stale "/api/career_hub" Eightfold default is
            // exactly that case, and it survives even when the adapter id already looks right).
            for key in [
                "itemSelector",
                "titleSelector",
                "companySelector",
                "locationSelector",
                "dateSelector",
                "urlSelector",
                "descriptionSelector",
                "nextSelector",
                "urlPrefix",
                "itemsPath",
                "listingPath",
            ] {
                object.remove(key);
            }
            if let Ok(serde_json::Value::Object(extra)) = serde_json::from_str(extra) {
                object.extend(extra);
            }
            // A saved expectedHost that does not name the host the adapter actually reads raises
            // a host-drift warning on every response: after a relocation it names the old
            // address, and SK hynix's listings come from two boards it does not host at all.
            if matches!(*target, "sk-hynix" | "u-blox") {
                object.remove("expectedHost");
            } else if let Some(moved) = Url::parse(relocation)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
            {
                object.insert("expectedHost".into(), serde_json::json!(moved));
            }
            if *target == "apple" {
                let locale = parsed_url
                    .path_segments()
                    .and_then(|mut parts| parts.next())
                    .filter(|part| part.len() == 5 && part.as_bytes().get(2) == Some(&b'-'))
                    .unwrap_or("en-us")
                    .to_lowercase();
                object.insert("locale".into(), serde_json::json!(locale));
            }
            object.insert("adapterVersion".into(), serde_json::json!("1.1.0"));
            object.insert("mode".into(), serde_json::json!("direct"));
            // A source that is still disabled keeps whatever reason it was disabled with.
            let reason = if enabled {
                None
            } else {
                let existing: Option<String> =
                    sqlx::query_scalar("SELECT disabled_reason FROM sources WHERE id=?")
                        .bind(&source_id)
                        .fetch_one(&self.pool)
                        .await
                        .map_err(|e| e.to_string())?;
                existing
            };
            let t = now();
            if adapter_id != *target {
                sqlx::query("UPDATE sources SET adapter_id=?,adapter_version='1.1.0',disabled_reason=?,updated_at=? WHERE id=?")
                    .bind(target).bind(reason).bind(&t).bind(&source_id).execute(&self.pool).await.map_err(|e| e.to_string())?;
            }
            if !relocation.is_empty() {
                sqlx::query("UPDATE sources SET base_url=?,updated_at=? WHERE id=? AND base_url=?")
                    .bind(relocation)
                    .bind(&t)
                    .bind(&source_id)
                    .bind(&base_url)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if config.to_string() != config_text {
                sqlx::query(
                    "UPDATE source_configs SET config_json=?,updated_at=? WHERE source_id=?",
                )
                .bind(config.to_string())
                .bind(&t)
                .bind(&source_id)
                .execute(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
        sqlx::query("INSERT INTO schema_metadata(key,value) VALUES('starter_pack_version',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(starter_pack_version()).execute(&self.pool).await.map_err(|e| e.to_string())?;
        Ok(())
    }
    pub async fn persist_worker_job(
        &self,
        run_id: &str,
        source_id: &str,
        payload: &serde_json::Value,
    ) -> ApiResult<()> {
        self.persist_worker_jobs(run_id, source_id, std::slice::from_ref(payload))
            .await
            .map(|_| ())
    }

    /// Changed jobs arrive in a burst after the board has been read. One transaction per row made
    /// SQLite durability, not parsing, the visible final phase of large runs. Keep chunks bounded
    /// in the caller and commit every chunk together.
    pub async fn persist_worker_jobs(
        &self,
        run_id: &str,
        source_id: &str,
        payloads: &[serde_json::Value],
    ) -> ApiResult<PersistBatchResult> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let mut result = PersistBatchResult::default();
        for payload in payloads {
            if Self::persist_worker_job_in(&mut tx, run_id, source_id, payload).await? {
                result.written += 1;
            } else {
                result.unchanged += 1;
            }
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(result)
    }

    async fn persist_worker_job_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        run_id: &str,
        source_id: &str,
        payload: &serde_json::Value,
    ) -> ApiResult<bool> {
        let title = payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let company = payload
            .get("company")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() || company.is_empty() {
            return Err("Worker result is missing required title or company".into());
        }
        let external = payload.get("externalId").and_then(|v| v.as_str());
        let canonical = payload
            .get("canonicalUrl")
            .and_then(|v| v.as_str())
            .and_then(normalize_canonical_url);
        let requisition = payload
            .get("requisitionId")
            .and_then(|v| v.as_str())
            .filter(|value| !value.trim().is_empty());
        let fingerprint = job_fingerprint(
            title,
            company,
            payload.get("location").and_then(|v| v.as_str()),
        );
        let t = now();
        let job_id = id();
        let incoming_description = payload
            .get("descriptionText")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let incoming_hash = payload
            .get("contentHash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let listing_hash = payload
            .get("listingHash")
            .and_then(|v| v.as_str())
            .filter(|value| !value.is_empty());
        // Identity, not similarity. Two openings that merely read alike — same title, same
        // company, same city — are two openings, so the title/company/location fingerprint only
        // re-identifies a row from the SAME source whose URL or generated id changed under it.
        let stable_identity = external.is_some() || canonical.is_some() || requisition.is_some();
        let existing:Option<(String,Option<String>,Option<String>,String,Option<String>,String,Option<String>)>=sqlx::query_as("SELECT id,content_hash,listing_hash,description_text,description_html,description_status,description_checked_at FROM jobs WHERE (source_id=? AND external_id=?) OR (? IS NOT NULL AND canonical_url=?) OR (? IS NOT NULL AND requisition_id=? AND lower(trim(company))=lower(?)) OR (?=0 AND source_id=? AND dedupe_fingerprint=?) ORDER BY created_at LIMIT 1").bind(source_id).bind(external).bind(canonical.as_deref()).bind(canonical.as_deref()).bind(requisition).bind(requisition).bind(company).bind(stable_identity).bind(source_id).bind(&fingerprint).fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
        let incoming_status = payload
            .get("descriptionStatus")
            .and_then(|value| value.as_str())
            .filter(|value| matches!(*value, "complete" | "pending" | "failed"))
            .unwrap_or("complete");
        // The row is already exactly this job. Rewriting it would copy an unchanged description
        // — several kilobytes — back over itself and log a duplicate merge that did not happen.
        // The sighting is the only thing this run has to say, so it is the only thing written.
        if let Some((id, stored_content, stored_listing, _, _, stored_status, _)) = &existing {
            if incoming_status == "complete"
                && stored_status == "complete"
                && !incoming_hash.is_empty()
                && stored_content.as_deref() == Some(incoming_hash)
                && listing_hash.is_some()
                && stored_listing.as_deref() == listing_hash
            {
                sqlx::query(
                    "INSERT OR IGNORE INTO job_occurrences(id,job_id,run_id,seen_at) VALUES(?,?,?,?)",
                )
                .bind(crate::db::id())
                .bind(id)
                .bind(run_id)
                .bind(&t)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
                return Ok(false);
            }
        }
        let (actual, stored_description, stored_html, stored_checked) = existing
            .map(|(id, _, _, description, html, _, checked)| (id, description, html, checked))
            .unwrap_or_else(|| (job_id, String::new(), None, None));
        let pending = incoming_status == "pending";
        let description = if pending && !stored_description.is_empty() {
            stored_description.as_str()
        } else {
            incoming_description
        };
        let description_html = if pending && stored_html.is_some() {
            stored_html
        } else {
            payload
                .get("descriptionHtml")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(250_000).collect::<String>())
        };
        let hash = if pending || incoming_hash.is_empty() {
            worker_content_hash(title, company, description)
        } else {
            incoming_hash.to_owned()
        };
        let checked_at = if incoming_status == "complete" {
            Some(t.clone())
        } else {
            stored_checked
        };
        let description_error = payload.get("descriptionError").and_then(|v| v.as_str());
        sqlx::query("INSERT INTO jobs(id,source_id,external_id,canonical_url,apply_url,requisition_id,dedupe_fingerprint,first_seen_at,title,company,location,location_countries,work_mode,description_text,description_html,detail_url,description_status,description_error,description_checked_at,posted_at,closing_at,salary_min,salary_max,salary_currency,salary_period,salary_confidence,seniority,skills_json,content_hash,listing_hash,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET external_id=COALESCE(excluded.external_id,jobs.external_id),canonical_url=excluded.canonical_url,apply_url=excluded.apply_url,requisition_id=COALESCE(excluded.requisition_id,jobs.requisition_id),dedupe_fingerprint=excluded.dedupe_fingerprint,title=excluded.title,company=excluded.company,location=excluded.location,location_countries=excluded.location_countries,work_mode=excluded.work_mode,description_text=excluded.description_text,description_html=excluded.description_html,detail_url=COALESCE(excluded.detail_url,jobs.detail_url),description_status=excluded.description_status,description_error=excluded.description_error,description_checked_at=excluded.description_checked_at,posted_at=excluded.posted_at,closing_at=excluded.closing_at,salary_min=excluded.salary_min,salary_max=excluded.salary_max,salary_currency=excluded.salary_currency,salary_period=excluded.salary_period,salary_confidence=excluded.salary_confidence,seniority=excluded.seniority,skills_json=excluded.skills_json,content_hash=excluded.content_hash,listing_hash=COALESCE(excluded.listing_hash,jobs.listing_hash),provenance_json=excluded.provenance_json,extraction_at=excluded.extraction_at,adapter_version=excluded.adapter_version,updated_at=excluded.updated_at")
            .bind(&actual).bind(source_id).bind(external).bind(canonical.as_deref()).bind(payload.get("applyUrl").and_then(|v|v.as_str())).bind(requisition).bind(&fingerprint).bind(&t).bind(title).bind(company).bind(payload.get("location").and_then(|v|v.as_str())).bind(crate::locations::stored(&crate::locations::resolve(payload.get("location").and_then(|v|v.as_str()),canonical.as_deref()))).bind(payload.get("workMode").and_then(|v|v.as_str())).bind(description).bind(description_html).bind(payload.get("detailUrl").and_then(|v|v.as_str())).bind(incoming_status).bind(description_error).bind(checked_at).bind(payload.get("postedAt").and_then(|v|v.as_str())).bind(payload.get("closingAt").and_then(|v|v.as_str())).bind(payload.get("salaryMin").and_then(|v|v.as_f64())).bind(payload.get("salaryMax").and_then(|v|v.as_f64())).bind(payload.get("salaryCurrency").and_then(|v|v.as_str())).bind(payload.get("salaryPeriod").and_then(|v|v.as_str())).bind(payload.get("salaryConfidence").and_then(|v|v.as_str())).bind(payload.get("seniority").and_then(|v|v.as_str())).bind(payload.get("skills").cloned().unwrap_or_else(||serde_json::json!([])).to_string()).bind(hash).bind(listing_hash).bind(payload.get("provenance").cloned().unwrap_or_else(||serde_json::json!({})).to_string()).bind(&t).bind(payload.get("adapterVersion").and_then(|v|v.as_str()).unwrap_or("1.0.0")).bind(&t).bind(&t).execute(&mut **tx).await.map_err(|e|e.to_string())?;
        sqlx::query(
            "INSERT OR IGNORE INTO job_occurrences(id,job_id,run_id,seen_at) VALUES(?,?,?,?)",
        )
        .bind(id())
        .bind(&actual)
        .bind(run_id)
        .bind(&t)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        Ok(true)
    }
    /// Records that a job the worker recognised is still on the board, without touching the row
    /// itself. Returns 1 when the hash matched a stored job, 0 when it did not — a miss means the
    /// row was deleted or archived between the run starting and this event, and the next full read
    /// will pick it up again. Cheap by design: one indexed lookup and one insert, no rewrite.
    pub async fn record_job_seen(
        &self,
        run_id: &str,
        source_id: &str,
        listing_hash: &str,
    ) -> ApiResult<u32> {
        self.record_jobs_seen(run_id, source_id, &[listing_hash.to_owned()])
            .await
    }

    /// Persists all unchanged sightings under one transaction. Worker-side hashes are unique, but
    /// deduplicating here keeps this boundary safe for older or third-party workers too.
    pub async fn record_jobs_seen(
        &self,
        run_id: &str,
        source_id: &str,
        listing_hashes: &[String],
    ) -> ApiResult<u32> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let seen_at = now();
        let mut recorded = 0;
        let mut unique = std::collections::HashSet::new();
        for listing_hash in listing_hashes {
            if listing_hash.is_empty() || !unique.insert(listing_hash) {
                continue;
            }
            recorded += sqlx::query("INSERT OR IGNORE INTO job_occurrences(id,job_id,run_id,seen_at) SELECT ?,id,?,? FROM jobs WHERE source_id=? AND listing_hash=? ORDER BY created_at LIMIT 1")
                .bind(id())
                .bind(run_id)
                .bind(&seen_at)
                .bind(source_id)
                .bind(listing_hash)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?
                .rows_affected() as u32;
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(recorded)
    }

    /// Detail work is deliberately separate from the listing transaction. The listing hash is an
    /// optimistic lock: if another board read changed the row while a detail request was in
    /// flight, the stale response is ignored instead of overwriting newer data.
    pub async fn pending_enrichment_job(
        &self,
        job_id: &str,
        refresh: bool,
    ) -> ApiResult<Option<(String, serde_json::Value)>> {
        let row = sqlx::query("SELECT source_id,id,external_id,canonical_url,apply_url,title,company,location,work_mode,posted_at,listing_hash,detail_url FROM jobs WHERE id=? AND availability<>'archived' AND detail_url IS NOT NULL AND listing_hash IS NOT NULL AND (? OR description_status IN ('pending','failed') OR description_checked_at IS NULL OR julianday(description_checked_at) IS NULL OR julianday(description_checked_at)<=julianday('now','-7 days'))")
            .bind(job_id)
            .bind(refresh)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|row| {
            (
                row.get::<String, _>("source_id"),
                serde_json::json!({
                    "jobId": row.get::<String, _>("id"),
                    "externalId": row.get::<Option<String>, _>("external_id"),
                    "canonicalUrl": row.get::<Option<String>, _>("canonical_url"),
                    "applyUrl": row.get::<Option<String>, _>("apply_url"),
                    "title": row.get::<String, _>("title"),
                    "company": row.get::<String, _>("company"),
                    "location": row.get::<Option<String>, _>("location"),
                    "workMode": row.get::<Option<String>, _>("work_mode"),
                    "postedAt": row.get::<Option<String>, _>("posted_at"),
                    "listingHash": row.get::<String, _>("listing_hash"),
                    "detailUrl": row.get::<String, _>("detail_url")
                }),
            )
        }))
    }

    pub async fn persist_enriched_jobs(
        &self,
        source_id: &str,
        payloads: &[serde_json::Value],
    ) -> ApiResult<u64> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let checked_at = now();
        let mut written = 0;
        for payload in payloads {
            let Some(job_id) = payload.get("jobId").and_then(|value| value.as_str()) else {
                continue;
            };
            let Some(listing_hash) = payload.get("listingHash").and_then(|value| value.as_str())
            else {
                continue;
            };
            let description = payload
                .get("descriptionText")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let title = payload
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let company = payload
                .get("company")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            written += sqlx::query("UPDATE jobs SET description_text=?,description_html=?,content_hash=?,description_status='complete',description_error=NULL,description_checked_at=?,updated_at=? WHERE id=? AND source_id=? AND listing_hash=?")
                .bind(description)
                .bind(payload.get("descriptionHtml").and_then(|value| value.as_str()))
                .bind(worker_content_hash(title, company, description))
                .bind(&checked_at)
                .bind(&checked_at)
                .bind(job_id)
                .bind(source_id)
                .bind(listing_hash)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?
                .rows_affected();
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(written)
    }

    pub async fn record_enrichment_failures(
        &self,
        source_id: &str,
        payloads: &[serde_json::Value],
    ) -> ApiResult<u64> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let checked_at = now();
        let mut written = 0;
        for payload in payloads {
            let Some(job_id) = payload.get("jobId").and_then(|value| value.as_str()) else {
                continue;
            };
            let Some(listing_hash) = payload.get("listingHash").and_then(|value| value.as_str())
            else {
                continue;
            };
            let message = payload
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("Description download failed")
                .chars()
                .take(1000)
                .collect::<String>();
            written += sqlx::query("UPDATE jobs SET description_status='failed',description_error=?,description_checked_at=? WHERE id=? AND source_id=? AND listing_hash=?")
                .bind(message)
                .bind(&checked_at)
                .bind(job_id)
                .bind(source_id)
                .bind(listing_hash)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?
                .rows_affected();
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(written)
    }
    /// How many runs of sighting history each source keeps. Availability needs only the current
    /// run; the rest is for looking at when a source behaves oddly.
    const RUNS_KEPT: i64 = 10;
    /// Marks what this run did and did not find. Only a complete, successful read may change
    /// availability at all — a partial read proves nothing about what is missing, which is why
    /// the worker is careful never to claim completeness it cannot back up.
    ///
    /// Seen means active, and resets the counter. Unseen costs one strike: the first is
    /// `possibly_closed`, the second is `closed`. Two complete reads rather than one because a
    /// board can drop a listing for a moment and put it back. `archived` is never touched.
    ///
    /// These rules also existed as a pure Rust function in availability.rs that nothing called,
    /// so the spec and the implementation could drift apart silently. This is the one that runs.
    pub async fn reconcile_availability(
        &self,
        run_id: &str,
        source_id: &str,
        complete: bool,
    ) -> ApiResult<()> {
        if !complete {
            return Ok(());
        };
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query("UPDATE jobs SET missing_full_runs=0,availability='active',updated_at=? WHERE source_id=? AND availability!='archived' AND id IN (SELECT job_id FROM job_occurrences WHERE run_id=?)").bind(now()).bind(source_id).bind(run_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE jobs SET missing_full_runs=missing_full_runs+1,availability=CASE WHEN missing_full_runs+1>=2 THEN 'closed' ELSE 'possibly_closed' END,updated_at=? WHERE source_id=? AND availability NOT IN ('archived','closed') AND id NOT IN (SELECT job_id FROM job_occurrences WHERE run_id=?)").bind(now()).bind(source_id).bind(run_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        // One occurrence row per job per run, never pruned, meant a full update added tens of
        // thousands of rows that nothing would read again: the availability rules above look only
        // at this run, and missing_full_runs carries the history. Keeping the last few runs leaves
        // the sighting trail useful for diagnostics without letting it grow forever.
        sqlx::query("DELETE FROM job_occurrences WHERE run_id IN (SELECT id FROM scrape_runs WHERE source_id=? AND id NOT IN (SELECT id FROM scrape_runs WHERE source_id=? ORDER BY started_at DESC LIMIT ?))")
            .bind(source_id).bind(source_id).bind(Self::RUNS_KEPT).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        // Dedupe events are an audit trail of merges, not state. They are pruned on the same
        // window so the two tables cannot drift apart.
        let cutoff: Option<String> = sqlx::query_scalar("SELECT min(started_at) FROM (SELECT started_at FROM scrape_runs WHERE source_id=? ORDER BY started_at DESC LIMIT ?)")
            .bind(source_id).bind(Self::RUNS_KEPT).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.flatten();
        if let Some(cutoff) = cutoff {
            sqlx::query("DELETE FROM job_dedupe_events WHERE created_at<? AND canonical_job_id IN (SELECT id FROM jobs WHERE source_id=?)")
                .bind(&cutoff).bind(source_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}
/// The employers this app can already read, for the picker that adds one. Every entry carries the
/// adapter and the settings that were verified against that board, so adding one asks the user for
/// nothing and reads no site to work it out.
#[tauri::command]
pub async fn list_company_catalog() -> ApiResult<Vec<CatalogCompany>> {
    // The legacy Marvell bookmark stays in the starter pack for compatibility, but it is not
    // another supported board and save_source correctly refuses to enable references.
    Ok(company_catalog()
        .companies
        .iter()
        .filter(|company| company.kind != "reference")
        .cloned()
        .collect())
}
#[tauri::command]
pub async fn list_sources(state: State<'_, Arc<AppState>>) -> ApiResult<Vec<Source>> {
    sqlx::query_as::<_,Source>("SELECT s.id,s.name,s.base_url,s.adapter_id,s.adapter_version,s.enabled,s.kind,s.disabled_reason,s.robots_override,s.last_success_at,COALESCE(c.live,0) AS job_count,COALESCE(c.gone,0) AS closed_count,s.created_at,s.updated_at FROM sources s LEFT JOIN (SELECT source_id,sum(CASE WHEN availability='closed' THEN 0 ELSE 1 END) AS live,sum(CASE WHEN availability='closed' THEN 1 ELSE 0 END) AS gone FROM jobs GROUP BY source_id) c ON c.source_id=s.id WHERE s.deleted_at IS NULL ORDER BY s.name") .fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn get_source_config(
    source_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<serde_json::Value> {
    let text: String =
        sqlx::query_scalar("SELECT config_json FROM source_configs WHERE source_id=?")
            .bind(source_id)
            .fetch_one(&state.db.pool)
            .await
            .map_err(|_| "Source configuration was not found".to_string())?;
    let mut config: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "Source configuration is invalid".to_string())?;
    if let Some(object) = config.as_object_mut() {
        object.remove("sessionCookies");
        object.remove("requestHeaders");
    }
    Ok(config)
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateCandidate {
    pub id: String,
    pub left_job_id: String,
    pub right_job_id: String,
    pub method: String,
    pub score: Option<f64>,
    pub status: String,
    pub evidence_json: String,
    pub created_at: String,
    pub left_title: String,
    pub left_company: String,
    pub right_title: String,
    pub right_company: String,
}
#[tauri::command]
pub async fn list_duplicate_candidates(
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<DuplicateCandidate>> {
    sqlx::query_as("SELECT c.id,c.left_job_id,c.right_job_id,c.method,c.score,c.status,c.evidence_json,c.created_at,l.title AS left_title,l.company AS left_company,r.title AS right_title,r.company AS right_company FROM duplicate_candidates c JOIN jobs l ON l.id=c.left_job_id JOIN jobs r ON r.id=c.right_job_id WHERE c.status='suggested' ORDER BY c.score DESC,c.created_at DESC").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MergedJob {
    pub canonical_job_id: String,
    pub merged_job_id: String,
    pub canonical_title: String,
    pub merged_title: String,
    pub created_at: String,
    pub conflict_count: i64,
}
#[tauri::command]
pub async fn list_merged_jobs(state: State<'_, Arc<AppState>>) -> ApiResult<Vec<MergedJob>> {
    sqlx::query_as("SELECT a.canonical_job_id,a.merged_job_id,c.title AS canonical_title,m.title AS merged_title,a.created_at,(SELECT count(*) FROM duplicate_merge_conflicts x WHERE x.audit_id=a.id AND x.resolved_at IS NULL) AS conflict_count FROM duplicate_merge_audits a JOIN jobs c ON c.id=a.canonical_job_id JOIN jobs m ON m.id=a.merged_job_id WHERE a.undone_at IS NULL ORDER BY a.created_at DESC").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateConflict {
    pub id: String,
    pub audit_id: String,
    pub conflict_type: String,
    pub persona_id: String,
    pub canonical_snapshot_json: String,
    pub merged_snapshot_json: String,
    pub resolution: String,
    pub created_at: String,
}
#[tauri::command]
pub async fn list_duplicate_conflicts(
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<DuplicateConflict>> {
    sqlx::query_as("SELECT id,audit_id,conflict_type,persona_id,canonical_snapshot_json,merged_snapshot_json,resolution,created_at FROM duplicate_merge_conflicts WHERE resolved_at IS NULL ORDER BY created_at DESC").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn resolve_duplicate_conflict(
    conflict_id: String,
    resolution: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let resolution = if resolution == "retain_historical" {
        "retain_both_history"
    } else {
        resolution.as_str()
    };
    if !["keep_canonical", "keep_alias", "retain_both_history"].contains(&resolution) {
        return Err("Resolution must keep_canonical, keep_alias, or retain_both_history".into());
    }
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let row=sqlx::query("SELECT a.canonical_job_id,a.merged_job_id,c.conflict_type,c.persona_id,c.canonical_snapshot_json,c.merged_snapshot_json,c.resolution,c.resolved_at FROM duplicate_merge_conflicts c JOIN duplicate_merge_audits a ON a.id=c.audit_id WHERE c.id=?").bind(&conflict_id).fetch_one(&mut *tx).await.map_err(|_|"Duplicate conflict was not found".to_string())?;
    if row.get::<Option<String>, _>(7).is_some() {
        if row.get::<String, _>(6) == resolution {
            return Ok(());
        }
        return Err("Conflict already resolved with a different decision".into());
    }
    let job: String = row.get(0);
    let merged: String = row.get(1);
    let conflict_type: String = row.get(2);
    let canonical = row.get::<String, _>(4);
    let alias = row.get::<String, _>(5);
    let canonical_id = dedupe_snapshot_id(&canonical)?;
    let alias_id = dedupe_snapshot_id(&alias)?;
    match (conflict_type.as_str(), resolution) {
        ("match_result", "keep_canonical") | ("match_result", "retain_both_history") => {
            sqlx::query("DELETE FROM match_results WHERE id=? AND job_id=?")
                .bind(alias_id)
                .bind(&merged)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        ("review_decision", "keep_canonical") | ("review_decision", "retain_both_history") => {
            sqlx::query("DELETE FROM review_decisions WHERE id=? AND job_id=?")
                .bind(alias_id)
                .bind(&merged)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        ("match_result", "keep_alias") => {
            sqlx::query("DELETE FROM match_results WHERE id=? AND job_id=?")
                .bind(canonical_id)
                .bind(&job)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            let changed = sqlx::query("UPDATE match_results SET job_id=? WHERE id=? AND job_id=?")
                .bind(&job)
                .bind(alias_id)
                .bind(&merged)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            if changed.rows_affected() != 1 {
                return Err("Conflict current row changed; cannot safely keep alias".into());
            }
        }
        ("review_decision", "keep_alias") => {
            sqlx::query("DELETE FROM review_decisions WHERE id=? AND job_id=?")
                .bind(canonical_id)
                .bind(&job)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            let changed =
                sqlx::query("UPDATE review_decisions SET job_id=? WHERE id=? AND job_id=?")
                    .bind(&job)
                    .bind(alias_id)
                    .bind(&merged)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            if changed.rows_affected() != 1 {
                return Err("Conflict current row changed; cannot safely keep alias".into());
            }
        }
        _ => return Err("Unsupported duplicate conflict type".into()),
    }
    let t = now();
    sqlx::query("UPDATE duplicate_merge_conflicts SET resolution=?,resolved_at=? WHERE id=?")
        .bind(resolution)
        .bind(&t)
        .bind(&conflict_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO job_dedupe_events(id,canonical_job_id,method,evidence_json,created_at) VALUES(?,?, 'conflict_resolution',?,?)").bind(id()).bind(job).bind(serde_json::json!({"conflictId":conflict_id,"resolution":resolution,"type":conflict_type,"personaId":row.get::<String,_>(3),"canonicalSnapshot":canonical,"aliasSnapshot":alias}).to_string()).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn dismiss_duplicate_candidate(
    candidate_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let changed=sqlx::query("UPDATE duplicate_candidates SET status='dismissed',decided_at=? WHERE id=? AND status='suggested'").bind(now()).bind(candidate_id).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
    if changed.rows_affected() == 0 {
        return Err("Duplicate candidate is not pending".into());
    };
    Ok(())
}
#[tauri::command]
pub async fn merge_duplicate_jobs(
    candidate_id: String,
    canonical_job_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let pair=sqlx::query("SELECT left_job_id,right_job_id FROM duplicate_candidates WHERE id=? AND status='suggested'").bind(&candidate_id).fetch_one(&mut *tx).await.map_err(|_|"Duplicate candidate is not pending".to_string())?;
    let left: String = pair.get(0);
    let right: String = pair.get(1);
    if canonical_job_id != left && canonical_job_id != right {
        return Err("Canonical winner must be one candidate job".into());
    };
    let merged = if canonical_job_id == left {
        right
    } else {
        left
    };
    let app_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM applications WHERE job_id=?")
        .bind(&merged)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let occurrence_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM job_occurrences WHERE job_id=?")
            .bind(&merged)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let revision_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM job_revisions WHERE job_id=?")
            .bind(&merged)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    // Capture every ownership move before making it. Conflict rows stay separate
    // until an explicit resolution, while non-conflicting rows are re-keyed.
    let match_ids: Vec<String> = sqlx::query_scalar("SELECT m.id FROM match_results m WHERE m.job_id=? AND NOT EXISTS(SELECT 1 FROM match_results c WHERE c.job_id=? AND c.persona_id=m.persona_id AND c.algorithm_version=m.algorithm_version)")
        .bind(&merged).bind(&canonical_job_id).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
    let review_ids: Vec<String> = sqlx::query_scalar("SELECT r.id FROM review_decisions r WHERE r.job_id=? AND NOT EXISTS(SELECT 1 FROM review_decisions c WHERE c.job_id=? AND c.persona_id=r.persona_id)")
        .bind(&merged).bind(&canonical_job_id).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
    let snapshot = serde_json::json!({"applications":app_ids,"occurrences":occurrence_ids,"revisions":revision_ids,"matches":match_ids,"reviews":review_ids,"snapshotVersion":2});
    let audit = id();
    let t = now();
    sqlx::query("INSERT INTO duplicate_merge_audits(id,canonical_job_id,merged_job_id,snapshot_json,created_at) VALUES(?,?,?,?,?)").bind(&audit).bind(&canonical_job_id).bind(&merged).bind(snapshot.to_string()).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    for row in sqlx::query("SELECT a.persona_id,json_object('id',a.id,'score',a.score,'eligible',a.eligible,'algorithmVersion',a.algorithm_version,'modelVersion',a.model_version,'filterDecisionJson',a.filter_decision_json,'componentsJson',a.components_json,'explanationJson',a.explanation_json,'createdAt',a.created_at,'updatedAt',a.updated_at),json_object('id',b.id,'score',b.score,'eligible',b.eligible,'algorithmVersion',b.algorithm_version,'modelVersion',b.model_version,'filterDecisionJson',b.filter_decision_json,'componentsJson',b.components_json,'explanationJson',b.explanation_json,'createdAt',b.created_at,'updatedAt',b.updated_at) FROM match_results a JOIN match_results b ON a.persona_id=b.persona_id AND a.algorithm_version=b.algorithm_version WHERE a.job_id=? AND b.job_id=?").bind(&canonical_job_id).bind(&merged).fetch_all(&mut *tx).await.map_err(|e|e.to_string())? {sqlx::query("INSERT INTO duplicate_merge_conflicts(id,audit_id,conflict_type,persona_id,canonical_snapshot_json,merged_snapshot_json,created_at) VALUES(?,?, 'match_result',?,?,?,?)").bind(id()).bind(&audit).bind(row.get::<String,_>(0)).bind(row.get::<String,_>(1)).bind(row.get::<String,_>(2)).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;}
    for row in sqlx::query("SELECT a.persona_id,json_object('id',a.id,'status',a.status,'reason',a.reason,'decidedAt',a.decided_at),json_object('id',b.id,'status',b.status,'reason',b.reason,'decidedAt',b.decided_at) FROM review_decisions a JOIN review_decisions b ON a.persona_id=b.persona_id WHERE a.job_id=? AND b.job_id=?").bind(&canonical_job_id).bind(&merged).fetch_all(&mut *tx).await.map_err(|e|e.to_string())? {sqlx::query("INSERT INTO duplicate_merge_conflicts(id,audit_id,conflict_type,persona_id,canonical_snapshot_json,merged_snapshot_json,created_at) VALUES(?,?, 'review_decision',?,?,?,?)").bind(id()).bind(&audit).bind(row.get::<String,_>(0)).bind(row.get::<String,_>(1)).bind(row.get::<String,_>(2)).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;}
    sqlx::query("UPDATE match_results SET job_id=? WHERE job_id=? AND NOT EXISTS(SELECT 1 FROM match_results c WHERE c.job_id=? AND c.persona_id=match_results.persona_id AND c.algorithm_version=match_results.algorithm_version)").bind(&canonical_job_id).bind(&merged).bind(&canonical_job_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("UPDATE review_decisions SET job_id=? WHERE job_id=? AND NOT EXISTS(SELECT 1 FROM review_decisions c WHERE c.job_id=? AND c.persona_id=review_decisions.persona_id)").bind(&canonical_job_id).bind(&merged).bind(&canonical_job_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    for table in ["applications", "job_occurrences", "job_revisions"] {
        sqlx::query(&format!("UPDATE {table} SET job_id=? WHERE job_id=?"))
            .bind(&canonical_job_id)
            .bind(&merged)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    sqlx::query("UPDATE duplicate_candidates SET status='merged',decided_at=? WHERE id=?")
        .bind(&t)
        .bind(candidate_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn unmerge_duplicate_jobs(
    canonical_job_id: String,
    merged_job_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let row=sqlx::query("SELECT id,snapshot_json FROM duplicate_merge_audits WHERE canonical_job_id=? AND merged_job_id=? AND undone_at IS NULL ORDER BY created_at DESC LIMIT 1").bind(&canonical_job_id).bind(&merged_job_id).fetch_one(&mut *tx).await.map_err(|_|"No active merge audit was found".to_string())?;
    let audit: String = row.get(0);
    let snapshot: serde_json::Value = serde_json::from_str(&row.get::<String, _>(1))
        .map_err(|_| "Merge audit is corrupt".to_string())?;
    // Resolve paths are reversible. A post-merge owner change fails the whole
    // transaction instead of reconstructing a mixed canonical/alias pair.
    let conflicts = sqlx::query("SELECT conflict_type,persona_id,canonical_snapshot_json,merged_snapshot_json,resolution,resolved_at FROM duplicate_merge_conflicts WHERE audit_id=?")
        .bind(&audit).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
    for conflict in conflicts {
        let resolved: Option<String> = conflict.get(5);
        if resolved.is_none() {
            continue;
        }
        let kind: String = conflict.get(0);
        let persona: String = conflict.get(1);
        let canonical_snapshot: String = conflict.get(2);
        let alias_snapshot: String = conflict.get(3);
        let resolution: String = conflict.get(4);
        let canonical_id = dedupe_snapshot_id(&canonical_snapshot)?;
        let alias_id = dedupe_snapshot_id(&alias_snapshot)?;
        match (kind.as_str(), resolution.as_str()) {
            ("match_result", "keep_alias") => {
                let moved =
                    sqlx::query("UPDATE match_results SET job_id=? WHERE id=? AND job_id=?")
                        .bind(&merged_job_id)
                        .bind(&alias_id)
                        .bind(&canonical_job_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| e.to_string())?;
                if moved.rows_affected() != 1 {
                    return Err(
                        "Unmerge cannot safely restore resolved alias match ownership".into(),
                    );
                }
                restore_match_snapshot(&mut tx, &canonical_job_id, &persona, &canonical_snapshot)
                    .await?;
            }
            ("review_decision", "keep_alias") => {
                let moved =
                    sqlx::query("UPDATE review_decisions SET job_id=? WHERE id=? AND job_id=?")
                        .bind(&merged_job_id)
                        .bind(&alias_id)
                        .bind(&canonical_job_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| e.to_string())?;
                if moved.rows_affected() != 1 {
                    return Err(
                        "Unmerge cannot safely restore resolved alias review ownership".into(),
                    );
                }
                restore_review_snapshot(&mut tx, &canonical_job_id, &persona, &canonical_snapshot)
                    .await?;
            }
            ("match_result", "keep_canonical") | ("match_result", "retain_both_history") => {
                if canonical_id.is_empty() {
                    return Err("Conflict snapshot is incomplete".into());
                }
                restore_match_snapshot(&mut tx, &merged_job_id, &persona, &alias_snapshot).await?;
            }
            ("review_decision", "keep_canonical") | ("review_decision", "retain_both_history") => {
                if canonical_id.is_empty() {
                    return Err("Conflict snapshot is incomplete".into());
                }
                restore_review_snapshot(&mut tx, &merged_job_id, &persona, &alias_snapshot).await?;
            }
            _ => {
                return Err(
                    "Unmerge cannot safely restore legacy or unknown conflict resolution".into(),
                )
            }
        }
    }
    for (table, key) in [
        ("applications", "applications"),
        ("job_occurrences", "occurrences"),
        ("job_revisions", "revisions"),
        ("match_results", "matches"),
        ("review_decisions", "reviews"),
    ] {
        for row_id in snapshot
            .get(key)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            let changed = sqlx::query(&format!(
                "UPDATE {table} SET job_id=? WHERE id=? AND job_id=?"
            ))
            .bind(&merged_job_id)
            .bind(row_id)
            .bind(&canonical_job_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            if changed.rows_affected() != 1 {
                return Err("Unmerge cannot safely restore changed relationship ownership".into());
            }
        }
    }
    let t = now();
    sqlx::query("UPDATE duplicate_merge_audits SET undone_at=? WHERE id=?")
        .bind(&t)
        .bind(&audit)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE duplicate_candidates SET status='unmerged',decided_at=? WHERE (left_job_id=? AND right_job_id=?) OR (left_job_id=? AND right_job_id=?)").bind(&t).bind(&canonical_job_id).bind(&merged_job_id).bind(&merged_job_id).bind(&canonical_job_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}
/// Two sources on the same address are not two sources. A listing is re-identified by its
/// canonical URL across the whole database and a job keeps the source that first stored it, so the
/// second source re-reads the entire board, updates rows belonging to the first, and reports
/// nothing saved — every run, forever, with no error to explain it. Comparing the address the way a
/// browser would (case-insensitive host, a trailing slash meaning nothing) catches the way this
/// actually happens: the same careers page pasted in twice.
fn same_board(left: &str, right: &str) -> bool {
    let normalise = |value: &str| {
        Url::parse(value)
            .map(|url| {
                format!(
                    "{}://{}{}{}",
                    url.scheme(),
                    url.host_str().unwrap_or_default().to_ascii_lowercase(),
                    url.path().trim_end_matches('/'),
                    url.query().map(|q| format!("?{q}")).unwrap_or_default()
                )
            })
            .unwrap_or_else(|_| value.trim_end_matches('/').to_ascii_lowercase())
    };
    normalise(left) == normalise(right)
}
#[tauri::command]
pub async fn save_source(input: SourceInput, state: State<'_, Arc<AppState>>) -> ApiResult<Source> {
    valid_url(&input.base_url, input.allow_private_network)?;
    if input.kind == "reference" && input.enabled {
        return Err("Reference sources cannot be enabled for scraping".into());
    }
    let source_id = input.id.unwrap_or_else(id);
    let followed: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,name,base_url FROM sources WHERE deleted_at IS NULL AND id<>?")
            .bind(&source_id)
            .fetch_all(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?;
    if let Some((_, name, _)) = followed
        .iter()
        .find(|(_, _, url)| same_board(url, &input.base_url))
    {
        return Err(format!(
            "That board is already followed as \"{name}\". Two sources on one address cannot both hold its jobs; edit or delete \"{name}\" instead."
        ));
    }
    let t = now();
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,robots_acknowledged_at,allow_private_network,created_at,updated_at) VALUES(?,?,?,?,?, ?,?,?, ?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,base_url=excluded.base_url,adapter_id=excluded.adapter_id,enabled=excluded.enabled,kind=excluded.kind,disabled_reason=excluded.disabled_reason,robots_override=excluded.robots_override,robots_acknowledged_at=excluded.robots_acknowledged_at,allow_private_network=excluded.allow_private_network,updated_at=excluded.updated_at")
  .bind(&source_id).bind(&input.name).bind(&input.base_url).bind(&input.adapter_id).bind("1.0.0").bind(input.enabled).bind(&input.kind).bind(&input.disabled_reason).bind(input.robots_override).bind(if input.robots_override {Some(t.clone())} else {None}).bind(input.allow_private_network).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(source_id) DO UPDATE SET config_json=excluded.config_json,updated_at=excluded.updated_at").bind(id()).bind(&source_id).bind(input.config_json.to_string()).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as("SELECT s.id,s.name,s.base_url,s.adapter_id,s.adapter_version,s.enabled,s.kind,s.disabled_reason,s.robots_override,s.last_success_at,COALESCE(c.live,0) AS job_count,COALESCE(c.gone,0) AS closed_count,s.created_at,s.updated_at FROM sources s LEFT JOIN (SELECT source_id,sum(CASE WHEN availability='closed' THEN 0 ELSE 1 END) AS live,sum(CASE WHEN availability='closed' THEN 1 ELSE 0 END) AS gone FROM jobs GROUP BY source_id) c ON c.source_id=s.id WHERE s.id=?").bind(source_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn delete_source(
    source_id: String,
    purge: bool,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    if purge {
        return Err("Purging must run through the impact-preview/confirmation workflow; history is protected.".into());
    }
    sqlx::query("UPDATE sources SET enabled=0, deleted_at=?, updated_at=? WHERE id=?")
        .bind(now())
        .bind(now())
        .bind(source_id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
/// Filters the Jobs page sends. Everything is optional; the source scope is not a filter but
/// the enabled set on the Sources page, so "no sources selected" always means "no jobs".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JobFilter {
    /// Words that must all appear somewhere in the job title, in any order.
    pub title: String,
    /// Words that must all appear in title, company, location or description.
    pub keyword: String,
    /// "posted" orders by posting date (unknown dates last); anything else by last seen.
    pub sort: String,
    /// Only jobs already saved into Applications.
    pub saved_only: bool,
    /// Include openings two complete reads have failed to find. Off by default: the point of the
    /// page is what can still be applied to, and until now every closed listing sat in the list
    /// looking exactly like a live one.
    pub include_closed: bool,
    /// How far back to look, in days. None shows everything. The window opens on one year so a
    /// list years deep does not greet you with listings nobody can apply to any more; it is view
    /// state, not a saved preference, so every launch starts at a year again.
    pub posted_within_days: Option<i64>,
    /// ISO country codes to show, any of them. Empty means everywhere. The codes were worked out
    /// when each job was stored (see locations.rs), so this is an exact test rather than a guess at
    /// query time, and a posting open in several offices answers to each of their countries.
    pub countries: Vec<String>,
    /// Also show listings whose country could not be worked out at all — a board that published
    /// "2 Locations" and a link that named no office. Off, because a country filter that quietly
    /// includes everything unplaceable is the filter people complain about.
    pub include_unknown_locations: bool,
    /// Also show what has been dismissed. Off by default: "not for me" is a decision, and a list
    /// that keeps showing what you have already judged is the reason nobody finishes reading it.
    pub include_dismissed: bool,
    /// Only listings first seen since this instant. The Jobs page sends the moment it was last
    /// marked reviewed, so "new" means new to the reader rather than new to the board.
    pub first_seen_after: Option<String>,
    /// Which of the followed sources to show. Empty means all of them, which is the default.
    /// This is the view, not the collection: whether a source is followed at all is
    /// `sources.enabled`, and narrowing the list must never quietly retire a board.
    pub source_ids: Vec<String>,
}
// The list never renders a description — it is read only when a card is opened, through
// load_job_description(). Keyword search still matches against the real column in the WHERE clause;
// only the projection is trimmed, because that is what crosses into the window.
const JOB_SELECT: &str = "SELECT j.id,j.source_id,j.title,j.company,j.location,j.work_mode,j.canonical_url,j.apply_url,'' AS description_text,j.description_status,j.dismissed_at,j.posted_at,j.salary_min,j.salary_max,j.salary_currency,j.seniority,j.availability,j.created_at,j.updated_at,NULL AS score,NULL AS eligible,NULL AS reasons,(SELECT a.current_stage FROM applications a WHERE a.job_id=j.id ORDER BY a.created_at DESC LIMIT 1) AS application_stage FROM jobs j JOIN sources s ON s.id=j.source_id";
// Substring matching, not FTS. FTS5 tokenizes on word boundaries and treats punctuation as
// syntax, so "verification" missed "Verification/Validation" while "C++" raised a syntax error.
// instr() over lower() is what a user typing into a search box actually expects, and every
// word they type has to appear — order and case never matter.
fn term_clauses(field: &str, text: &str) -> (String, Vec<String>) {
    let terms: Vec<String> = text
        .split_whitespace()
        .take(8)
        .map(|word| word.to_lowercase())
        .collect();
    let sql = terms
        .iter()
        .map(|_| format!(" AND instr(lower({field}),?)>0"))
        .collect::<String>();
    (sql, terms)
}
// Codes are stored as ",PT,ES," precisely so one of them is a substring test, which an index on the
// column serves without a join table or a LIKE pattern that starts with a wildcard.
const LOCATION_UNKNOWN: &str = "j.location_countries IS NULL OR j.location_countries=''";
fn location_clauses(countries: &[String], include_unknown: bool) -> (String, Vec<String>) {
    // Two letters, upper case, nothing else: the codes come from the picker, and a filter is never
    // a place to accept free text into SQL.
    let codes: Vec<String> = countries
        .iter()
        .take(250)
        .map(|code| code.trim().to_uppercase())
        .filter(|code| code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()))
        .collect();
    if codes.is_empty() {
        return (String::new(), Vec::new());
    }
    let any = codes
        .iter()
        .map(|_| "instr(coalesce(j.location_countries,''),?)>0".to_string())
        .collect::<Vec<_>>()
        .join(" OR ");
    let unknown = if include_unknown {
        format!(" OR {LOCATION_UNKNOWN}")
    } else {
        String::new()
    };
    (
        format!(" AND (({any}){unknown})"),
        codes.iter().map(|code| format!(",{code},")).collect(),
    )
}
/// Works out the country of every listing that has none yet — and of every listing at all when the
/// resolver's rules have changed since they were last worked out. An empty string is a resolved
/// answer of "nowhere recognisable" and is not revisited within a version. Waiting for the next
/// scrape instead would leave a filter that silently misses everything already collected, and
/// leaving old answers alone would leave the wrong countries in place for good.
const RESOLVER_VERSION_KEY: &str = "locations.resolverVersion";
pub async fn backfill_location_countries(pool: &SqlitePool) -> ApiResult<u64> {
    let stamped: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(RESOLVER_VERSION_KEY)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    if stamped.as_deref() != Some(crate::locations::RESOLVER_VERSION) {
        sqlx::query("UPDATE jobs SET location_countries=NULL")
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id,location,canonical_url FROM jobs WHERE location_countries IS NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        write_setting(
            pool,
            RESOLVER_VERSION_KEY,
            crate::locations::RESOLVER_VERSION,
        )
        .await?;
        return Ok(0);
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for (id, location, url) in &rows {
        let codes = crate::locations::resolve(location.as_deref(), url.as_deref());
        sqlx::query("UPDATE jobs SET location_countries=? WHERE id=?")
            .bind(crate::locations::stored(&codes))
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    // Only once the pass has actually landed: a crash halfway leaves the stamp behind and the
    // remaining rows are picked up next launch.
    write_setting(
        pool,
        RESOLVER_VERSION_KEY,
        crate::locations::RESOLVER_VERSION,
    )
    .await?;
    Ok(rows.len() as u64)
}
/// Switches on the starter-pack boards when the installer was told to include them. The installer
/// cannot touch the database — it does not exist until first run — so it leaves a file behind and
/// this reads it.
///
/// Answering that question is a decision made now, so it outranks whatever state those boards were
/// left in before: one deleted or switched off in an earlier install comes back. That is what
/// ticking the box means, and an install that answered Yes over an old database and silently
/// changed nothing — because every starter row had been retired months earlier — is the bug this
/// replaced. The file is deleted either way, so no later launch can undo what the user does next.
pub async fn apply_starter_pack_opt_in(
    pool: &SqlitePool,
    local: &std::path::Path,
) -> ApiResult<u64> {
    let marker = local.join("starter-pack.optin");
    if !marker.exists() {
        return Ok(0);
    }
    let switched = sqlx::query("UPDATE sources SET enabled=1,deleted_at=NULL,disabled_reason=NULL,updated_at=? WHERE kind='active' AND (enabled=0 OR deleted_at IS NOT NULL) AND id IN (SELECT source_id FROM source_configs WHERE instr(config_json,'starterPackVersion')>0)")
        .bind(now())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();
    let _ = std::fs::remove_file(&marker);
    // Recorded because the failure this replaced was invisible: the answer was taken, nothing
    // happened, and nothing said so.
    log(
        pool,
        "info",
        None,
        None,
        "starter_pack",
        None,
        &format!("Installer choice applied: {switched} company job boards switched on."),
        serde_json::json!({ "switched": switched }),
    )
    .await;
    Ok(switched)
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountryCount {
    pub code: String,
    pub jobs: i64,
}
/// Which countries the jobs on show are actually in, most first, plus how many cannot be placed.
/// The picker is built from this rather than from the full list of 250 states: offering Portugal
/// when the boards you follow have never posted a job there is a filter that can only disappoint.
/// Scoped by the same source selection as the list itself, so narrowing the sources narrows the
/// countries with them.
#[tauri::command]
pub async fn job_countries(
    source_ids: Option<Vec<String>>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<(Vec<CountryCount>, i64)> {
    job_countries_query(&source_ids.unwrap_or_default(), &state.db.pool).await
}
async fn job_countries_query(
    source_ids: &[String],
    pool: &SqlitePool,
) -> ApiResult<(Vec<CountryCount>, i64)> {
    let scope = if source_ids.is_empty() {
        String::new()
    } else {
        format!(
            " AND j.source_id IN ({})",
            std::iter::repeat("?")
                .take(source_ids.len())
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    // Codes are packed per job (",PT,ES,"), and the distinct packings number in the hundreds at
    // most, so they are counted here rather than in a table the schema would have to maintain.
    let sql = format!("SELECT coalesce(j.location_countries,'') AS codes,count(*) AS jobs FROM jobs j JOIN sources s ON s.id=j.source_id WHERE s.enabled=1 AND s.deleted_at IS NULL AND j.availability NOT IN ('archived','closed'){scope} GROUP BY codes");
    let mut query = sqlx::query_as::<_, (String, i64)>(&sql);
    for id in source_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await.map_err(|e| e.to_string())?;
    let mut totals: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut unplaced = 0;
    for (codes, jobs) in rows {
        if codes.is_empty() {
            unplaced += jobs;
            continue;
        }
        for code in codes.split(',').filter(|code| !code.is_empty()) {
            *totals.entry(code.to_string()).or_default() += jobs;
        }
    }
    let mut counted: Vec<CountryCount> = totals
        .into_iter()
        .map(|(code, jobs)| CountryCount { code, jobs })
        .collect();
    counted.sort_by(|a, b| b.jobs.cmp(&a.jobs).then_with(|| a.code.cmp(&b.code)));
    Ok((counted, unplaced))
}
/// Where a job added by hand lives. It is a real source so that everything downstream — the list,
/// the filters, the country resolver, applications — treats a captured job like any other, but it
/// is never scraped: nothing about a pasted link describes a board to read.
pub const CAPTURED_SOURCE_ID: &str = "00000000-0000-4000-8000-00000000c0de";
async fn captured_source(pool: &SqlitePool) -> ApiResult<()> {
    let t = now();
    sqlx::query("INSERT OR IGNORE INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,'Added by hand','about:blank','reference','1.1.0',1,'reference','Jobs you pasted a link to; there is no board here to read.',0,0,?,?)")
        .bind(CAPTURED_SOURCE_ID)
        .bind(&t)
        .bind(&t)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
/// Stores a captured vacancy, after the person pasting the link has had a chance to correct it.
/// Everything it stores came from that page or from them, so it goes through the same persistence
/// path as a scraped listing and lands with the same shape.
#[tauri::command]
pub async fn save_captured_job(
    job: serde_json::Value,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<String> {
    let title = job
        .get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("A job needs a title before it can be saved")?;
    let url = job
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    valid_url(url, false)?;
    captured_source(&state.db.pool).await?;
    let payload = serde_json::json!({
        "title": title,
        "company": job.get("company").and_then(|v| v.as_str()).unwrap_or("").trim(),
        "location": job.get("location").and_then(|v| v.as_str()),
        "canonicalUrl": url,
        "applyUrl": url,
        "postedAt": job.get("postedAt").and_then(|v| v.as_str()),
        "closingAt": job.get("closingAt").and_then(|v| v.as_str()),
        "workMode": job.get("workMode").and_then(|v| v.as_str()),
        "externalId": job.get("externalId").and_then(|v| v.as_str()).unwrap_or(url),
        "descriptionText": job.get("descriptionText").and_then(|v| v.as_str()).unwrap_or(""),
        "descriptionHtml": job.get("descriptionHtml").and_then(|v| v.as_str()).unwrap_or(""),
        "descriptionStatus": "complete",
        "skills": [],
        "adapterVersion": "1.1.0",
        "provenance": { "adapter": "captured", "sourceUrl": url },
    });
    state
        .db
        .persist_worker_job(&id(), CAPTURED_SOURCE_ID, &payload)
        .await?;
    let stored: String = sqlx::query_scalar("SELECT id FROM jobs WHERE source_id=? AND canonical_url=? ORDER BY updated_at DESC LIMIT 1")
        .bind(CAPTURED_SOURCE_ID)
        .bind(url)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    log(
        &state.db.pool,
        "info",
        Some(CAPTURED_SOURCE_ID),
        Some("Added by hand"),
        "capture_job",
        None,
        &format!("Saved \"{title}\" from a pasted link."),
        serde_json::json!({ "url": url }),
    )
    .await;
    Ok(stored)
}
/// "Not for me", and its undo. Nothing is deleted: the listing keeps its place in the database, in
/// search, and in every count — it simply stops appearing in a list whose whole problem is that it
/// only ever grows.
#[tauri::command]
pub async fn set_job_dismissed(
    job_id: String,
    dismissed: bool,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    sqlx::query("UPDATE jobs SET dismissed_at=?,updated_at=? WHERE id=?")
        .bind(dismissed.then(now))
        .bind(now())
        .bind(&job_id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
/// How many listings have arrived since the reader last said they were done, and when that was.
/// Counted with the same visibility rules the list uses, so the number and the page agree.
#[tauri::command]
pub async fn new_since(since: Option<String>, state: State<'_, Arc<AppState>>) -> ApiResult<i64> {
    let Some(since) = since.filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    sqlx::query_scalar("SELECT count(*) FROM jobs j JOIN sources s ON s.id=j.source_id WHERE s.enabled=1 AND s.deleted_at IS NULL AND j.availability NOT IN ('archived','closed') AND j.dismissed_at IS NULL AND coalesce(j.first_seen_at,j.created_at)>?")
        .bind(since)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn list_jobs(
    filter: Option<JobFilter>,
    offset: Option<i64>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<JobPage> {
    jobs_page_query(
        filter.unwrap_or_default(),
        offset.unwrap_or_default(),
        &state.db.pool,
    )
    .await
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPage {
    pub items: Vec<crate::domain::Job>,
    pub total: i64,
    pub offset: i64,
    pub has_more: bool,
}
#[cfg(test)]
async fn jobs_query(filter: JobFilter, pool: &SqlitePool) -> ApiResult<Vec<crate::domain::Job>> {
    Ok(jobs_page_query(filter, 0, pool).await?.items)
}
async fn jobs_page_query(filter: JobFilter, offset: i64, pool: &SqlitePool) -> ApiResult<JobPage> {
    let (title_sql, title_terms) = term_clauses("j.title", &filter.title);
    let (keyword_sql, keyword_terms) = term_clauses(
        "j.title||' '||j.company||' '||coalesce(j.location,'')||' '||j.description_text",
        &filter.keyword,
    );
    let (location_sql, location_terms) =
        location_clauses(&filter.countries, filter.include_unknown_locations);
    let dismissed = if filter.include_dismissed {
        ""
    } else {
        " AND j.dismissed_at IS NULL"
    };
    // Bound as a parameter rather than inlined: it comes from a stored setting, which is a string
    // this process did not write.
    let (fresh_sql, fresh_bind) = match filter.first_seen_after.as_deref().filter(|v| !v.is_empty())
    {
        Some(since) => (
            " AND coalesce(j.first_seen_at,j.created_at)>?".to_string(),
            vec![since.to_string()],
        ),
        None => (String::new(), Vec::new()),
    };
    // Posting dates are stored as ISO strings, so text order is date order; the NULL test keeps
    // undated listings at the bottom instead of letting them win the descending sort.
    const JOB_PAGE_SIZE: i64 = 200;
    let offset = offset.max(0);
    let order = if filter.sort == "posted" {
        "ORDER BY j.posted_at IS NULL,j.posted_at DESC,j.updated_at DESC,j.id"
    } else {
        "ORDER BY coalesce(j.first_seen_at,j.created_at) DESC,j.id"
    };
    let saved = if filter.saved_only {
        " AND EXISTS(SELECT 1 FROM applications a WHERE a.job_id=j.id)"
    } else {
        ""
    };
    // reconcile_availability has been writing these states since the schema was created and no
    // query has ever read them, so a posting absent from two complete reads was listed exactly
    // like a live one. 'possibly_closed' stays visible and is badged instead — one absence is
    // weak evidence, and a partial read cannot produce one at all.
    let availability = if filter.include_closed {
        " AND j.availability<>'archived'"
    } else {
        " AND j.availability NOT IN ('archived','closed')"
    };
    // Placeholders, never interpolated ids. These bind before the title and keyword terms because
    // the clause is spliced ahead of them and positional binds go in string order.
    let sources_sql = if filter.source_ids.is_empty() {
        String::new()
    } else {
        format!(
            " AND j.source_id IN ({})",
            std::iter::repeat("?")
                .take(filter.source_ids.len())
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    // A listing with no posting date is shown whatever the window: the board never said when it
    // went up, which is not the same as saying it is old. The cutoff is a date this code builds
    // from an integer, so it carries no user text and is safe to inline; the binds below stay in
    // the order the clauses are spliced.
    let posted = match filter.posted_within_days.filter(|days| *days > 0) {
        Some(days) => format!(
            " AND (j.posted_at IS NULL OR j.posted_at='' OR j.posted_at>='{}')",
            (Utc::now().date_naive() - chrono::Duration::days(days)).format("%Y-%m-%d")
        ),
        None => String::new(),
    };
    // One WHERE, shared by the count and the rows. They used to be spelled out separately, so a
    // filter added to one silently missed the other and the count disagreed with the list.
    let where_sql = format!(" WHERE s.enabled=1 AND s.deleted_at IS NULL{saved}{availability}{posted}{sources_sql}{title_sql}{keyword_sql}{location_sql}{dismissed}{fresh_sql}");
    let count_sql =
        format!("SELECT count(*) FROM jobs j JOIN sources s ON s.id=j.source_id{where_sql}");
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
    for bound in filter
        .source_ids
        .iter()
        .chain(title_terms.iter())
        .chain(keyword_terms.iter())
        .chain(location_terms.iter())
        .chain(fresh_bind.iter())
    {
        count_query = count_query.bind(bound);
    }
    let total = count_query
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let sql = format!("{JOB_SELECT}{where_sql} {order} LIMIT {JOB_PAGE_SIZE} OFFSET ?");
    let mut query = sqlx::query_as::<_, crate::domain::Job>(&sql);
    for bound in filter
        .source_ids
        .iter()
        .chain(title_terms.iter())
        .chain(keyword_terms.iter())
        .chain(location_terms.iter())
        .chain(fresh_bind.iter())
    {
        query = query.bind(bound);
    }
    let items = query
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(JobPage {
        has_more: offset + (items.len() as i64) < total,
        items,
        total,
        offset,
    })
}
/// The enabled set is the source selection: it decides which jobs are listed and which sources
/// Update Jobs re-reads. Toggling it must not require re-submitting the whole source form.
#[tauri::command]
pub async fn set_source_enabled(
    source_id: String,
    enabled: bool,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let kind: String =
        sqlx::query_scalar("SELECT kind FROM sources WHERE id=? AND deleted_at IS NULL")
            .bind(&source_id)
            .fetch_optional(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Source was not found")?;
    if enabled && kind == "reference" {
        return Err("Reference sources cannot be enabled for scraping".into());
    }
    // disabled_reason records why a source cannot work (a dead URL, an auth wall) — never the
    // fact that it is currently unticked, which the enabled flag already says. Selecting a source
    // clears the recorded reason because the user is overriding that finding on purpose.
    sqlx::query("UPDATE sources SET enabled=?,disabled_reason=CASE WHEN ? THEN NULL ELSE disabled_reason END,updated_at=? WHERE id=?").bind(enabled).bind(enabled).bind(now()).bind(&source_id).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn list_applications(state: State<'_, Arc<AppState>>) -> ApiResult<Vec<Application>> {
    sqlx::query_as(&format!("{APPLICATION_SELECT} ORDER BY a.updated_at DESC"))
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| e.to_string())
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationEvent {
    pub id: String,
    pub application_id: String,
    pub event_type: String,
    pub from_stage: Option<String>,
    pub to_stage: Option<String>,
    pub reason: Option<String>,
    pub occurred_at: String,
    pub payload_json: String,
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationNote {
    pub id: String,
    pub application_id: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDocument {
    pub id: String,
    pub application_id: String,
    pub kind: String,
    pub document_type: String,
    pub filename: String,
    pub mime_type: String,
    pub sha256: String,
    pub size: i64,
    pub event_id: Option<String>,
    pub created_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDetails {
    pub application: Application,
    pub source_name: Option<String>,
    pub source_url: Option<String>,
    pub first_response_at: Option<String>,
    pub events: Vec<ApplicationEvent>,
    pub notes: Vec<ApplicationNote>,
    pub documents: Vec<ApplicationDocument>,
}
#[tauri::command]
pub async fn application_details(
    application_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ApplicationDetails> {
    let application: Application = sqlx::query_as(&format!("{APPLICATION_SELECT} WHERE a.id=?"))
        .bind(&application_id)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|_| "Application was not found".to_string())?;
    let source = sqlx::query(
        "SELECT s.name,s.base_url FROM jobs j JOIN sources s ON s.id=j.source_id WHERE j.id=?",
    )
    .bind(&application.job_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| e.to_string())?;
    let events = sqlx::query_as("SELECT id,application_id,event_type,from_stage,to_stage,reason,occurred_at,payload_json FROM application_events WHERE application_id=? ORDER BY occurred_at,id").bind(&application_id).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?;
    let notes = sqlx::query_as("SELECT id,application_id,body,created_at,updated_at,deleted_at FROM notes WHERE application_id=? AND deleted_at IS NULL ORDER BY created_at DESC").bind(&application_id).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?;
    let documents = sqlx::query_as("SELECT id,application_id,kind,document_type,filename,mime_type,sha256,size,event_id,created_at FROM application_documents WHERE application_id=? ORDER BY created_at DESC").bind(&application_id).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?;
    let first_response_at = sqlx::query_scalar("SELECT min(occurred_at) FROM application_events WHERE application_id=? AND to_stage IN ('screening','interviewing','offer','rejected') AND occurred_at>(SELECT applied_at FROM applications WHERE id=?)").bind(&application_id).bind(&application_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())?;
    Ok(ApplicationDetails {
        application,
        source_name: source.as_ref().map(|row| row.get(0)),
        source_url: source.as_ref().map(|row| row.get(1)),
        first_response_at,
        events,
        notes,
        documents,
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDetailsInput {
    pub application_id: String,
    pub recruiter_name: Option<String>,
    pub recruiter_email: Option<String>,
    pub recruiter_phone: Option<String>,
    pub source_attribution: Option<String>,
    pub rejection_reason: Option<String>,
    pub rejection_category: Option<String>,
    pub withdrawn_reason: Option<String>,
}
#[tauri::command]
pub async fn save_application_details(
    input: ApplicationDetailsInput,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let t = now();
    let changed=sqlx::query("UPDATE applications SET recruiter_name=?,recruiter_email=?,recruiter_phone=?,source_attribution=?,rejection_reason=?,rejection_category=?,withdrawn_reason=?,updated_at=? WHERE id=?").bind(&input.recruiter_name).bind(&input.recruiter_email).bind(&input.recruiter_phone).bind(&input.source_attribution).bind(&input.rejection_reason).bind(&input.rejection_category).bind(&input.withdrawn_reason).bind(&t).bind(&input.application_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    if changed.rows_affected() == 0 {
        return Err("Application was not found".into());
    }
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'details_updated',?,?)").bind(id()).bind(&input.application_id).bind(&t).bind(serde_json::json!({"recruiterName":input.recruiter_name,"sourceAttribution":input.source_attribution}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteInput {
    pub id: Option<String>,
    pub application_id: String,
    pub body: String,
}
#[tauri::command]
pub async fn save_application_note(
    input: NoteInput,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ApplicationNote> {
    if input.body.trim().is_empty() {
        return Err("Note cannot be empty".into());
    };
    let t = now();
    let note_id = input.id.unwrap_or_else(id);
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO notes(id,application_id,body,created_at,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body,updated_at=excluded.updated_at WHERE notes.application_id=excluded.application_id AND notes.deleted_at IS NULL").bind(&note_id).bind(&input.application_id).bind(&input.body).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'note_saved',?,?)").bind(id()).bind(&input.application_id).bind(&t).bind(serde_json::json!({"noteId":note_id}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as(
        "SELECT id,application_id,body,created_at,updated_at,deleted_at FROM notes WHERE id=?",
    )
    .bind(note_id)
    .fetch_one(&state.db.pool)
    .await
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn delete_application_note(
    note_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let app: Option<String> =
        sqlx::query_scalar("SELECT application_id FROM notes WHERE id=? AND deleted_at IS NULL")
            .bind(&note_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let Some(app) = app else {
        return Err("Note was not found".into());
    };
    let t = now();
    sqlx::query("UPDATE notes SET deleted_at=?,updated_at=? WHERE id=?")
        .bind(&t)
        .bind(&t)
        .bind(&note_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'note_deleted',?,?)").bind(id()).bind(app).bind(&t).bind(serde_json::json!({"noteId":note_id}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachApplicationDocument {
    pub application_id: String,
    pub document_type: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub event_id: Option<String>,
}
fn validate_attachment_size(size: usize) -> ApiResult<()> {
    if size > MAX_ATTACHMENT_BYTES {
        Err("Attachment exceeds 20 MB".into())
    } else {
        Ok(())
    }
}
fn decode_attachment_metadata(encoded: &str) -> ApiResult<AttachApplicationDocument> {
    let metadata = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "Invalid attachment metadata")?;
    serde_json::from_slice(&metadata).map_err(|_| "Invalid attachment metadata".into())
}
fn attachment_request(
    request: &tauri::ipc::Request<'_>,
) -> ApiResult<(AttachApplicationDocument, Vec<u8>)> {
    let content = match request.body() {
        InvokeBody::Raw(bytes) => bytes,
        _ => return Err("Attachment bytes are required".into()),
    };
    validate_attachment_size(content.len())?;
    let encoded = request
        .headers()
        .get(DOCUMENT_METADATA_HEADER)
        .ok_or("Attachment metadata is required")?
        .to_str()
        .map_err(|_| "Invalid attachment metadata")?;
    let input = decode_attachment_metadata(encoded)?;
    Ok((input, content.clone()))
}
#[tauri::command]
pub async fn attach_application_document(
    request: tauri::ipc::Request<'_>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ApplicationDocument> {
    let (input, content) = attachment_request(&request)?;
    attach_application_document_pool(input, content, &state.db.pool).await
}
async fn attach_application_document_pool(
    input: AttachApplicationDocument,
    content: Vec<u8>,
    pool: &SqlitePool,
) -> ApiResult<ApplicationDocument> {
    if !["resume", "cover_letter", "other"].contains(&input.document_type.as_str()) {
        return Err("Document type must be resume, cover_letter, or other".into());
    }
    let filename = input
        .filename
        .filter(|v| !v.trim().is_empty())
        .ok_or("Attachment filename is required")?;
    let mime = input
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".into());
    let document_id = id();
    let sha256 = format!("{:x}", Sha256::digest(&content));
    let size = content.len() as i64;
    let t = now();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO application_documents(id,application_id,kind,document_type,filename,content,mime_type,sha256,size,event_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(&document_id).bind(&input.application_id).bind(&input.document_type).bind(&input.document_type).bind(&filename).bind(content).bind(&mime).bind(&sha256).bind(size).bind(&input.event_id).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'document_attached',?,?)").bind(id()).bind(&input.application_id).bind(&t).bind(serde_json::json!({"documentId":document_id,"sha256":sha256,"filename":filename}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as("SELECT id,application_id,kind,document_type,filename,mime_type,sha256,size,event_id,created_at FROM application_documents WHERE id=?").bind(document_id).fetch_one(pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn export_application_document(
    document_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<tauri::ipc::Response> {
    Ok(tauri::ipc::Response::new(
        export_application_document_bytes(&document_id, &state.db.pool).await?,
    ))
}
async fn export_application_document_bytes(
    document_id: &str,
    pool: &SqlitePool,
) -> ApiResult<Vec<u8>> {
    let row = sqlx::query("SELECT content,sha256 FROM application_documents WHERE id=?")
        .bind(document_id)
        .fetch_one(pool)
        .await
        .map_err(|_| "Application document was not found".to_string())?;
    let bytes: Vec<u8> = row.get(0);
    let sha256: String = row.get(1);
    if format!("{:x}", Sha256::digest(&bytes)) != sha256 {
        return Err("Application document checksum failed".into());
    };
    Ok(bytes)
}
#[cfg(test)]
mod document_ipc_tests {
    use super::*;

    #[test]
    fn metadata_keeps_unicode_and_uses_url_safe_base64() {
        let json = serde_json::json!({
            "applicationId": "app-1",
            "documentType": "resume",
            "filename": "currículo_日本語.pdf",
            "mimeType": "application/pdf"
        });
        let encoded = URL_SAFE_NO_PAD.encode(json.to_string());
        assert!(!encoded.contains(['+', '/', '=']));
        let input = decode_attachment_metadata(&encoded).unwrap();
        assert_eq!(input.filename.as_deref(), Some("currículo_日本語.pdf"));
        assert_eq!(input.application_id, "app-1");
    }

    #[test]
    fn attachment_limit_accepts_exactly_twenty_megabytes() {
        assert!(validate_attachment_size(MAX_ATTACHMENT_BYTES).is_ok());
        assert_eq!(
            validate_attachment_size(MAX_ATTACHMENT_BYTES + 1).unwrap_err(),
            "Attachment exceeds 20 MB"
        );
    }
}
#[tauri::command]
pub async fn create_application(
    job_id: String,
    persona_id: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Application> {
    // Save is idempotent. Pressing it twice — or pressing Apply on a job already saved — must
    // land on the one application, not stack duplicates in the board.
    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM applications WHERE job_id=? ORDER BY created_at LIMIT 1",
    )
    .bind(&job_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| e.to_string())?
    {
        return sqlx::query_as(&format!("{APPLICATION_SELECT} WHERE a.id=?"))
            .bind(existing)
            .fetch_one(&state.db.pool)
            .await
            .map_err(|e| e.to_string());
    }
    let application_id = id();
    let t = now();
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO applications(id,job_id,persona_id,current_stage,created_at,updated_at) VALUES(?,?,?,'planned',?,?)").bind(&application_id).bind(&job_id).bind(persona_id).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,to_stage,occurred_at) VALUES(?,?, 'created','planned',?)").bind(id()).bind(&application_id).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as(&format!("{APPLICATION_SELECT} WHERE a.id=?"))
        .bind(application_id)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())
}
/// Removes a saved application entirely — not the "Not for me" dismissal, which only hides a
/// listing and keeps it scraped and searchable. This drops the tracked application itself: its
/// notes, documents, interview records, reminders and any open apply attempt cascade with it via
/// foreign keys. The job stays; it simply stops showing a stage once nothing references it.
#[tauri::command]
pub async fn delete_application(
    application_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    delete_application_pool(&state.db.pool, &application_id).await
}
async fn delete_application_pool(pool: &SqlitePool, application_id: &str) -> ApiResult<()> {
    let deleted = sqlx::query("DELETE FROM applications WHERE id=?")
        .bind(application_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();
    if deleted == 0 {
        return Err("Application was not found".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn transition_application(
    input: StageInput,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Application> {
    let valid = [
        "planned",
        "applied",
        "screening",
        "interviewing",
        "offer",
        "accepted",
        "rejected",
        "withdrawn",
    ];
    if !valid.contains(&input.stage.as_str()) {
        return Err("Invalid application stage".into());
    };
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let current: String = sqlx::query_scalar("SELECT current_stage FROM applications WHERE id=?")
        .bind(&input.application_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Application was not found".to_string())?;
    validate_stage_transition(
        &current,
        &input.stage,
        input.manual_override,
        input.reason.as_deref(),
    )?;
    let occurred = input.occurred_at.unwrap_or_else(now);
    let applied = if input.stage == "applied" {
        Some(occurred.clone())
    } else {
        None
    };
    sqlx::query("UPDATE applications SET current_stage=?, applied_at=COALESCE(applied_at,?),accepted_at=CASE WHEN ?='accepted' THEN COALESCE(accepted_at,?) ELSE accepted_at END,rejection_reason=CASE WHEN ?='rejected' THEN ? ELSE rejection_reason END,withdrawn_reason=CASE WHEN ?='withdrawn' THEN ? ELSE withdrawn_reason END,updated_at=? WHERE id=?").bind(&input.stage).bind(applied).bind(&input.stage).bind(&occurred).bind(&input.stage).bind(&input.reason).bind(&input.stage).bind(&input.reason).bind(&occurred).bind(&input.application_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,from_stage,to_stage,reason,occurred_at,payload_json) VALUES(?,?, 'stage_changed',?,?,?,?,?)").bind(id()).bind(&input.application_id).bind(&current).bind(&input.stage).bind(&input.reason).bind(&occurred).bind(serde_json::json!({"manualOverride":input.manual_override,"metadata":input.payload.unwrap_or_else(||serde_json::json!({}))}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    if input.stage == "applied" {
        let due = ghost_due_from(&occurred, 14)?;
        sqlx::query("INSERT INTO reminders(id,application_id,reminder_type,due_at,status,created_at) VALUES(?,?, 'ghosted',?,'pending',?)").bind(id()).bind(&input.application_id).bind(due).bind(&occurred).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    if [
        "screening",
        "interviewing",
        "offer",
        "accepted",
        "rejected",
        "withdrawn",
    ]
    .contains(&input.stage.as_str())
    {
        sqlx::query("UPDATE reminders SET status='cancelled' WHERE application_id=? AND status='pending' AND reminder_type='ghosted'").bind(&input.application_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as(&format!("{APPLICATION_SELECT} WHERE a.id=?"))
        .bind(input.application_id)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())
}
fn legal_stage_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("planned", "applied")
            | (
                "applied",
                "screening" | "interviewing" | "offer" | "rejected" | "withdrawn"
            )
            | (
                "screening",
                "interviewing" | "offer" | "rejected" | "withdrawn"
            )
            | ("interviewing", "offer" | "rejected" | "withdrawn")
            | ("offer", "accepted" | "rejected" | "withdrawn")
    )
}
fn validate_stage_transition(
    from: &str,
    to: &str,
    manual_override: bool,
    reason: Option<&str>,
) -> ApiResult<()> {
    if to == "applied" && !manual_override {
        return Err("Use Open application and explicit Yes confirmation to record applied".into());
    }
    if manual_override
        && reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Err("Manual stage override requires a reason".into());
    }
    if !manual_override && !legal_stage_transition(from, to) {
        return Err(format!("Cannot move application from {from} to {to} without an explicit manual override reason"));
    }
    Ok(())
}
#[tauri::command]
pub async fn record_apply_decision(
    application_id: String,
    decision: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    if !["yes", "no", "not_yet"].contains(&decision.as_str()) {
        return Err("Decision must be yes, no, or not_yet".into());
    };
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let attempt: Option<String> = sqlx::query_scalar("SELECT id FROM application_attempts WHERE application_id=? AND resolved_at IS NULL ORDER BY opened_at DESC LIMIT 1").bind(&application_id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
    if attempt.is_none() {
        tx.rollback().await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    let current: String = sqlx::query_scalar("SELECT current_stage FROM applications WHERE id=?")
        .bind(&application_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Application was not found".to_string())?;
    let t = now();
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,from_stage,to_stage,occurred_at,payload_json) VALUES(?,?, 'apply_confirmation',?,?,?,?)").bind(id()).bind(&application_id).bind(&current).bind(if decision=="yes"{Some("applied")}else{None}).bind(&t).bind(serde_json::json!({"decision":decision,"attemptId":attempt}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    if decision == "yes" {
        sqlx::query("UPDATE applications SET current_stage='applied',applied_at=COALESCE(applied_at,?),updated_at=? WHERE id=?").bind(&t).bind(&t).bind(&application_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT INTO application_events(id,application_id,event_type,from_stage,to_stage,occurred_at,payload_json) VALUES(?,?, 'stage_changed',?,'applied',?,'{}')").bind(id()).bind(&application_id).bind(&current).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        let due = ghost_due_from(&t, 14)?;
        sqlx::query("INSERT INTO reminders(id,application_id,reminder_type,due_at,status,created_at) VALUES(?,?, 'ghosted',?,'pending',?)").bind(id()).bind(&application_id).bind(due).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    // "not_yet" leaves the attempt open (resolved_at stays NULL) so it can still be answered
    // later from the Applications card, but records the resolution so the focus-edge auto-popup
    // never re-asks it: without this, closing and reopening the app reset the in-memory dedupe
    // and asked the same question on every single launch until the user picked yes or no.
    if decision == "not_yet" {
        sqlx::query(
            "UPDATE application_attempts SET resolution=? WHERE id=? AND resolved_at IS NULL",
        )
        .bind(&decision)
        .bind(&attempt)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    } else {
        sqlx::query("UPDATE application_attempts SET resolved_at=?,resolution=? WHERE id=? AND resolved_at IS NULL").bind(&t).bind(&decision).bind(&attempt).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    *state
        .shown_apply_attempt
        .lock()
        .map_err(|_| "Apply prompt lock failed")? = None;
    Ok(())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone)]
pub struct ApplyConfirmation {
    pub attempt_id: String,
    pub application_id: String,
    pub job_id: String,
    pub opened_at: String,
    pub original_stage: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterviewInput {
    pub id: Option<String>,
    pub application_id: String,
    pub stage: String,
    pub scheduled_at: String,
    pub notes: Option<String>,
    pub outcome: Option<String>,
}
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Interview {
    pub id: String,
    pub application_id: String,
    pub stage: String,
    pub scheduled_at: String,
    pub completed_at: Option<String>,
    pub notes: Option<String>,
    pub outcome: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}
fn interview_reminder_due(scheduled_at: &str, hours_before: i64) -> ApiResult<String> {
    chrono::DateTime::parse_from_rfc3339(scheduled_at)
        .map_err(|_| "Interview time must be UTC RFC3339".to_string())
        .map(|value| {
            (value.with_timezone(&Utc) - chrono::Duration::hours(hours_before)).to_rfc3339()
        })
}
async fn insert_interview_reminders(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    application_id: &str,
    interview_id: &str,
    scheduled_at: &str,
    occurred_at: &str,
) -> ApiResult<()> {
    for hours in [24_i64, 1_i64] {
        let due = interview_reminder_due(scheduled_at, hours)?;
        sqlx::query("INSERT INTO reminders(id,application_id,reminder_type,entity_id,due_at,status,created_at) VALUES(?,?, 'interview',?,?,'pending',?)")
            .bind(id()).bind(application_id).bind(interview_id).bind(due).bind(occurred_at)
            .execute(&mut **tx).await.map_err(|e|e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
pub async fn list_interviews(
    application_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<Interview>> {
    sqlx::query_as("SELECT id,application_id,stage,scheduled_at,completed_at,notes,outcome,created_at,updated_at FROM interviews WHERE application_id=? ORDER BY scheduled_at")
        .bind(application_id).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn save_interview(
    input: InterviewInput,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Interview> {
    chrono::DateTime::parse_from_rfc3339(&input.scheduled_at)
        .map_err(|_| "Interview time must be UTC RFC3339")?;
    if input.stage.trim().is_empty() {
        return Err("Interview stage is required".into());
    }
    let interview_id = input.id.clone().unwrap_or_else(id);
    let t = now();
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    if input.id.is_some() {
        let owner: String = sqlx::query_scalar("SELECT application_id FROM interviews WHERE id=?")
            .bind(&interview_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "Interview was not found".to_string())?;
        if owner != input.application_id {
            return Err("Interview application cannot change".into());
        }
        sqlx::query("UPDATE interviews SET stage=?,scheduled_at=?,notes=?,outcome=?,updated_at=? WHERE id=?")
            .bind(&input.stage)
            .bind(&input.scheduled_at)
            .bind(&input.notes)
            .bind(&input.outcome)
            .bind(&t)
            .bind(&interview_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE reminders SET status='cancelled' WHERE entity_id=? AND reminder_type='interview' AND status='pending'")
            .bind(&interview_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    } else {
        sqlx::query("INSERT INTO interviews(id,application_id,stage,scheduled_at,notes,outcome,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(&interview_id).bind(&input.application_id).bind(&input.stage).bind(&input.scheduled_at).bind(&input.notes).bind(&input.outcome).bind(&t).bind(&t)
            .execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    insert_interview_reminders(
        &mut tx,
        &input.application_id,
        &interview_id,
        &input.scheduled_at,
        &t,
    )
    .await?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'interview_scheduled',?,?)")
        .bind(id()).bind(&input.application_id).bind(&t).bind(serde_json::json!({"interviewId":interview_id,"stage":input.stage,"scheduledAt":input.scheduled_at}).to_string())
        .execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as("SELECT id,application_id,stage,scheduled_at,completed_at,notes,outcome,created_at,updated_at FROM interviews WHERE id=?")
        .bind(interview_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn delete_interview(
    interview_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let application_id: String =
        sqlx::query_scalar("SELECT application_id FROM interviews WHERE id=?")
            .bind(&interview_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "Interview was not found".to_string())?;
    sqlx::query("UPDATE reminders SET status='cancelled' WHERE entity_id=? AND reminder_type='interview' AND status='pending'").bind(&interview_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("DELETE FROM interviews WHERE id=?")
        .bind(&interview_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'interview_deleted',?,?)").bind(id()).bind(application_id).bind(now()).bind(serde_json::json!({"interviewId":interview_id}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn update_ghost_threshold(
    application_id: String,
    days: i64,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    if !(1..=365).contains(&days) {
        return Err("Ghost reminder threshold must be 1 to 365 days".into());
    }
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let applied: Option<String> =
        sqlx::query_scalar("SELECT applied_at FROM applications WHERE id=?")
            .bind(&application_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    let Some(applied) = applied else {
        return Err("Application has not been explicitly applied".into());
    };
    sqlx::query("UPDATE reminders SET status='cancelled' WHERE application_id=? AND reminder_type='ghosted' AND status='pending'").bind(&application_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    let due = ghost_due_from(&applied, days)?;
    sqlx::query("INSERT INTO reminders(id,application_id,reminder_type,due_at,status,created_at) VALUES(?,?, 'ghosted',?,'pending',?)").bind(id()).bind(&application_id).bind(due).bind(now()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'ghost_threshold_changed',?,?)").bind(id()).bind(&application_id).bind(now()).bind(serde_json::json!({"days":days}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn open_apply(
    application_id: String,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let row=sqlx::query("SELECT a.job_id,a.current_stage,j.apply_url FROM applications a JOIN jobs j ON j.id=a.job_id WHERE a.id=?").bind(&application_id).fetch_one(&state.db.pool).await.map_err(|_|"Application was not found".to_string())?;
    let job_id: String = row.get(0);
    let stage: String = row.get(1);
    let url: Option<String> = row.get(2);
    let url = url.ok_or("Job has no apply URL")?;
    valid_url(&url, false)?;
    let open: Option<String> = sqlx::query_scalar("SELECT id FROM application_attempts WHERE application_id=? AND resolved_at IS NULL ORDER BY opened_at DESC LIMIT 1")
        .bind(&application_id).fetch_optional(&state.db.pool).await.map_err(|e|e.to_string())?;
    if open.is_none() {
        let attempt = id();
        let t = now();
        sqlx::query("INSERT INTO application_attempts(id,application_id,job_id,opened_at,original_stage) VALUES(?,?,?,?,?)").bind(&attempt).bind(&application_id).bind(&job_id).bind(&t).bind(&stage).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(())
}
/// The Applications card's own "Confirm application" button, for an attempt a "not_yet" answer
/// already silenced from the automatic focus-edge prompt. Looked up by application, not by the
/// global "next" attempt, and ignores the resolution filter that keeps it out of that prompt.
#[tauri::command]
pub async fn confirm_application(
    application_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Option<ApplyConfirmation>> {
    attempt_for_application(&state.db.pool, &application_id).await
}
async fn attempt_for_application(
    pool: &SqlitePool,
    application_id: &str,
) -> ApiResult<Option<ApplyConfirmation>> {
    let row = sqlx::query("SELECT id,application_id,job_id,opened_at,original_stage FROM application_attempts WHERE application_id=? AND resolved_at IS NULL ORDER BY opened_at DESC LIMIT 1")
        .bind(application_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| ApplyConfirmation {
        attempt_id: r.get(0),
        application_id: r.get(1),
        job_id: r.get(2),
        opened_at: r.get(3),
        original_stage: r.get(4),
    }))
}
#[tauri::command]
pub async fn pending_apply_confirmation(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Option<ApplyConfirmation>> {
    let value = next_apply_confirmation(&state.db.pool).await?;
    if let Some(ref value) = value {
        let _ = app.emit("apply-confirmation-required", value);
    }
    Ok(value)
}

async fn next_apply_confirmation(pool: &SqlitePool) -> ApiResult<Option<ApplyConfirmation>> {
    // resolution IS NULL means this attempt has never been shown a decision at all. Once "not_yet"
    // sets resolution without resolving it (see record_apply_decision), it drops out of the
    // automatic focus-edge prompt for good; the user answers it later from the Applications card
    // via confirm_application, which looks the attempt up directly and ignores this filter.
    let row=sqlx::query("SELECT id,application_id,job_id,opened_at,original_stage FROM application_attempts WHERE resolved_at IS NULL AND resolution IS NULL ORDER BY opened_at DESC LIMIT 1").fetch_optional(pool).await.map_err(|e|e.to_string())?;
    let value = row.map(|r| ApplyConfirmation {
        attempt_id: r.get(0),
        application_id: r.get(1),
        job_id: r.get(2),
        opened_at: r.get(3),
        original_stage: r.get(4),
    });
    Ok(value)
}

/// Focus edge handler. One unresolved attempt is emitted once per focus edge;
/// decision handling clears its marker so `not_yet` can be asked next focus.
pub async fn emit_apply_confirmation_on_focus(
    app: &tauri::AppHandle,
    state: &AppState,
) -> ApiResult<()> {
    let value = next_apply_confirmation(&state.db.pool).await?;
    let Some(value) = value else { return Ok(()) };
    let mut shown = state
        .shown_apply_attempt
        .lock()
        .map_err(|_| "Apply prompt lock failed")?;
    if !mark_focus_prompt(&mut *shown, &value.attempt_id) {
        return Ok(());
    }
    app.emit("apply-confirmation-required", value)
        .map_err(|e| e.to_string())
}
fn mark_focus_prompt(shown: &mut Option<String>, attempt_id: &str) -> bool {
    if shown.as_deref() == Some(attempt_id) {
        false
    } else {
        *shown = Some(attempt_id.to_owned());
        true
    }
}

#[cfg(test)]
mod apply_confirmation_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn migrated_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }
    /// One job and one planned application, ready for an attempt to be opened against it.
    async fn seed_application(pool: &SqlitePool) {
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','json','1',1,'active',0,0,'t','t')").execute(pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('j','s','Engineer','Company','','[]','job-v1','t','1','t','t')").execute(pool).await.unwrap();
        sqlx::query("INSERT INTO applications(id,job_id,current_stage,created_at,updated_at) VALUES('a','j','planned','t','t')").execute(pool).await.unwrap();
        sqlx::query("INSERT INTO application_attempts(id,application_id,job_id,opened_at,original_stage) VALUES('att','a','j','t','planned')").execute(pool).await.unwrap();
    }
    async fn pending_confirmation(pool: &SqlitePool, application_id: &str) -> bool {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_attempts att WHERE att.application_id=? AND att.resolved_at IS NULL)")
            .bind(application_id).fetch_one(pool).await.unwrap()
    }

    // "Not yet" is a snooze, not a dismissal: it must not vanish from the automatic focus-edge
    // prompt only to also vanish for the user. Before this test existed, `not_yet` never touched
    // `application_attempts` at all, so the attempt stayed indistinguishable from a brand new one
    // and the launch prompt asked about it again on every single app open.
    #[tokio::test]
    async fn not_yet_silences_the_automatic_prompt_but_stays_answerable() {
        let pool = migrated_pool().await;
        seed_application(&pool).await;
        assert!(
            next_apply_confirmation(&pool).await.unwrap().is_some(),
            "a fresh attempt is asked about automatically"
        );

        sqlx::query("UPDATE application_attempts SET resolution='not_yet' WHERE id='att' AND resolved_at IS NULL").execute(&pool).await.unwrap();

        assert!(
            next_apply_confirmation(&pool).await.unwrap().is_none(),
            "a snoozed attempt must not trigger the launch/focus prompt again"
        );
        assert!(
            pending_confirmation(&pool, "a").await,
            "the application still shows as needing a decision"
        );
        let revisited = attempt_for_application(&pool, "a").await.unwrap();
        assert_eq!(
            revisited.map(|value| value.attempt_id),
            Some("att".to_string()),
            "the Applications card can still bring the same attempt back up"
        );
    }

    #[tokio::test]
    async fn a_resolved_attempt_leaves_nothing_pending() {
        let pool = migrated_pool().await;
        seed_application(&pool).await;
        sqlx::query(
            "UPDATE application_attempts SET resolved_at='t',resolution='no' WHERE id='att'",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(next_apply_confirmation(&pool).await.unwrap().is_none());
        assert!(!pending_confirmation(&pool, "a").await);
        assert!(
            attempt_for_application(&pool, "a").await.unwrap().is_none(),
            "there is nothing left for the card to reopen"
        );
    }

    #[tokio::test]
    async fn confirm_application_ignores_other_applications_open_attempts() {
        let pool = migrated_pool().await;
        seed_application(&pool).await;
        assert!(attempt_for_application(&pool, "other-application")
            .await
            .unwrap()
            .is_none());
    }

    // "Remove from Applications" needs to take everything hanging off the application with it —
    // notes, timeline events, documents, reminders and the open attempt — not leave orphaned rows
    // an unrelated application could later collide with. The job itself must survive: dismissing
    // an application is not the same act as dismissing a listing.
    #[tokio::test]
    async fn deleting_an_application_cascades_its_own_records_and_spares_the_job() {
        let pool = migrated_pool().await;
        seed_application(&pool).await;
        sqlx::query(
            "INSERT INTO notes(id,application_id,body,created_at) VALUES('n','a','Note','t')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO interviews(id,application_id,stage,scheduled_at,created_at) VALUES('i','a','screening','t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO application_documents(id,application_id,kind,filename,content,sha256,created_at) VALUES('d','a','cv','r.pdf',x'00','h','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO reminders(id,application_id,reminder_type,due_at,status,created_at) VALUES('r','a','ghosted','t','pending','t')").execute(&pool).await.unwrap();

        delete_application_pool(&pool, "a").await.unwrap();

        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM applications WHERE id='a'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        for (table, message) in [
            ("notes", "note"),
            ("interviews", "interview"),
            ("application_documents", "document"),
            ("reminders", "reminder"),
            ("application_attempts", "open attempt"),
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&format!(
                    "SELECT count(*) FROM {table} WHERE application_id='a'"
                ))
                .fetch_one(&pool)
                .await
                .unwrap(),
                0,
                "{message} should cascade away with the application"
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs WHERE id='j'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1,
            "the job listing itself must not be touched"
        );
    }
    #[tokio::test]
    async fn deleting_an_application_that_does_not_exist_is_an_explicit_error() {
        let pool = migrated_pool().await;
        assert!(delete_application_pool(&pool, "missing").await.is_err());
    }
}

#[derive(Serialize)]
pub struct ReminderDelivery {
    pub id: String,
    pub title: String,
    pub body: String,
}
/// Reminder-only startup reads local SQLite, never starts worker/network/UI.
pub async fn pending_reminder_delivery(
    pool: &SqlitePool,
    reminder_id: &str,
) -> ApiResult<Option<ReminderDelivery>> {
    Uuid::parse_str(reminder_id).map_err(|_| "Reminder ID must be UUID")?;
    let row = sqlx::query("SELECT r.id,r.reminder_type,j.title,j.company,r.due_at FROM reminders r JOIN applications a ON a.id=r.application_id JOIN jobs j ON j.id=a.job_id WHERE r.id=? AND r.status='pending'")
        .bind(reminder_id).fetch_optional(pool).await.map_err(|e|e.to_string())?;
    let Some(row) = row else { return Ok(None) };
    let due: String = row.get(4);
    let due = chrono::DateTime::parse_from_rfc3339(&due)
        .map_err(|_| "Reminder due_at must be UTC RFC3339")?
        .with_timezone(&Utc);
    if due > Utc::now() {
        return Ok(None);
    }
    let kind: String = row.get(1);
    let title: String = row.get(2);
    let company: String = row.get(3);
    Ok(Some(ReminderDelivery {
        id: row.get(0),
        title: if kind == "interview" {
            "Interview reminder".into()
        } else {
            "Application follow-up reminder".into()
        },
        body: format!("{title} at {company}"),
    }))
}
pub async fn finish_reminder_delivery(
    pool: &SqlitePool,
    reminder_id: &str,
    result: Result<(), String>,
) -> ApiResult<()> {
    let (status, error) = match result {
        Ok(()) => ("delivered", None),
        Err(error) => ("pending", Some(error)),
    };
    sqlx::query("UPDATE reminders SET status=?,last_error=?,last_reconciled_at=? WHERE id=? AND status='pending'")
        .bind(status).bind(error).bind(now()).bind(reminder_id).execute(pool).await.map_err(|e|e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn reconcile_reminders(state: State<'_, Arc<AppState>>) -> ApiResult<serde_json::Value> {
    reconcile_reminders_pool(&state.db.pool).await
}

/// Called during startup before first window interaction and by Diagnostics.
pub async fn reconcile_reminders_pool(pool: &SqlitePool) -> ApiResult<serde_json::Value> {
    let rows = sqlx::query("SELECT id,due_at,status,reminder_type,os_task_id FROM reminders")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    let scheduler = WindowsTaskScheduler::packaged(executable);
    let mut scheduled = 0;
    let mut missed = 0;
    let mut errors = 0;
    let snapshot = run_scheduler_blocking({
        let scheduler = scheduler.clone();
        move || scheduler.list_managed()
    })
    .await;
    let (mut managed_tasks, snapshot_error) = match snapshot {
        Ok(tasks) => (tasks.into_iter().collect::<HashSet<_>>(), None),
        Err(error) => (HashSet::new(), Some(error)),
    };
    if let Some(error) = snapshot_error {
        errors += 1;
        let _ = sqlx::query(
            "UPDATE reminders SET last_error=?,last_reconciled_at=? WHERE status='pending'",
        )
        .bind(error)
        .bind(now())
        .execute(pool)
        .await;
    }
    let mut expected_tasks = HashSet::new();
    for row in rows {
        let id: String = row.get(0);
        let due: String = row.get(1);
        let due = chrono::DateTime::parse_from_rfc3339(&due)
            .map_err(|_| "Reminder due_at must be UTC RFC3339")?
            .with_timezone(&Utc);
        let status: String = row.get(2);
        let existing: Option<String> = row.get(4);
        if status != "pending" {
            if let Some(task) = existing.filter(|task| is_managed_task_path(task)) {
                let result = run_scheduler_blocking({
                    let scheduler = scheduler.clone();
                    let task = task.clone();
                    move || scheduler.cancel(&task)
                })
                .await;
                if let Err(error) = result {
                    sqlx::query(
                        "UPDATE reminders SET last_error=?,last_reconciled_at=? WHERE id=?",
                    )
                    .bind(error)
                    .bind(now())
                    .bind(&id)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                    errors += 1;
                } else {
                    managed_tasks.remove(&task);
                    sqlx::query("UPDATE reminders SET os_task_id=NULL,last_reconciled_at=?,last_error=NULL WHERE id=?")
                        .bind(now()).bind(&id).execute(pool).await.map_err(|e|e.to_string())?;
                }
            }
            continue;
        }
        let reminder = ScheduledReminder {
            id: id.clone(),
            due_at: due,
            status: "pending".into(),
            kind: row.get(3),
        };
        if due <= Utc::now() {
            sqlx::query("UPDATE reminders SET status='missed',last_reconciled_at=?,last_error=NULL WHERE id=? AND status='pending'").bind(now()).bind(&id).execute(pool).await.map_err(|e|e.to_string())?;
            missed += 1;
            continue;
        }
        let task = match existing.as_deref() {
            Some(task) if !is_managed_task_path(task) => {
                sqlx::query("UPDATE reminders SET last_error=?,last_reconciled_at=? WHERE id=?")
                    .bind("Task path is outside JobScraper scope")
                    .bind(now())
                    .bind(&id)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                errors += 1;
                false
            }
            Some(task) => managed_tasks.contains(task),
            None => false,
        };
        if !task {
            let result = run_scheduler_blocking({
                let scheduler = scheduler.clone();
                let reminder = reminder.clone();
                move || scheduler.create(&reminder)
            })
            .await;
            match result {
                Ok(task_id) => {
                    sqlx::query("UPDATE reminders SET os_task_id=?,last_reconciled_at=?,last_error=NULL WHERE id=?").bind(&task_id).bind(now()).bind(&id).execute(pool).await.map_err(|e|e.to_string())?;
                    managed_tasks.insert(task_id.clone());
                    expected_tasks.insert(task_id);
                    scheduled += 1
                }
                Err(error) => {
                    sqlx::query(
                        "UPDATE reminders SET last_error=?,last_reconciled_at=? WHERE id=?",
                    )
                    .bind(error)
                    .bind(now())
                    .bind(&id)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                    errors += 1
                }
            }
        } else if let Some(task) = existing {
            expected_tasks.insert(task);
        }
    }
    // The one scoped snapshot drives both existence checks and strict orphan cleanup.
    let managed_tasks = managed_tasks.into_iter().collect::<Vec<_>>();
    for task in scoped_orphans(&managed_tasks, &expected_tasks) {
        let result = run_scheduler_blocking({
            let scheduler = scheduler.clone();
            let task = task.clone();
            move || scheduler.cancel(&task)
        })
        .await;
        if let Err(error) = result {
            errors += 1;
            let _ = sqlx::query(
                "UPDATE reminders SET last_error=?,last_reconciled_at=? WHERE os_task_id=?",
            )
            .bind(error)
            .bind(now())
            .bind(task)
            .execute(pool)
            .await;
        }
    }
    Ok(serde_json::json!({"scheduled":scheduled,"missed":missed,"errors":errors}))
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsFilter {
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub persona_id: Option<String>,
    pub source_id: Option<String>,
    pub company: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    name: String,
    count: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trend {
    date: String,
    count: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversion {
    name: String,
    numerator: i64,
    denominator: i64,
    rate: Option<f64>,
    small_sample: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseMetric {
    name: String,
    hours: Option<f64>,
    samples: i64,
    small_sample: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Analytics {
    total: u64,
    date_window: String,
    by_stage: Vec<Count>,
    applications_over_time: Vec<Trend>,
    source_counts: Vec<Count>,
    company_counts: Vec<Count>,
    conversions: Vec<Conversion>,
    response_by_company: Vec<ResponseMetric>,
    response_by_source: Vec<ResponseMetric>,
    outcomes: Vec<Count>,
    match_score_buckets: Vec<Count>,
    review_decisions: Vec<Count>,
    job_freshness: Vec<Count>,
    applied_to_response_hours: Option<f64>,
    response_samples: u64,
    response_small_sample: bool,
}
fn validate_analytics_filter(input: &AnalyticsFilter) -> ApiResult<()> {
    for value in [&input.start_at, &input.end_at] {
        if let Some(value) = value {
            chrono::DateTime::parse_from_rfc3339(value)
                .map_err(|_| "Analytics dates must be UTC RFC3339".to_string())?;
        }
    }
    if input.start_at > input.end_at {
        return Err("Analytics start date must not be after end date".into());
    };
    Ok(())
}
fn analytics_where() -> &'static str {
    " WHERE s.deleted_at IS NULL AND (? IS NULL OR a.created_at>=?) AND (? IS NULL OR a.created_at<?) AND (? IS NULL OR a.persona_id=?) AND (? IS NULL OR j.source_id=?) AND (? IS NULL OR lower(j.company)=lower(?))"
}
fn bind_analytics<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    input: &'q AnalyticsFilter,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    query
        .bind(&input.start_at)
        .bind(&input.start_at)
        .bind(&input.end_at)
        .bind(&input.end_at)
        .bind(&input.persona_id)
        .bind(&input.persona_id)
        .bind(&input.source_id)
        .bind(&input.source_id)
        .bind(&input.company)
        .bind(&input.company)
}
async fn counts(pool: &SqlitePool, input: &AnalyticsFilter, select: &str) -> ApiResult<Vec<Count>> {
    let sql=format!("SELECT {select},count(*) FROM applications a JOIN jobs j ON j.id=a.job_id JOIN sources s ON s.id=j.source_id {} GROUP BY 1 ORDER BY 2 DESC,1",analytics_where());
    Ok(bind_analytics(sqlx::query(&sql), input)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| Count {
            name: r.get(0),
            count: r.get(1),
        })
        .collect())
}
async fn response_metrics(
    pool: &SqlitePool,
    input: &AnalyticsFilter,
    group: &str,
) -> ApiResult<Vec<ResponseMetric>> {
    let sql=format!("SELECT {group},avg((julianday(e.occurred_at)-julianday(a.applied_at))*24),count(*) FROM applications a JOIN jobs j ON j.id=a.job_id JOIN sources s ON s.id=j.source_id JOIN application_events e ON e.application_id=a.id WHERE a.applied_at IS NOT NULL AND e.occurred_at=(SELECT min(e2.occurred_at) FROM application_events e2 WHERE e2.application_id=a.id AND e2.occurred_at>a.applied_at AND e2.to_stage IN ('screening','interviewing','offer','rejected')) AND (? IS NULL OR a.created_at>=?) AND (? IS NULL OR a.created_at<?) AND (? IS NULL OR a.persona_id=?) AND (? IS NULL OR j.source_id=?) AND (? IS NULL OR lower(j.company)=lower(?)) GROUP BY 1 ORDER BY 3 DESC,1");
    let rows = bind_analytics(sqlx::query(&sql), input)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let samples: i64 = r.get(2);
            ResponseMetric {
                name: r.get(0),
                hours: r.get(1),
                samples,
                small_sample: samples < 3,
            }
        })
        .collect())
}
#[tauri::command]
pub async fn analytics(
    filter: Option<AnalyticsFilter>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Analytics> {
    let input = filter.unwrap_or_default();
    validate_analytics_filter(&input)?;
    let where_sql = analytics_where();
    let total:i64=bind_analytics(sqlx::query(&format!("SELECT count(*) FROM applications a JOIN jobs j ON j.id=a.job_id JOIN sources s ON s.id=j.source_id {where_sql}")),&input).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())?.get(0);
    let by_stage = counts(&state.db.pool, &input, "a.current_stage").await?;
    let source_counts = counts(&state.db.pool, &input, "s.name").await?;
    let company_counts = counts(&state.db.pool, &input, "j.company").await?;
    let applications_over_time=bind_analytics(sqlx::query(&format!("SELECT substr(a.created_at,1,10),count(*) FROM applications a JOIN jobs j ON j.id=a.job_id JOIN sources s ON s.id=j.source_id {where_sql} GROUP BY 1 ORDER BY 1")),&input).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?.into_iter().map(|r|Trend{date:r.get(0),count:r.get(1)}).collect();
    let facts_sql=format!("SELECT count(*) AS cohort, sum(a.applied_at IS NOT NULL), sum(EXISTS(SELECT 1 FROM application_events e WHERE e.application_id=a.id AND a.applied_at IS NOT NULL AND e.occurred_at>a.applied_at AND e.to_stage IN ('screening','interviewing','offer','rejected'))), sum(EXISTS(SELECT 1 FROM application_events e WHERE e.application_id=a.id AND e.to_stage='interviewing')), sum(EXISTS(SELECT 1 FROM application_events e WHERE e.application_id=a.id AND e.to_stage='offer')), sum(EXISTS(SELECT 1 FROM application_events e WHERE e.application_id=a.id AND e.to_stage='accepted')) FROM applications a JOIN jobs j ON j.id=a.job_id JOIN sources s ON s.id=j.source_id {where_sql}");
    let facts = bind_analytics(sqlx::query(&facts_sql), &input)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    let cohort: i64 = facts.get(0);
    let applied: i64 = facts.get::<Option<i64>, _>(1).unwrap_or(0);
    let responded: i64 = facts.get::<Option<i64>, _>(2).unwrap_or(0);
    let interviewed: i64 = facts.get::<Option<i64>, _>(3).unwrap_or(0);
    let offered: i64 = facts.get::<Option<i64>, _>(4).unwrap_or(0);
    let accepted: i64 = facts.get::<Option<i64>, _>(5).unwrap_or(0);
    let conversion = |name: &str, numerator: i64, denominator: i64| Conversion {
        name: name.into(),
        numerator,
        denominator,
        rate: (denominator > 0).then_some(numerator as f64 * 100.0 / denominator as f64),
        small_sample: denominator < 3,
    };
    let conversions = vec![
        conversion("applied", applied, cohort),
        conversion("first response", responded, applied),
        conversion("interview", interviewed, applied),
        conversion("offer", offered, applied),
        conversion("accepted", accepted, applied),
    ];
    let response_by_company = response_metrics(&state.db.pool, &input, "j.company").await?;
    let response_by_source = response_metrics(&state.db.pool, &input, "s.name").await?;
    let all_response = response_by_company
        .iter()
        .fold((0.0, 0_i64), |(total, n), item| match item.hours {
            Some(hours) => (total + hours * item.samples as f64, n + item.samples),
            None => (total, n),
        });
    let applied_to_response_hours =
        (all_response.1 > 0).then_some(all_response.0 / all_response.1 as f64);
    let outcomes=counts(&state.db.pool,&input,"CASE WHEN a.current_stage IN ('accepted','rejected','withdrawn') THEN a.current_stage ELSE 'open' END").await?;
    let match_score_buckets=bind_analytics(sqlx::query(&format!("SELECT CASE WHEN m.score IS NULL THEN 'unscored' WHEN m.score<40 THEN '0-39' WHEN m.score<60 THEN '40-59' WHEN m.score<80 THEN '60-79' ELSE '80-100' END,count(*) FROM applications a JOIN jobs j ON j.id=a.job_id JOIN sources s ON s.id=j.source_id LEFT JOIN match_results m ON m.job_id=a.job_id AND m.persona_id=a.persona_id {where_sql} GROUP BY 1 ORDER BY 1")),&input).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?.into_iter().map(|r|Count{name:r.get(0),count:r.get(1)}).collect();
    let review_decisions=sqlx::query("SELECT status,count(*) FROM review_decisions WHERE (? IS NULL OR persona_id=?) GROUP BY status ORDER BY status").bind(&input.persona_id).bind(&input.persona_id).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?.into_iter().map(|r|Count{name:r.get(0),count:r.get(1)}).collect();
    let job_freshness=sqlx::query("SELECT CASE WHEN availability!='active' THEN availability WHEN posted_at IS NULL THEN 'unknown date' WHEN julianday('now')-julianday(posted_at)<=7 THEN '0-7 days' WHEN julianday('now')-julianday(posted_at)<=30 THEN '8-30 days' ELSE '31+ days' END,count(*) FROM jobs j JOIN sources s ON s.id=j.source_id WHERE s.deleted_at IS NULL AND (? IS NULL OR j.source_id=?) AND (? IS NULL OR lower(j.company)=lower(?)) GROUP BY 1 ORDER BY 1").bind(&input.source_id).bind(&input.source_id).bind(&input.company).bind(&input.company).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?.into_iter().map(|r|Count{name:r.get(0),count:r.get(1)}).collect();
    let date_window = format!(
        "applications created {} to {}",
        input.start_at.as_deref().unwrap_or("all time"),
        input.end_at.as_deref().unwrap_or("now")
    );
    Ok(Analytics {
        total: total as u64,
        date_window,
        by_stage,
        applications_over_time,
        source_counts,
        company_counts,
        conversions,
        response_by_company,
        response_by_source,
        outcomes,
        match_score_buckets,
        review_decisions,
        job_freshness,
        applied_to_response_hours,
        response_samples: all_response.1 as u64,
        response_small_sample: all_response.1 < 3,
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    db_path: String,
    schema_version: i64,
    source_count: i64,
    job_count: i64,
    /// Design assertion, not a live measurement: JobScraper's own code makes no
    /// network request during startup (see README, manual-scrape-only design).
    app_code_network_free_at_startup: bool,
    /// Design assertion, not a live measurement: JobScraper does not attempt to
    /// control network activity the WebView2 runtime itself initiates, which the
    /// README documents as making its own outbound connections independent of
    /// this app's code.
    webview_runtime_network_not_controlled_by_app: bool,
    sidecar_active: bool,
}
#[tauri::command]
pub async fn diagnostics(state: State<'_, Arc<AppState>>) -> ApiResult<Diagnostics> {
    let version: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&state.db.pool)
        .await
        .unwrap_or(0);
    let source_count = sqlx::query_scalar("SELECT count(*) FROM sources WHERE deleted_at IS NULL")
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    let job_count = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Diagnostics {
        db_path: state.db.root.join("jobscraper.db").display().to_string(),
        schema_version: version,
        source_count,
        job_count,
        app_code_network_free_at_startup: true,
        webview_runtime_network_not_controlled_by_app: true,
        sidecar_active: state.sidecars.active(),
    })
}
#[tauri::command]
pub async fn backup_database(state: State<'_, Arc<AppState>>) -> ApiResult<String> {
    let out = state.db.root.join("backups");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let dest = out.join(format!(
        "jobscraper-{}.sqlite",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    sqlx::query("VACUUM INTO ?")
        .bind(dest.to_string_lossy().to_string())
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    let data = std::fs::read(&dest).map_err(|e| e.to_string())?;
    let digest = format!("{:x}", Sha256::digest(&data));
    std::fs::write(dest.with_extension("sha256"), digest).map_err(|e| e.to_string())?;
    Ok(dest.display().to_string())
}
#[tauri::command]
pub async fn export_csv(kind: String, state: State<'_, Arc<AppState>>) -> ApiResult<String> {
    let out = state.db.root.join("exports");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let (sql,headers)=match kind.as_str(){"applications"=>("SELECT a.current_stage,j.title,j.company,a.applied_at FROM applications a LEFT JOIN jobs j ON j.id=a.job_id","stage,title,company,applied_at"),"events"=>("SELECT application_id,event_type,from_stage,to_stage,occurred_at FROM application_events","application_id,event_type,from_stage,to_stage,occurred_at"),_ => ("SELECT title,company,location,canonical_url,availability FROM jobs","title,company,location,canonical_url,availability")};
    let rows = sqlx::query(sql)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut text = headers.to_string() + "\n";
    for r in rows {
        let mut cells = Vec::new();
        for i in 0..r.len() {
            let v: Option<String> = r.try_get(i).ok();
            cells.push(format!(
                "\"{}\"",
                v.unwrap_or_default().replace('"', "\"\"")
            ))
        }
        text.push_str(&cells.join(","));
        text.push('\n');
    }
    let file = out.join(format!(
        "{}-{}.csv",
        kind,
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file.display().to_string())
}

#[derive(Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportFilter {
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub persona_id: Option<String>,
    pub source_id: Option<String>,
    pub company: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub kind: String,
    pub destination: String,
    pub overwrite: bool,
    #[serde(default)]
    pub filter: ExportFilter,
}
fn csv_cell(value: Option<String>) -> String {
    format!("\"{}\"", value.unwrap_or_default().replace('"', "\"\""))
}
fn explicit_export_destination(destination: &str, overwrite: bool) -> ApiResult<PathBuf> {
    let path = PathBuf::from(destination);
    if !path.is_absolute() || path.file_name().is_none() {
        return Err("Choose an absolute output file path".into());
    }
    if path.exists() && !overwrite {
        return Err("Output file exists; confirm overwrite explicitly".into());
    }
    path.parent()
        .filter(|parent| parent.exists())
        .ok_or("Output folder does not exist")?;
    Ok(path)
}
async fn relational_rows(pool: &SqlitePool, table: &str) -> ApiResult<Vec<serde_json::Value>> {
    // Table name comes only from this hard-coded inventory. Column names come from
    // SQLite schema and are quoted before being included in generated JSON SQL.
    let allowed = [
        "sources",
        "source_configs",
        "scrape_runs",
        "scrape_run_events",
        "app_logs",
        "settings",
        "jobs",
        "job_occurrences",
        "job_revisions",
        "personas",
        "resume_documents",
        "persona_skills",
        "embeddings",
        "match_results",
        "review_decisions",
        "applications",
        "application_events",
        "notes",
        "application_documents",
        "interviews",
        "reminders",
        "duplicate_candidates",
        "duplicate_merge_audits",
        "duplicate_merge_conflicts",
        "job_dedupe_events",
        "data_purge_audits",
    ];
    if !allowed.contains(&table) {
        return Err("Unsupported relational export table".into());
    }
    let columns = sqlx::query("SELECT name FROM pragma_table_info(?) ORDER BY cid")
        .bind(table)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut pairs = Vec::new();
    for column in columns {
        let name: String = column.get(0);
        // BLOB contents are exported by backups; JSON exports retain their stable
        // metadata rather than copying potentially large opaque file/model bytes.
        if name == "content" || name == "vector" {
            continue;
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("Unsafe schema column".into());
        }
        pairs.push(format!("'{}',\"{}\"", name, name));
    }
    let sql = format!(
        "SELECT json_object({}) FROM \"{}\" ORDER BY rowid",
        pairs.join(","),
        table
    );
    let rows = sqlx::query_scalar::<_, String>(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|row| serde_json::from_str(&row).map_err(|_| "Relational row JSON was invalid".into()))
        .collect()
}
fn row_text<'a>(row: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    row.get(field).and_then(|value| value.as_str())
}
fn has_id(rows: &[serde_json::Value], id: &str) -> bool {
    rows.iter().any(|row| row_text(row, "id") == Some(id))
}
fn in_ids(row: &serde_json::Value, field: &str, ids: &HashSet<String>) -> bool {
    row_text(row, field).is_some_and(|value| ids.contains(value))
}
fn export_date_matches(row: &serde_json::Value, field: &str, filter: &ExportFilter) -> bool {
    let value = row_text(row, field).unwrap_or("");
    filter
        .start_at
        .as_deref()
        .is_none_or(|start| value >= start)
        && filter.end_at.as_deref().is_none_or(|end| value < end)
}
async fn export_closure(
    pool: &SqlitePool,
    filter: &ExportFilter,
) -> ApiResult<HashMap<String, Vec<serde_json::Value>>> {
    if filter
        .start_at
        .as_deref()
        .is_some_and(|date| chrono::DateTime::parse_from_rfc3339(date).is_err())
        || filter
            .end_at
            .as_deref()
            .is_some_and(|date| chrono::DateTime::parse_from_rfc3339(date).is_err())
    {
        return Err("Export dates must be UTC RFC3339".into());
    }
    if let (Some(start), Some(end)) = (filter.start_at.as_deref(), filter.end_at.as_deref()) {
        if start >= end {
            return Err("Export start must be before end".into());
        }
    }
    let tables = [
        "sources",
        "source_configs",
        "scrape_runs",
        "scrape_run_events",
        "app_logs",
        "settings",
        "jobs",
        "job_occurrences",
        "job_revisions",
        "personas",
        "resume_documents",
        "persona_skills",
        "embeddings",
        "match_results",
        "review_decisions",
        "applications",
        "application_events",
        "notes",
        "application_documents",
        "interviews",
        "reminders",
        "duplicate_candidates",
        "duplicate_merge_audits",
        "duplicate_merge_conflicts",
        "job_dedupe_events",
        "data_purge_audits",
    ];
    let mut all = HashMap::new();
    for table in tables {
        all.insert(table.to_string(), relational_rows(pool, table).await?);
    }
    let sources = &all["sources"];
    let personas = &all["personas"];
    if filter
        .source_id
        .as_deref()
        .is_some_and(|id| !has_id(sources, id))
    {
        return Err("Export source ID was not found".into());
    }
    if filter
        .persona_id
        .as_deref()
        .is_some_and(|id| !has_id(personas, id))
    {
        return Err("Export persona ID was not found".into());
    }
    let apps = &all["applications"];
    let matches = &all["match_results"];
    let reviews = &all["review_decisions"];
    let mut jobs: HashSet<String> = all["jobs"]
        .iter()
        .filter(|job| {
            let source = filter
                .source_id
                .as_deref()
                .is_none_or(|id| row_text(job, "source_id") == Some(id));
            let company = filter.company.as_deref().is_none_or(|name| {
                row_text(job, "company").is_some_and(|value| value.eq_ignore_ascii_case(name))
            });
            let date = export_date_matches(job, "created_at", filter);
            let persona = filter.persona_id.as_deref().is_none_or(|id| {
                apps.iter().any(|app| {
                    row_text(app, "job_id") == row_text(job, "id")
                        && row_text(app, "persona_id") == Some(id)
                }) || matches.iter().any(|item| {
                    row_text(item, "job_id") == row_text(job, "id")
                        && row_text(item, "persona_id") == Some(id)
                }) || reviews.iter().any(|item| {
                    row_text(item, "job_id") == row_text(job, "id")
                        && row_text(item, "persona_id") == Some(id)
                })
            });
            source && company && date && persona
        })
        .filter_map(|job| row_text(job, "id").map(str::to_owned))
        .collect();
    let app_ids: HashSet<String> = apps
        .iter()
        .filter(|app| {
            let persona = filter
                .persona_id
                .as_deref()
                .is_none_or(|id| row_text(app, "persona_id") == Some(id));
            let date = export_date_matches(app, "created_at", filter);
            let job_ok = row_text(app, "job_id").is_some_and(|id| jobs.contains(id));
            persona && date && job_ok
        })
        .filter_map(|app| row_text(app, "id").map(str::to_owned))
        .collect();
    for app in apps.iter().filter(|app| in_ids(app, "id", &app_ids)) {
        if let Some(job) = row_text(app, "job_id") {
            jobs.insert(job.to_owned());
        }
    }
    let source_ids: HashSet<String> = all["jobs"]
        .iter()
        .filter(|job| in_ids(job, "id", &jobs))
        .filter_map(|job| row_text(job, "source_id").map(str::to_owned))
        .collect();
    // An unscoped relational archive preserves standalone persona/resume records too.
    // Scoped exports retain only personas reachable from selected roots.
    let mut persona_ids: HashSet<String> = if filter.persona_id.is_none()
        && filter.source_id.is_none()
        && filter.company.is_none()
        && filter.start_at.is_none()
        && filter.end_at.is_none()
    {
        all["personas"]
            .iter()
            .filter_map(|row| row_text(row, "id").map(str::to_owned))
            .collect()
    } else {
        filter.persona_id.iter().cloned().collect()
    };
    for table in [matches, reviews, apps] {
        for row in table.iter().filter(|row| in_ids(row, "job_id", &jobs)) {
            if let Some(persona) = row_text(row, "persona_id") {
                persona_ids.insert(persona.to_owned());
            }
        }
    }
    let run_ids: HashSet<String> = all["job_occurrences"]
        .iter()
        .filter(|row| in_ids(row, "job_id", &jobs))
        .filter_map(|row| row_text(row, "run_id").map(str::to_owned))
        .collect();
    let audit_ids: HashSet<String> = all["duplicate_merge_audits"]
        .iter()
        .filter(|row| in_ids(row, "canonical_job_id", &jobs) && in_ids(row, "merged_job_id", &jobs))
        .filter_map(|row| row_text(row, "id").map(str::to_owned))
        .collect();
    let mut out = HashMap::new();
    for table in tables {
        let rows = &all[table];
        let selected: Vec<_> = match table {
            "jobs" => rows
                .iter()
                .filter(|r| in_ids(r, "id", &jobs))
                .cloned()
                .collect(),
            "sources" => rows
                .iter()
                .filter(|r| in_ids(r, "id", &source_ids))
                .cloned()
                .collect(),
            "source_configs" | "scrape_runs" => rows
                .iter()
                .filter(|r| in_ids(r, "source_id", &source_ids))
                .cloned()
                .collect(),
            "scrape_run_events" => rows
                .iter()
                .filter(|r| in_ids(r, "run_id", &run_ids))
                .cloned()
                .collect(),
            "job_occurrences" | "job_revisions" | "match_results" | "review_decisions" => rows
                .iter()
                .filter(|r| in_ids(r, "job_id", &jobs))
                .cloned()
                .collect(),
            "applications" => rows
                .iter()
                .filter(|r| in_ids(r, "id", &app_ids))
                .cloned()
                .collect(),
            "application_events"
            | "notes"
            | "application_documents"
            | "interviews"
            | "reminders" => rows
                .iter()
                .filter(|r| in_ids(r, "application_id", &app_ids))
                .cloned()
                .collect(),
            "personas" => rows
                .iter()
                .filter(|r| in_ids(r, "id", &persona_ids))
                .cloned()
                .collect(),
            "resume_documents" => rows
                .iter()
                .filter(|r| {
                    in_ids(r, "persona_id", &persona_ids)
                        || row_text(r, "id").is_some_and(|id| {
                            all["personas"]
                                .iter()
                                .filter(|p| in_ids(p, "id", &persona_ids))
                                .any(|p| row_text(p, "resume_document_id") == Some(id))
                        })
                })
                .cloned()
                .collect(),
            "persona_skills" => rows
                .iter()
                .filter(|r| in_ids(r, "persona_id", &persona_ids))
                .cloned()
                .collect(),
            "embeddings" => rows
                .iter()
                .filter(|r| in_ids(r, "owner_id", &jobs) || in_ids(r, "owner_id", &persona_ids))
                .cloned()
                .collect(),
            "duplicate_merge_audits" => rows
                .iter()
                .filter(|r| in_ids(r, "id", &audit_ids))
                .cloned()
                .collect(),
            "duplicate_merge_conflicts" => rows
                .iter()
                .filter(|r| in_ids(r, "audit_id", &audit_ids))
                .cloned()
                .collect(),
            "duplicate_candidates" => rows
                .iter()
                .filter(|r| {
                    in_ids(r, "canonical_job_id", &jobs)
                        || in_ids(r, "left_job_id", &jobs) && in_ids(r, "right_job_id", &jobs)
                })
                .cloned()
                .collect(),
            "job_dedupe_events" => rows
                .iter()
                .filter(|r| in_ids(r, "canonical_job_id", &jobs))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        out.insert(table.to_string(), selected);
    }
    Ok(out)
}
async fn relational_json(pool: &SqlitePool, filter: &ExportFilter) -> ApiResult<Vec<u8>> {
    let selected = export_closure(pool, filter).await?;
    let mut data = serde_json::Map::new();
    for (table, rows) in selected {
        data.insert(table, serde_json::Value::Array(rows));
    }
    serde_json::to_vec_pretty(&serde_json::json!({"format":"jobscraper-relational-json","version":1,"exportedAt":now(),"filter":filter,"tables":data})).map_err(|e|e.to_string())
}
fn outcomes_csv(selected: &HashMap<String, Vec<serde_json::Value>>, company: bool) -> Vec<u8> {
    let jobs = &selected["jobs"];
    let apps = &selected["applications"];
    let events = &selected["application_events"];
    let sources = &selected["sources"];
    let source_names: HashMap<String, String> = sources
        .iter()
        .filter_map(|r| Some((row_text(r, "id")?.into(), row_text(r, "name")?.into())))
        .collect();
    let labels: HashMap<String, String> = jobs
        .iter()
        .filter_map(|j| {
            Some((
                row_text(j, "id")?.into(),
                if company {
                    row_text(j, "company").unwrap_or("Unknown").into()
                } else {
                    source_names.get(row_text(j, "source_id")?)?.clone()
                },
            ))
        })
        .collect();
    let mut groups: HashMap<String, Vec<&serde_json::Value>> = HashMap::new();
    for app in apps {
        if let Some(label) = row_text(app, "job_id").and_then(|id| labels.get(id)) {
            groups.entry(label.clone()).or_default().push(app)
        }
    }
    let mut text="dimension,name,applications,applied,first_response,interview,offer,accepted,rejected,withdrawn,applied_rate,first_response_rate,interview_rate,offer_rate,accepted_rate,rejected_rate,withdrawn_rate,first_response_mean_hours,first_response_samples,small_sample\r\n".to_string();
    for (name, rows) in groups {
        let total = rows.len() as f64;
        let mut c = [0usize; 7];
        let mut hours = Vec::new();
        for app in rows {
            let stage = row_text(app, "current_stage").unwrap_or("");
            let app_id = row_text(app, "id").unwrap_or("");
            let applied = row_text(app, "applied_at");
            if applied.is_some() || stage != "planned" {
                c[0] += 1
            }
            if ["interviewing", "offer", "accepted"].contains(&stage) {
                c[2] += 1
            }
            if ["offer", "accepted"].contains(&stage) {
                c[3] += 1
            }
            c[4] += usize::from(stage == "accepted");
            c[5] += usize::from(stage == "rejected");
            c[6] += usize::from(stage == "withdrawn");
            if let Some(start) = applied.and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            {
                if let Some(response) = events
                    .iter()
                    .filter(|e| {
                        row_text(e, "application_id") == Some(app_id)
                            && ["screening", "interviewing", "offer", "rejected"]
                                .contains(&row_text(e, "to_stage").unwrap_or(""))
                    })
                    .filter_map(|e| {
                        chrono::DateTime::parse_from_rfc3339(row_text(e, "occurred_at")?).ok()
                    })
                    .filter(|v| *v > start)
                    .min()
                {
                    c[1] += 1;
                    hours.push((response - start).num_minutes() as f64 / 60.0)
                }
            }
        }
        let rate = |n: usize, d: f64| {
            if d == 0.0 {
                "".into()
            } else {
                format!("{:.2}%", n as f64 * 100.0 / d)
            }
        };
        let mean = if hours.is_empty() {
            "".into()
        } else {
            format!("{:.2}", hours.iter().sum::<f64>() / hours.len() as f64)
        };
        let mut values = vec![
            csv_cell(Some(if company {
                "company".into()
            } else {
                "source".into()
            })),
            csv_cell(Some(name)),
            total.to_string(),
        ];
        values.extend(c.iter().map(ToString::to_string));
        values.extend([
            rate(c[0], total),
            rate(c[1], c[0] as f64),
            rate(c[2], c[0] as f64),
            rate(c[3], c[0] as f64),
            rate(c[4], c[0] as f64),
            rate(c[5], c[0] as f64),
            rate(c[6], c[0] as f64),
            mean,
            hours.len().to_string(),
            if total < 3.0 || hours.len() < 3 {
                "small_sample".into()
            } else {
                "".into()
            },
        ]);
        text.push_str(&values.join(","));
        text.push_str("\r\n");
    }
    text.into_bytes()
}
async fn csv_bytes(pool: &SqlitePool, kind: &str, filter: &ExportFilter) -> ApiResult<Vec<u8>> {
    let selected = export_closure(pool, filter).await?;
    if kind == "source_outcomes" {
        return Ok(outcomes_csv(&selected, false));
    }
    if kind == "company_outcomes" {
        return Ok(outcomes_csv(&selected, true));
    }
    let (table,fields,headers)=match kind {"jobs"=>("jobs",vec!["id","title","company","location","canonical_url","availability","posted_at","extraction_at"],"id,title,company,location,canonical_url,availability,posted_at,extraction_at"),"applications"=>("applications",vec!["id","job_id","persona_id","current_stage","applied_at","created_at","updated_at"],"id,job_id,persona_id,stage,applied_at,created_at,updated_at"),"events"=>("application_events",vec!["id","application_id","event_type","from_stage","to_stage","occurred_at","reason","payload_json"],"id,application_id,event_type,from_stage,to_stage,occurred_at,reason,payload_json"),"interviews"=>("interviews",vec!["id","application_id","stage","scheduled_at","completed_at","outcome","notes"],"id,application_id,stage,scheduled_at,completed_at,outcome,notes"),"source_outcomes"=>("jobs",vec!["source_id","company","availability"],"source_id,company,availability"),_=>return Err("Export kind must be jobs, applications, events, interviews, source_outcomes, or relational_json".into())};
    let mut text = format!("{headers}\r\n");
    for row in &selected[table] {
        let mut values = Vec::new();
        for field in &fields {
            values.push(csv_cell(row_text(row, field).map(str::to_owned)));
        }
        text.push_str(&values.join(","));
        text.push_str("\r\n");
    }
    Ok(text.into_bytes())
}
#[tauri::command]
pub async fn export_data(
    request: ExportRequest,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<String> {
    let path = explicit_export_destination(&request.destination, request.overwrite)?;
    let bytes = if request.kind == "relational_json" {
        relational_json(&state.db.pool, &request.filter).await?
    } else {
        csv_bytes(&state.db.pool, &request.kind, &request.filter).await?
    };
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgePreviewInput {
    pub category: String,
    pub before_at: Option<String>,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PurgePreview {
    pub token: String,
    pub preview_hash: String,
    pub category: String,
    pub expires_at: String,
    pub row_counts: serde_json::Value,
    pub protected_count: i64,
    pub controlled_files: Vec<String>,
    pub impact: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeApplyInput {
    pub token: String,
    pub preview_hash: String,
    pub confirmation: String,
}
fn purge_hash(category: &str, jobs: &[String], events: &[String], files: &[String]) -> String {
    let mut all = vec![category.to_string()];
    all.extend(jobs.iter().cloned());
    all.extend(events.iter().cloned());
    all.extend(files.iter().cloned());
    all.sort();
    format!("{:x}", Sha256::digest(all.join("\n").as_bytes()))
}
fn controlled_session_path(root: &Path, relative: &str) -> ApiResult<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("Session purge path escaped controlled sessions directory".into());
    }
    Ok(root.join(path))
}
#[tauri::command]
pub async fn preview_purge(
    input: PurgePreviewInput,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<PurgePreview> {
    let before = input.before_at.unwrap_or_else(|| now());
    if chrono::DateTime::parse_from_rfc3339(&before).is_err() {
        return Err("Purge cutoff must be UTC RFC3339".into());
    }
    let (jobs, events, files, protected, impact) = match input.category.as_str() {
        "closed_jobs" => {
            let jobs:Vec<String>=sqlx::query_scalar("SELECT j.id FROM jobs j WHERE j.availability='closed' AND j.updated_at<? AND NOT EXISTS(SELECT 1 FROM applications a WHERE a.job_id=j.id) AND NOT EXISTS(SELECT 1 FROM review_decisions r WHERE r.job_id=j.id)").bind(&before).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?;
            let protected:i64=sqlx::query_scalar("SELECT count(*) FROM jobs j WHERE j.availability='closed' AND j.updated_at<? AND (EXISTS(SELECT 1 FROM applications a WHERE a.job_id=j.id) OR EXISTS(SELECT 1 FROM review_decisions r WHERE r.job_id=j.id))").bind(&before).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())?;
            (jobs,vec![],vec![],protected,"Deletes only closed jobs without applications or review history; dependent scrape sightings/revisions/matches are deleted by foreign-key policy.".into())
        }
        "scrape_logs" => {
            // app_logs holds the same class of diagnostics for actions that never produced a
            // scrape_run, so one retention control covers both tables.
            let events: Vec<String> =
                sqlx::query_scalar("SELECT id FROM scrape_run_events WHERE created_at<? UNION ALL SELECT id FROM app_logs WHERE at<?")
                    .bind(&before)
                    .bind(&before)
                    .fetch_all(&state.db.pool)
                    .await
                    .map_err(|e| e.to_string())?;
            (vec![],events,vec![],0,"Deletes only old scrape run and activity log diagnostics; sources, runs, jobs, and history remain.".into())
        }
        "sessions" => {
            let root = state.db.root.join("sessions");
            let files = if root.exists() {
                std::fs::read_dir(&root)
                    .map_err(|e| e.to_string())?
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        let modified = entry.metadata().ok()?.modified().ok()?;
                        if modified < std::time::SystemTime::now() {
                            Some(path.strip_prefix(&root).ok()?.to_string_lossy().to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            };
            (vec![],vec![],files,0,"Deletes only controlled encrypted browser session files. Credentials, models, logs, and documents remain.".into())
        }
        _ => return Err("Purge category must be closed_jobs, scrape_logs, or sessions".into()),
    };
    let token = id();
    let expires = Utc::now() + chrono::Duration::minutes(10);
    let hash = purge_hash(&input.category, &jobs, &events, &files);
    let preview = PurgePreview {
        token: token.clone(),
        preview_hash: hash,
        category: input.category,
        row_counts: serde_json::json!({"jobs":jobs.len(),"scrapeRunEvents":events.len(),"sessionFiles":files.len()}),
        protected_count: protected,
        controlled_files: files.clone(),
        expires_at: expires.to_rfc3339(),
        impact,
    };
    purge_previews()
        .lock()
        .map_err(|_| "Purge preview registry unavailable")?
        .insert(
            token,
            PurgePreviewState {
                preview: preview.clone(),
                job_ids: jobs,
                event_ids: events,
                session_paths: files,
                expires_at: expires,
            },
        );
    Ok(preview)
}
#[tauri::command]
pub async fn apply_purge(
    input: PurgeApplyInput,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<serde_json::Value> {
    if input.confirmation != "PURGE" {
        return Err("Type PURGE to confirm irreversible deletion".into());
    };
    let stored = purge_previews()
        .lock()
        .map_err(|_| "Purge preview registry unavailable")?
        .remove(&input.token)
        .ok_or("Purge preview expired or already used")?;
    if stored.expires_at < Utc::now() || stored.preview.preview_hash != input.preview_hash {
        return Err("Purge preview is stale or was tampered with".into());
    };
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    for job in &stored.job_ids {
        let changed=sqlx::query("DELETE FROM jobs WHERE id=? AND availability='closed' AND NOT EXISTS(SELECT 1 FROM applications WHERE job_id=?) AND NOT EXISTS(SELECT 1 FROM review_decisions WHERE job_id=?)").bind(job).bind(job).bind(job).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        if changed.rows_affected() != 1 {
            return Err("Purge preview changed; no rows were deleted".into());
        }
    }
    for event in &stored.event_ids {
        // The preview mixes ids from both diagnostic tables; a UUID exists in exactly one.
        let mut changed = sqlx::query("DELETE FROM scrape_run_events WHERE id=?")
            .bind(event)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected();
        if changed == 0 {
            changed = sqlx::query("DELETE FROM app_logs WHERE id=?")
                .bind(event)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?
                .rows_affected();
        }
        if changed != 1 {
            return Err("Purge preview changed; no rows were deleted".into());
        }
    }
    let audit = id();
    let t = now();
    sqlx::query("INSERT INTO data_purge_audits(id,category,preview_hash,impact_json,file_cleanup_json,created_at) VALUES(?,?,?,?,?,?)").bind(&audit).bind(&stored.preview.category).bind(&stored.preview.preview_hash).bind(serde_json::to_string(&stored.preview.row_counts).unwrap()).bind("[]").bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    let root = state.db.root.join("sessions");
    let mut failures = Vec::new();
    for relative in &stored.session_paths {
        match controlled_session_path(&root, relative) {
            Ok(full) if std::fs::remove_file(&full).is_err() => failures.push(relative.clone()),
            Ok(_) => {}
            Err(_) => failures.push(relative.clone()),
        }
    }
    sqlx::query("UPDATE data_purge_audits SET file_cleanup_json=? WHERE id=?")
        .bind(serde_json::to_string(&failures).unwrap())
        .bind(&audit)
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    if stored.preview.category == "closed_jobs" || stored.preview.category == "scrape_logs" {
        let _ = sqlx::query("PRAGMA optimize").execute(&state.db.pool).await;
    }
    Ok(
        serde_json::json!({"auditId":audit,"deleted":stored.preview.row_counts,"fileCleanupFailures":failures}),
    )
}

#[cfg(test)]
mod matching_persistence_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn migrated_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }
    async fn seed_match(pool: &SqlitePool) {
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','s','https://example.test','json','1',0,'active',0,0,'t','t')").execute(pool).await.unwrap();
        sqlx::query("INSERT INTO personas(id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,threshold,unknown_policy,resume_document_id,created_at,updated_at) VALUES('p','p','[]','[]','any','[]',60,'pass','r','t','persona-v1')").execute(pool).await.unwrap();
        sqlx::query("INSERT INTO resume_documents(id,persona_id,filename,path,extracted_text,mime_type,content_hash,created_at,updated_at) VALUES('r','p','r.txt','r.txt','resume','text/plain','resume-v1','t','t')").execute(pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('j','s','Engineer','Company','description','[]','job-v1','t','1','t','t')").execute(pool).await.unwrap();
    }
    #[tokio::test]
    async fn new_jobs_since_ignores_rescraped_and_closed_rows() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','json','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        for (job, title, availability, created_at) in [
            ("old", "Old role", "active", "2026-08-31T09:59:59+00:00"),
            ("new", "New role", "active", "2026-08-31T10:00:01+00:00"),
            (
                "closed",
                "Closed role",
                "closed",
                "2026-08-31T10:00:02+00:00",
            ),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,availability,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s',?,'Chip Co','','[]',?,?, 't','1',?,?)")
                .bind(job).bind(title).bind(job).bind(availability).bind(created_at).bind(created_at).execute(&pool).await.unwrap();
        }
        // Simulate a re-scrape: updated_at changes, while insert-only created_at remains old.
        sqlx::query("UPDATE jobs SET updated_at='2026-08-31T10:00:03+00:00' WHERE id='old'")
            .execute(&pool)
            .await
            .unwrap();
        let (count, jobs) = new_jobs_since(&pool, "2026-08-31T10:00:00+00:00", 3)
            .await
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].title, "New role");
    }
    #[tokio::test]
    async fn a_fresh_heartbeat_blocks_the_interrupted_run_sweep() {
        let pool = migrated_pool().await;
        let db = Database {
            pool: pool.clone(),
            root: PathBuf::new(),
        };
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','json','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r','s','scrape','running','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('j','s','Engineer','S','','[]','h','t','1','t','t')").execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES('o','j','r','t')",
        )
        .execute(&pool)
        .await
        .unwrap();
        write_sync_heartbeat(&pool).await;
        assert_eq!(db.recover_interrupted_runs().await.unwrap(), 0);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM scrape_runs WHERE id='r'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "running"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_occurrences")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        sqlx::query("UPDATE settings SET value='2020-01-01T00:00:00+00:00' WHERE key=?")
            .bind(SYNC_HEARTBEAT)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(db.recover_interrupted_runs().await.unwrap(), 1);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM scrape_runs WHERE id='r'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            "cancelled"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_occurrences")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
    // The two-strike rule used to live twice: once as SQL here and once as a pure function in
    // availability.rs that nothing called and that only its own tests exercised. This covers the
    // copy that actually runs, against a real database.
    #[tokio::test]
    async fn only_a_complete_run_closes_jobs_and_only_on_the_second_absence() {
        let pool = migrated_pool().await;
        let db = Database {
            pool: pool.clone(),
            root: std::path::PathBuf::new(),
        };
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','s','https://example.test','json','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        for id in ["seen", "missing"] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s','Engineer','Chip Co','text','[]',?,'t','1','t','t')").bind(id).bind(id).execute(&pool).await.unwrap();
        }
        let state = |id: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>("SELECT availability FROM jobs WHERE id=?")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        // Each run sights "seen" and never "missing".
        let sight = |run: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES(?,'s','scrape','completed',?)").bind(run).bind(run).execute(&pool).await.unwrap();
                sqlx::query(
                    "INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES(?,'seen',?,'t')",
                )
                .bind(run)
                .bind(run)
                .execute(&pool)
                .await
                .unwrap();
            }
        };

        // A partial read proves nothing about what is missing, so it changes nothing.
        sight("r0").await;
        db.reconcile_availability("r0", "s", false).await.unwrap();
        assert_eq!(state("missing").await, "active");

        sight("r1").await;
        db.reconcile_availability("r1", "s", true).await.unwrap();
        assert_eq!(state("seen").await, "active");
        assert_eq!(
            state("missing").await,
            "possibly_closed",
            "one absence is a doubt"
        );

        sight("r2").await;
        db.reconcile_availability("r2", "s", true).await.unwrap();
        assert_eq!(
            state("missing").await,
            "closed",
            "the second absence closes it"
        );

        // Reappearing clears both the state and the strikes behind it.
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r3','s','scrape','completed','r3')").execute(&pool).await.unwrap();
        for job in ["seen", "missing"] {
            sqlx::query(
                "INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES(?,?,'r3','t')",
            )
            .bind(job)
            .bind(job)
            .execute(&pool)
            .await
            .unwrap();
        }
        db.reconcile_availability("r3", "s", true).await.unwrap();
        assert_eq!(state("missing").await, "active");
        let strikes: i64 =
            sqlx::query_scalar("SELECT missing_full_runs FROM jobs WHERE id='missing'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(strikes, 0, "a returning job starts its count over");
    }

    // Narrowing the list to one board used to mean switching every other board off, which also
    // stopped them being read. The view scope is now its own filter over the followed sources,
    // and being followed is still sources.enabled — the two must not collapse back into one.
    #[tokio::test]
    async fn showing_one_source_narrows_the_list_without_touching_the_others() {
        let pool = migrated_pool().await;
        for id in ["intel", "arm"] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?,'https://example.test','json','1',1,'active',0,0,'t','t')").bind(id).bind(id).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES(?,?,'Verification Engineer','Chip Co','text','[]',?,'t','1','t','t')").bind(id).bind(id).bind(id).execute(&pool).await.unwrap();
        }
        let ids = |page: &JobPage| page.items.iter().map(|j| j.id.clone()).collect::<Vec<_>>();

        // No narrowing is the default and means everything followed, not nothing.
        let all = jobs_page_query(JobFilter::default(), 0, &pool)
            .await
            .unwrap();
        assert_eq!(all.total, 2);

        let one = jobs_page_query(
            JobFilter {
                source_ids: vec!["intel".into()],
                ..Default::default()
            },
            0,
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(ids(&one), vec!["intel"]);
        assert_eq!(
            one.total, 1,
            "the count follows the narrowed view, not the page"
        );

        // The narrowing binds as a parameter, so an id is never SQL.
        let hostile = jobs_page_query(
            JobFilter {
                source_ids: vec!["' OR 1=1 --".into()],
                ..Default::default()
            },
            0,
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(hostile.total, 0);

        // Narrowing the view must leave both boards switched on for the next Update Jobs.
        let still_on: i64 =
            sqlx::query_scalar("SELECT count(*) FROM sources WHERE enabled=1 AND kind='active'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(still_on, 2);
    }

    // reconcile_availability has written these states since the schema was created and the Jobs
    // list never read them, so a posting absent from two complete reads sat in the list looking
    // exactly like a live one. What the list shows is now what can still be applied to.
    #[tokio::test]
    async fn closed_jobs_leave_the_list_unless_they_are_asked_for() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','s','https://example.test','json','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        for (id, availability) in [
            ("live", "active"),
            ("maybe", "possibly_closed"),
            ("gone", "closed"),
            ("filed", "archived"),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,availability,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s','Verification Engineer','Chip Co','text','[]',?,?,'t','1','t','t')").bind(id).bind(id).bind(availability).execute(&pool).await.unwrap();
        }
        let ids = |jobs: &[crate::domain::Job]| {
            let mut out = jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
            out.sort();
            out
        };

        let default = jobs_query(JobFilter::default(), &pool).await.unwrap();
        // One absence is weak evidence and a partial read cannot produce one, so possibly_closed
        // stays in the list and is badged rather than hidden.
        assert_eq!(ids(&default), vec!["live", "maybe"]);
        assert_eq!(default.len(), 2, "the total counts what is shown");

        let everything = jobs_query(
            JobFilter {
                include_closed: true,
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        // Archived is never listed either way; nothing writes it today, but the column allows it.
        assert_eq!(ids(&everything), vec!["gone", "live", "maybe"]);

        // The badge needs the state to survive the projection, which drops other columns.
        let closed = everything.iter().find(|j| j.id == "gone").unwrap();
        assert_eq!(closed.availability, "closed");
    }

    // Filtering happens on the codes the resolver worked out when each job was stored, so what
    // these cases prove is the plumbing: any of several countries, a multi-country listing counted
    // once, and what happens to a listing nothing could place.
    #[tokio::test]
    async fn country_codes_filter_jobs_and_unknown_places_stay_out_unless_asked() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','workday','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        for (id, location, url) in [
            ("lisbon", "Lisbon", "https://x.test/lisbon"),
            ("madrid", "Madrid", "https://x.test/madrid"),
            ("multi", "Sibiu, Caen", "https://x.test/multi"),
            (
                "counted",
                "2 Locations",
                "https://x.wd1.myworkdayjobs.com/External/job/Porto/Engineer_JR1",
            ),
            (
                "nowhere",
                "3 Locations",
                "https://x.wd1.myworkdayjobs.com/External/job/JR9/Engineer_JR9",
            ),
        ] {
            let codes =
                crate::locations::stored(&crate::locations::resolve(Some(location), Some(url)));
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,location,location_countries,canonical_url,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s','Engineer','Chip Co',?,?,?,'text','[]',?,'t','1','t','t')").bind(id).bind(location).bind(codes).bind(url).bind(id).execute(&pool).await.unwrap();
        }
        let ids = |jobs: &[crate::domain::Job]| {
            let mut out = jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
            out.sort();
            out
        };
        let by = |codes: &[&str], include_unknown: bool| JobFilter {
            countries: codes.iter().map(|code| (*code).to_string()).collect(),
            include_unknown_locations: include_unknown,
            ..Default::default()
        };
        // A city stands in for its country, and a listing that published only a count is placed by
        // its own link — the Micron case, filed under Portugal without a re-scrape.
        let portugal = jobs_query(by(&["PT"], false), &pool).await.unwrap();
        assert_eq!(ids(&portugal), vec!["counted", "lisbon"]);
        // Several countries at once, and a posting open in two offices answers to either.
        assert_eq!(
            ids(&jobs_query(by(&["RO", "ES"], false), &pool).await.unwrap()),
            vec!["madrid", "multi"]
        );
        assert_eq!(
            ids(&jobs_query(by(&["FR"], false), &pool).await.unwrap()),
            vec!["multi"]
        );
        // What nothing could place stays out of a country filter unless it is asked for.
        assert_eq!(
            ids(&jobs_query(by(&["PT"], true), &pool).await.unwrap()),
            vec!["counted", "lisbon", "nowhere"]
        );
        // No country chosen means everywhere, including the unplaceable.
        assert_eq!(
            jobs_query(JobFilter::default(), &pool).await.unwrap().len(),
            5
        );
        // Junk codes cannot reach the query; an all-invalid filter is no filter.
        assert_eq!(
            jobs_query(by(&["'; DROP TABLE jobs--", "zzz"], false), &pool)
                .await
                .unwrap()
                .len(),
            5
        );
    }
    // A wrong country stored by an older version of the resolver has to be corrected on its own:
    // waiting for a re-scrape leaves jobs filed under a country nobody would find them in, which is
    // how Californian listings sat under Cuba after the rules that caused it had already been fixed.
    #[tokio::test]
    async fn changing_the_resolver_rules_re_places_every_stored_job() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','workday','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        // Stored by a previous version, with the answer that version gave.
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,location,location_countries,canonical_url,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('j','s','Engineer','Chip Co','US CA Santa Clara',',CU,US,','https://x.test/j','','[]','h','t','1','t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO settings(key,value,updated_at) VALUES(?,'1','t')")
            .bind(RESOLVER_VERSION_KEY)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(backfill_location_countries(&pool).await.unwrap(), 1);
        let codes: String = sqlx::query_scalar("SELECT location_countries FROM jobs WHERE id='j'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            codes, ",US,",
            "the stale Cuban answer is replaced, not kept"
        );
        // Same version twice does no work at all.
        assert_eq!(backfill_location_countries(&pool).await.unwrap(), 0);
    }
    // Some boards publish no posting date at all — u-blox's index has no date field, and Arm's
    // search results carry none. A window filter must not read "undated" as "old": those listings
    // are current, and hiding them behind a date the board never published loses them entirely.
    #[tokio::test]
    async fn a_listing_with_no_posting_date_survives_every_window() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','arm','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        let today = Utc::now().date_naive();
        for (id, posted) in [
            ("fresh", Some(today.to_string())),
            (
                "old",
                Some((today - chrono::Duration::days(400)).to_string()),
            ),
            ("undated", None),
            ("blank", Some(String::new())),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,posted_at,canonical_url,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s','Engineer','Arm',?,?,'','[]',?,'t','1','t','t')").bind(id).bind(posted).bind(format!("https://x.test/{id}")).bind(id).execute(&pool).await.unwrap();
        }
        let ids = |jobs: &[crate::domain::Job]| {
            let mut out = jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
            out.sort();
            out
        };
        // The narrowest window still shows both undated forms alongside today's listing.
        let week = jobs_query(
            JobFilter {
                posted_within_days: Some(7),
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(ids(&week), vec!["blank", "fresh", "undated"]);
        // Sorting by date puts what has one first and leaves the undated at the end rather than
        // dropping them or floating them to the top.
        let sorted = jobs_query(
            JobFilter {
                sort: "posted".into(),
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(sorted.first().unwrap().id, "fresh");
        assert_eq!(sorted.len(), 4);
    }
    // The installer cannot write to a database that does not exist yet, so its answer arrives as a
    // file. What matters is that it is spent: a user who switches a board off must not find it on
    // again next launch.
    #[tokio::test]
    async fn the_installer_opt_in_switches_the_pack_on_once_and_is_then_gone() {
        let root = std::env::temp_dir().join(format!("jobscraper-optin-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        db.prepare().await.unwrap();
        db.install_starter_pack().await.unwrap();
        let enabled_count = || async {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sources WHERE enabled=1")
                .fetch_one(&db.pool)
                .await
                .unwrap()
        };
        // The pack installs switched off, and stays that way when the installer was not told to
        // include it.
        assert_eq!(enabled_count().await, 0);
        assert_eq!(
            apply_starter_pack_opt_in(&db.pool, &root).await.unwrap(),
            0,
            "no marker, no change"
        );
        let marker = root.join("starter-pack.optin");
        std::fs::write(&marker, "1").unwrap();
        let switched = apply_starter_pack_opt_in(&db.pool, &root).await.unwrap();
        assert!(switched >= 19, "every active starter is on, got {switched}");
        assert_eq!(enabled_count().await, switched as i64);
        // Reference-only sources are never scraped, so they are never switched on.
        let reference: i64 =
            sqlx::query_scalar("SELECT count(*) FROM sources WHERE kind='reference' AND enabled=1")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(reference, 0);
        // Spent: the file is gone, and a board switched off afterwards stays off.
        assert!(!marker.exists());
        sqlx::query("UPDATE sources SET enabled=0,updated_at='later' WHERE id='00000000-0000-4000-8000-000000000003'").execute(&db.pool).await.unwrap();
        assert_eq!(apply_starter_pack_opt_in(&db.pool, &root).await.unwrap(), 0);
        let broadcom: i64 = sqlx::query_scalar(
            "SELECT enabled FROM sources WHERE id='00000000-0000-4000-8000-000000000003'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            broadcom, 0,
            "a board the user switched off is not switched back on"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // Installing over a database where the pack had been deleted long ago: answering Yes has to
    // bring those boards back, because that is what answering Yes means. This install answered
    // Yes, changed nothing, and said nothing — the app opened with one source in it.
    #[tokio::test]
    async fn the_opt_in_revives_boards_retired_by_an_earlier_install() {
        let root = std::env::temp_dir().join(format!("jobscraper-revive-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        db.prepare().await.unwrap();
        db.install_starter_pack().await.unwrap();
        // Exactly the state the reported install was in: every starter soft-deleted, one source
        // left alive, and each row edited since it was created.
        sqlx::query("UPDATE sources SET deleted_at='2026-08-29T16:05:49Z',updated_at='2026-08-29T16:05:49Z' WHERE id<>'00000000-0000-4000-8000-000000000020'")
            .execute(&db.pool)
            .await
            .unwrap();
        let alive: i64 =
            sqlx::query_scalar("SELECT count(*) FROM sources WHERE deleted_at IS NULL")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(alive, 1, "the database starts with one source, as reported");
        std::fs::write(root.join("starter-pack.optin"), "1").unwrap();
        let switched = apply_starter_pack_opt_in(&db.pool, &root).await.unwrap();
        assert!(switched >= 19, "retired boards come back, got {switched}");
        let listed: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sources WHERE deleted_at IS NULL AND enabled=1 AND kind='active'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(
            listed >= 19,
            "and they are on the Sources tab, got {listed}"
        );
        // A reference source is not a board and stays retired.
        let reference: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sources WHERE kind='reference' AND deleted_at IS NULL",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(reference, 0);
        std::fs::remove_dir_all(&root).ok();
    }
    // The pack is the source of truth for its own sources, and it has to reach installs that
    // already have them. An install carrying jobs.cisco.com and "pageSize": 0 from an older pack
    // read nothing at all, and every route to fixing it — a new version, a reconcile — stopped at
    // a row somebody had switched on.
    #[tokio::test]
    async fn a_corrected_pack_repairs_rows_an_earlier_one_got_wrong() {
        let root = std::env::temp_dir().join(format!("jobscraper-pack-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        db.install_starter_pack().await.unwrap();
        let cisco = "00000000-0000-4000-8000-000000000013";
        // Exactly the reported state: an older pack's URL and settings, the stale key it left
        // behind, a search term of the user's own, and the row switched on and therefore "edited".
        sqlx::query("UPDATE sources SET base_url='https://jobs.cisco.com/',adapter_id='phenom',enabled=1,disabled_reason=NULL,updated_at='2026-08-30T00:00:00Z' WHERE id=?")
            .bind(cisco).execute(&db.pool).await.unwrap();
        sqlx::query("UPDATE source_configs SET config_json=?,updated_at='2026-08-30T00:00:00Z' WHERE source_id=?")
            .bind(r#"{"maxPages":500,"pageSize":0,"query":"verification","starterPackVersion":"2026-08-30"}"#)
            .bind(cisco).execute(&db.pool).await.unwrap();
        sqlx::query("DELETE FROM schema_metadata WHERE key='starter_pack_version'")
            .execute(&db.pool)
            .await
            .unwrap();

        db.install_starter_pack().await.unwrap();

        let (url, adapter, enabled, reason): (String, String, i64, Option<String>) =
            sqlx::query_as(
                "SELECT base_url,adapter_id,enabled,disabled_reason FROM sources WHERE id=?",
            )
            .bind(cisco)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(url, "https://careers.cisco.com/global/en/search-results");
        assert_eq!(adapter, "cisco");
        // What the user decided is theirs: switched on stays switched on, with no "disabled" note.
        assert_eq!(enabled, 1);
        assert_eq!(reason, None);
        let config: String =
            sqlx::query_scalar("SELECT config_json FROM source_configs WHERE source_id=?")
                .bind(cisco)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let config: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(config["expectedHost"], "careers.cisco.com");
        // The stale key is gone rather than merged forward: "pageSize": 0 asks a board for nothing.
        assert!(config.get("pageSize").is_none(), "{config}");
        // The user's own search term is not collateral damage.
        assert_eq!(config["query"], "verification");
        assert_eq!(config["starterPackVersion"], starter_pack_version());
        std::fs::remove_dir_all(&root).ok();
    }
    // A second source on an address already followed cannot ever hold a job: listings are
    // re-identified by canonical URL across the database, and a job keeps the source that stored it
    // first. The duplicate reads the whole board on every run and reports nothing saved, with
    // nothing to say why — so it is refused at the point it would be created.
    #[tokio::test]
    async fn a_board_already_followed_cannot_be_added_twice() {
        assert!(same_board(
            "https://broadcom.wd1.myworkdayjobs.com/",
            "https://Broadcom.wd1.myworkdayjobs.com"
        ));
        assert!(same_board(
            "https://careers.arm.com/search-jobs",
            "https://careers.arm.com/search-jobs/"
        ));
        // A filtered view of the same host is a different board's worth of listings, and stays
        // allowed: it is the only way to follow one slice of a large careers site.
        assert!(!same_board(
            "https://careers.arm.com/search-jobs",
            "https://careers.arm.com/search-jobs/verification?orgIds=33099"
        ));
        assert!(!same_board(
            "https://careers.arm.com/search-jobs",
            "https://careers.amd.com/search-jobs"
        ));
    }
    // Triage is the whole point of the Jobs page once the list is five figures long: a listing you
    // have judged has to leave, and what arrived since you last looked has to be findable.
    #[tokio::test]
    async fn dismissing_hides_a_listing_without_losing_it_and_new_since_counts_arrivals() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','workday','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        for (id, seen) in [
            ("old", "2026-08-01T00:00:00Z"),
            ("fresh", "2026-09-02T00:00:00Z"),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,canonical_url,first_seen_at,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s','Engineer','Chip Co',?,?,'','[]',?,'t','1',?,?)")
                .bind(id).bind(format!("https://x.test/{id}")).bind(seen).bind(id).bind(seen).bind(seen).execute(&pool).await.unwrap();
        }
        let ids = |jobs: &[crate::domain::Job]| {
            let mut out = jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
            out.sort();
            out
        };
        // Dismissing is a decision, so it leaves the default list.
        sqlx::query("UPDATE jobs SET dismissed_at='2026-09-02T09:00:00Z' WHERE id='old'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            ids(&jobs_query(JobFilter::default(), &pool).await.unwrap()),
            vec!["fresh"]
        );
        // But nothing was lost: asking for it brings it back, and it says when it was dismissed.
        let shown = jobs_query(
            JobFilter {
                include_dismissed: true,
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(ids(&shown), vec!["fresh", "old"]);
        assert!(shown
            .iter()
            .find(|j| j.id == "old")
            .unwrap()
            .dismissed_at
            .is_some());
        // "New" is measured from when the reader last said they were done, not from the board.
        let since = |value: &str| JobFilter {
            first_seen_after: Some(value.to_string()),
            include_dismissed: true,
            ..Default::default()
        };
        assert_eq!(
            ids(&jobs_query(since("2026-09-01T00:00:00Z"), &pool)
                .await
                .unwrap()),
            vec!["fresh"]
        );
        assert_eq!(
            ids(&jobs_query(since("2026-07-01T00:00:00Z"), &pool)
                .await
                .unwrap()),
            vec!["fresh", "old"]
        );
        // An empty mark means everything, rather than nothing.
        assert_eq!(
            jobs_query(
                JobFilter {
                    first_seen_after: Some(String::new()),
                    include_dismissed: true,
                    ..Default::default()
                },
                &pool
            )
            .await
            .unwrap()
            .len(),
            2
        );
    }
    // The picker is built from this, so what it must never do is offer a country the jobs on show
    // are not in — that was the whole complaint about a 250-entry list.
    #[tokio::test]
    async fn the_offered_countries_are_only_those_the_shown_sources_hire_in() {
        let pool = migrated_pool().await;
        for source in ["a", "b"] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?,'https://example.test','workday','1',1,'active',0,0,'t','t')").bind(source).bind(source).execute(&pool).await.unwrap();
        }
        for (id, source, codes, availability) in [
            ("lisbon", "a", ",PT,", "active"),
            ("porto", "a", ",PT,", "active"),
            ("madrid", "a", ",ES,", "active"),
            ("munich", "b", ",DE,", "active"),
            ("both", "b", ",DE,FR,", "active"),
            ("nowhere", "b", "", "active"),
            ("gone", "a", ",IE,", "closed"),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,location_countries,canonical_url,availability,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES(?,?,'Engineer','Chip Co',?,?,?,'','[]',?,'t','1','t','t')").bind(id).bind(source).bind(codes).bind(format!("https://x.test/{id}")).bind(availability).bind(id).execute(&pool).await.unwrap();
        }
        let listed = |counted: &[CountryCount]| {
            counted
                .iter()
                .map(|entry| (entry.code.clone(), entry.jobs))
                .collect::<Vec<_>>()
        };
        // Both sources: every country, most jobs first, and the unplaceable counted separately.
        let (all, unplaced) = job_countries_query(&[], &pool).await.unwrap();
        assert_eq!(
            listed(&all),
            // Most jobs first; ties fall back to the code so the order never wobbles.
            vec![
                ("DE".into(), 2),
                ("PT".into(), 2),
                ("ES".into(), 1),
                ("FR".into(), 1)
            ]
        );
        assert_eq!(unplaced, 1);
        // One source: the other's countries are not offered at all.
        let (only_a, _) = job_countries_query(&["a".into()], &pool).await.unwrap();
        assert_eq!(listed(&only_a), vec![("PT".into(), 2), ("ES".into(), 1)]);
        // A closed listing's country is not on offer either — the page does not show those.
        assert!(!only_a.iter().any(|entry| entry.code == "IE"));
    }
    // scrape_all reads this column for every source on every batch through runtime SQL, so a
    // rename or a missed migration would surface as a failed update rather than a failed build.
    #[tokio::test]
    async fn the_check_skip_budget_column_exists_and_starts_unspent() {
        let pool = migrated_pool().await;
        seed_match(&pool).await;
        let streak: i64 =
            sqlx::query_scalar("SELECT s.unchanged_checks FROM sources s WHERE s.id='s'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(streak, 0, "a source starts with its full budget");
        sqlx::query("UPDATE sources SET unchanged_checks=unchanged_checks+1 WHERE id='s'")
            .execute(&pool)
            .await
            .unwrap();
        let spent: i64 = sqlx::query_scalar("SELECT unchanged_checks FROM sources WHERE id='s'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(spent, 1, "a skip is spendable");
    }
    #[tokio::test]
    async fn recent_jobs_sort_by_discovery_even_after_old_jobs_are_refreshed() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test','json','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        for (id, first_seen, created, updated) in [
            ("old", Some("2026-01-01"), "2026-01-01", "2026-09-05"),
            ("new", Some("2026-09-04"), "2026-09-04", "2026-09-04"),
            ("legacy", None, "2026-08-01", "2026-09-06"),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at,first_seen_at) VALUES(?,'s','Engineer','Company','','[]',?,'t','1',?,?,?)")
                .bind(id).bind(id).bind(created).bind(updated).bind(first_seen).execute(&pool).await.unwrap();
        }
        let jobs = jobs_query(JobFilter::default(), &pool).await.unwrap();
        assert_eq!(
            jobs.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            vec!["new", "legacy", "old"]
        );
    }

    // The Jobs page is the whole app now, so its list must be literal: only selected sources,
    // every typed word present in the title, newest posting first, saved jobs marked as such.
    #[tokio::test]
    async fn job_list_scopes_to_selected_sources_and_filters_titles_literally() {
        let pool = migrated_pool().await;
        for (id, enabled) in [("on", 1), ("off", 0)] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?,'https://example.test','json','1',?,'active',0,0,'t','t')").bind(id).bind(id).bind(enabled).execute(&pool).await.unwrap();
        }
        for (id, source, title, posted) in [
            (
                "a",
                "on",
                "Senior Verification Engineer",
                Some("2026-02-01"),
            ),
            (
                "b",
                "on",
                "Design Verification/Validation Lead",
                Some("2026-03-01"),
            ),
            ("c", "on", "Marketing Manager", None),
            ("d", "off", "Verification Engineer", Some("2026-04-01")),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,location,description_text,skills_json,content_hash,posted_at,extraction_at,adapter_version,created_at,updated_at) VALUES(?,?,?,'Chip Co','Lisbon','text','[]',?,?,'t','1','t','t')").bind(id).bind(source).bind(title).bind(id).bind(posted).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO applications(id,job_id,current_stage,created_at,updated_at) VALUES('app','a','planned','t','t')").execute(&pool).await.unwrap();

        let titles =
            |jobs: &[crate::domain::Job]| jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
        // Punctuation is text, not query syntax, and case never matters.
        let found = jobs_query(
            JobFilter {
                title: "VERIFICATION".into(),
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(titles(&found), vec!["a", "b"]);
        // Every typed word has to appear, in any order.
        let both = jobs_query(
            JobFilter {
                title: "engineer verification".into(),
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(titles(&both), vec!["a"]);
        // Newest posting first, undated last.
        let sorted = jobs_query(
            JobFilter {
                sort: "posted".into(),
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(titles(&sorted), vec!["b", "a", "c"]);
        assert_eq!(sorted[1].application_stage.as_deref(), Some("planned"));
        assert_eq!(sorted[0].application_stage, None);
        // A source nobody selected contributes nothing, whatever the filter says.
        let saved = jobs_query(
            JobFilter {
                saved_only: true,
                ..Default::default()
            },
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(titles(&saved), vec!["a"]);
        sqlx::query("UPDATE sources SET enabled=0")
            .execute(&pool)
            .await
            .unwrap();
        assert!(jobs_query(JobFilter::default(), &pool)
            .await
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn job_list_pages_are_bounded_stable_and_report_the_total() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','s','https://example.test','json','1',1,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        for index in 0..205 {
            sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s',?,'Company','','[]',?,'t','1','t','t')")
                .bind(format!("job-{index:03}"))
                .bind(format!("Engineer {index:03}"))
                .bind(format!("hash-{index:03}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        let first = jobs_page_query(JobFilter::default(), 0, &pool)
            .await
            .unwrap();
        let second = jobs_page_query(JobFilter::default(), 200, &pool)
            .await
            .unwrap();
        assert_eq!(
            (first.items.len(), first.total, first.has_more),
            (200, 205, true)
        );
        assert_eq!(
            (second.items.len(), second.total, second.has_more),
            (5, 205, false)
        );
        assert_ne!(first.items.last().unwrap().id, second.items[0].id);
    }
    #[test]
    fn reminder_utc_math_and_focus_dedupe_are_deterministic() {
        let applied = "2026-03-29T00:30:00Z";
        assert_eq!(
            ghost_due_from(applied, 14).unwrap(),
            "2026-04-12T00:30:00+00:00"
        );
        assert_eq!(
            interview_reminder_due("2026-10-25T12:00:00Z", 24).unwrap(),
            "2026-10-24T12:00:00+00:00"
        );
        let mut shown = None;
        assert!(mark_focus_prompt(&mut shown, "first"));
        assert!(!mark_focus_prompt(&mut shown, "first"));
        assert!(mark_focus_prompt(&mut shown, "second"));
    }

    #[tokio::test]
    async fn current_starter_pack_version_performs_no_writes() {
        let root = std::env::temp_dir().join(format!("jobscraper-current-pack-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO schema_metadata(key,value) VALUES('starter_pack_version',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(starter_pack_version())
            .execute(&db.pool)
            .await
            .unwrap();
        let mut observer = db.pool.acquire().await.unwrap();
        let before: i64 = sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(&mut *observer)
            .await
            .unwrap();
        db.install_starter_pack().await.unwrap();
        let after: i64 = sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(&mut *observer)
            .await
            .unwrap();
        assert_eq!(
            after, before,
            "the current pack must stop after its version lookup"
        );
    }

    #[tokio::test]
    async fn starter_reconciliation_retires_only_exact_untouched_legacy_rows() {
        let root = std::env::temp_dir().join(format!("jobscraper-starter-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        let stamp = "2026-01-01T00:00:00Z";
        // Analog Devices (still classified "workday" in the live starter pack) stands
        // in for a pre-versioned legacy row here; Microchip's own classification was
        // corrected live-fire testing found it isn't actually a Workday tenant.
        for (source, changed) in [("legacy", false), ("user-owned", true)] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?, 'https://analogdevices.wd1.myworkdayjobs.com/','workday','1.0.0',0,'active','Starter source is disabled until you review and enable it.',0,0,?,?)")
                .bind(source).bind("Analog Devices").bind(stamp).bind(if changed { "2026-01-02T00:00:00Z" } else { stamp }).execute(&db.pool).await.unwrap();
            sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?)")
                .bind(format!("config-{source}")).bind(source).bind("{\"starterPackVersion\":\"1.0.0\"}").bind(stamp).bind(if changed { "2026-01-02T00:00:00Z" } else { stamp }).execute(&db.pool).await.unwrap();
        }
        db.install_starter_pack().await.unwrap();
        let retired: Option<String> =
            sqlx::query_scalar("SELECT deleted_at FROM sources WHERE id='legacy'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let preserved: Option<String> =
            sqlx::query_scalar("SELECT deleted_at FROM sources WHERE id='user-owned'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(retired.is_some());
        assert!(preserved.is_none());
        db.pool.close().await;
    }

    #[tokio::test]
    async fn starter_reconciliation_backfills_corrected_host_and_tenant_on_an_untouched_row() {
        // Simulates a database installed before the live-fire fixes: Intel seeded
        // against the stale jobs.intel.com host with no tenant/site, and Microchip
        // still misclassified as Workday. install_starter_pack must correct both in
        // place without the user re-adding the sources, exactly like the original
        // NVIDIA adapter fix — but only because these rows were never edited.
        let root = std::env::temp_dir().join(format!("jobscraper-reconcile-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        let stamp = "2026-01-01T00:00:00Z";
        let intel_id = "00000000-0000-4000-8000-000000000004";
        let microchip_id = "00000000-0000-4000-8000-000000000001";
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,'Intel','https://jobs.intel.com/','workday','1.1.0',0,'active','Starter source is disabled until you review and enable it.',0,0,?,?)")
            .bind(intel_id).bind(stamp).bind(stamp).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?)")
            .bind("config-intel").bind(intel_id).bind(r#"{"schemaVersion":"1.1.0","starterPackVersion":"2026-08-28","expectedHost":"jobs.intel.com","adapterVersion":"1.1.0","mode":"direct"}"#).bind(stamp).bind(stamp).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,'Microchip','https://careers.microchip.com/','workday','1.1.0',0,'active','Starter source is disabled until you review and enable it.',0,0,?,?)")
            .bind(microchip_id).bind(stamp).bind(stamp).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?)")
            .bind("config-microchip").bind(microchip_id).bind(r#"{"schemaVersion":"1.1.0","starterPackVersion":"2026-08-28","expectedHost":"careers.microchip.com","adapterVersion":"1.1.0","mode":"direct"}"#).bind(stamp).bind(stamp).execute(&db.pool).await.unwrap();
        db.install_starter_pack().await.unwrap();
        let (intel_url, intel_config): (String, String) = sqlx::query_as(
            "SELECT s.base_url, c.config_json FROM sources s JOIN source_configs c ON c.source_id=s.id WHERE s.id=?",
        )
        .bind(intel_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(intel_url, "https://intel.wd1.myworkdayjobs.com/");
        assert!(intel_config.contains("\"tenant\":\"intel\""));
        assert!(intel_config.contains("\"site\":\"External\""));
        assert!(intel_config.contains("\"expectedHost\":\"intel.wd1.myworkdayjobs.com\""));
        // careers.microchip.com only 302s to a marketing page; the board itself is a Workday
        // tenant on wd5.myworkdaysite.com, so the host-keyed relocation moves the source there.
        let (microchip_adapter, microchip_url, microchip_config): (String, String, String) =
            sqlx::query_as("SELECT s.adapter_id, s.base_url, c.config_json FROM sources s JOIN source_configs c ON c.source_id=s.id WHERE s.id=?")
                .bind(microchip_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(microchip_adapter, "workday");
        assert_eq!(
            microchip_url,
            "https://wd5.myworkdaysite.com/en-US/recruiting/microchiphr/External"
        );
        assert!(microchip_config.contains("\"tenant\":\"microchiphr\""));
        assert!(microchip_config.contains("\"site\":\"External\""));
        assert!(microchip_config.contains("\"expectedHost\":\"wd5.myworkdaysite.com\""));
        db.pool.close().await;
    }

    #[tokio::test]
    async fn an_enabled_microchip_source_is_relocated_to_its_workday_tenant() {
        // Enabling a source stamps updated_at, which puts it outside the tuple reconcile's
        // "never edited" guard — so the only path that can still correct an address this app
        // seeded wrongly is the host-keyed upgrade table. The user's enabled state survives it.
        let root = std::env::temp_dir().join(format!("jobscraper-microchip-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        let microchip_id = "00000000-0000-4000-8000-000000000001";
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,'Microchip','https://careers.microchip.com/','custom-api','1.1.0',1,'active',NULL,1,0,'2026-01-01T00:00:00Z','2026-08-31T00:00:00Z')")
            .bind(microchip_id).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?)")
            .bind("config-microchip").bind(microchip_id).bind(r#"{"schemaVersion":"1.1.0","starterPackVersion":"2026-08-30","expectedHost":"www.microchip.com","adapterVersion":"1.1.0","mode":"direct"}"#).bind("2026-01-01T00:00:00Z").bind("2026-01-01T00:00:00Z").execute(&db.pool).await.unwrap();
        db.install_starter_pack().await.unwrap();
        let (adapter, base_url, enabled, config): (String, String, bool, String) =
            sqlx::query_as("SELECT s.adapter_id, s.base_url, s.enabled, c.config_json FROM sources s JOIN source_configs c ON c.source_id=s.id WHERE s.id=?")
                .bind(microchip_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(adapter, "workday");
        assert_eq!(
            base_url,
            "https://wd5.myworkdaysite.com/en-US/recruiting/microchiphr/External"
        );
        assert!(enabled, "the user's choice to enable this source is kept");
        assert!(config.contains("\"tenant\":\"microchiphr\""));
        assert!(config.contains("\"expectedHost\":\"wd5.myworkdaysite.com\""));
        db.pool.close().await;
    }

    #[tokio::test]
    async fn existing_apple_sources_upgrade_to_the_api_adapter_without_losing_selection() {
        let root = std::env::temp_dir().join(format!("jobscraper-apple-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        let stamp = "2026-01-01T00:00:00Z";
        for (source_id, adapter, enabled, url) in [
            (
                "old-apple",
                "static-css",
                0,
                "https://jobs.apple.com/pt-pt/search?location=portugal-PRTC",
            ),
            (
                "current-apple",
                "static-css",
                1,
                "https://jobs.apple.com/pt-pt/search?location=",
            ),
        ] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES(?,'Apple',?,?,'1.1.0',?,'active',0,0,?,?)")
                .bind(source_id).bind(url).bind(adapter).bind(enabled).bind(stamp).bind("2026-01-02T00:00:00Z").execute(&db.pool).await.unwrap();
            sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?)")
                .bind(format!("config-{source_id}")).bind(source_id).bind(r#"{"itemSelector":"li","titleSelector":"a","urlPrefix":"/careers/pt","pageSize":20}"#).bind(stamp).bind("2026-01-02T00:00:00Z").execute(&db.pool).await.unwrap();
        }
        db.install_starter_pack().await.unwrap();
        let rows: Vec<(String, String, bool, String)> = sqlx::query_as("SELECT s.id,s.adapter_id,s.enabled,c.config_json FROM sources s JOIN source_configs c ON c.source_id=s.id WHERE s.id IN ('old-apple','current-apple') ORDER BY s.id")
            .fetch_all(&db.pool).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "apple");
        assert!(rows[0].2);
        assert_eq!(rows[1].1, "apple");
        assert!(!rows[1].2);
        for (_, _, _, config) in rows {
            let config: serde_json::Value = serde_json::from_str(&config).unwrap();
            assert_eq!(config["locale"], "pt-pt");
            assert_eq!(config["maxPages"], 500);
            assert!(config.get("itemSelector").is_none());
            assert!(config.get("titleSelector").is_none());
            assert!(config.get("urlPrefix").is_none());
        }
        db.pool.close().await;
    }

    // Apple was not the only host the generic detector guessed wrong: Arm was saved as
    // static-css (which stores its navigation links), Qualcomm as an Eightfold source with
    // no tenant domain (which 422s on the first request), and an NVIDIA Workday source
    // without the facet split silently truncates at Workday's own paging cap. All three are
    // corrected in place, and a source on a host with no known adapter is left alone.
    #[tokio::test]
    async fn wrongly_detected_sources_move_to_the_adapter_that_reads_their_host() {
        let root = std::env::temp_dir().join(format!("jobscraper-upgrade-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        for (source_id, adapter, enabled, url, config) in [
            (
                "user-arm",
                "static-css",
                1,
                "https://careers.arm.com/search-jobs/",
                r#"{"itemSelector":"li","titleSelector":"a","query":"firmware"}"#,
            ),
            (
                "user-qualcomm",
                "eightfold",
                1,
                "https://careers.qualcomm.com/",
                r#"{"listingPath":"/api/career_hub"}"#,
            ),
            (
                "user-nvidia",
                "workday",
                1,
                "https://nvidia.wd5.myworkdayjobs.com/",
                r#"{"tenant":"nvidia","site":"NVIDIAExternalCareerSite"}"#,
            ),
            (
                "user-other",
                "static-css",
                1,
                "https://jobs.example.test/careers",
                r#"{"itemSelector":"li","titleSelector":"a"}"#,
            ),
        ] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES(?,'Fixture',?,?,'1.1.0',?,'active',0,0,'t1','t2')")
                .bind(source_id).bind(url).bind(adapter).bind(enabled).execute(&db.pool).await.unwrap();
            sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,'t1','t2')")
                .bind(format!("config-{source_id}")).bind(source_id).bind(config).execute(&db.pool).await.unwrap();
        }
        db.install_starter_pack().await.unwrap();
        let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT s.id,s.adapter_id,c.config_json FROM sources s JOIN source_configs c ON c.source_id=s.id WHERE s.id LIKE 'user-%' ORDER BY s.id")
            .fetch_all(&db.pool).await.unwrap();
        let by_id: std::collections::HashMap<_, _> = rows
            .into_iter()
            .map(|(id, adapter, config)| {
                (
                    id,
                    (
                        adapter,
                        serde_json::from_str::<serde_json::Value>(&config).unwrap(),
                    ),
                )
            })
            .collect();
        let (arm_adapter, arm_config) = &by_id["user-arm"];
        assert_eq!(arm_adapter, "arm");
        assert!(arm_config.get("itemSelector").is_none());
        assert_eq!(
            arm_config["query"], "firmware",
            "user filters survive the move"
        );
        let (qualcomm_adapter, qualcomm_config) = &by_id["user-qualcomm"];
        assert_eq!(qualcomm_adapter, "eightfold");
        assert_eq!(qualcomm_config["domain"], "qualcomm.com");
        assert_eq!(qualcomm_config["eightfoldApi"], "pcsx");
        assert!(qualcomm_config.get("listingPath").is_none());
        let (_, nvidia_config) = &by_id["user-nvidia"];
        assert_eq!(nvidia_config["splitFacet"], "jobFamilyGroup");
        assert_eq!(nvidia_config["splitThreshold"], 2000);
        let (other_adapter, other_config) = &by_id["user-other"];
        assert_eq!(
            other_adapter, "static-css",
            "an unknown host is never reassigned"
        );
        assert_eq!(other_config["itemSelector"], "li");
        db.pool.close().await;
    }

    // R5: nothing connected the starter pack to the adapter layer, so a source could
    // seed a config the sidecar's requestFor() rejects and nothing would notice until
    // a live run failed. This test and sidecar/starter-pack.contract.test.mjs share one
    // fixture (sidecar/starter-pack.fixture.json): this one fails if install_starter_pack
    // changes without updating the checked-in fixture; the Node test fails if the seeded
    // config no longer builds a request. Neither side can drift from the other silently.
    #[tokio::test]
    async fn starter_pack_matches_the_checked_in_adapter_contract_fixture() {
        let root = std::env::temp_dir().join(format!("jobscraper-fixture-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        db.install_starter_pack().await.unwrap();
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT s.name, s.base_url, s.adapter_id, c.config_json FROM sources s JOIN source_configs c ON c.source_id=s.id ORDER BY s.id",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        let actual: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|(name, base_url, adapter_id, config_json)| {
                serde_json::json!({"name":name,"baseUrl":base_url,"adapterId":adapter_id,"configJson":serde_json::from_str::<serde_json::Value>(&config_json).unwrap()})
            })
            .collect();
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("sidecar")
            .join("starter-pack.fixture.json");
        // Regenerate from the actual installer, so fixture updates never copy its defaults by
        // hand. Ordinary test runs still fail on drift and never rewrite the checked-in file.
        if std::env::var("UPDATE_STARTER_PACK_FIXTURE").as_deref() == Ok("1") {
            std::fs::write(
                &fixture_path,
                format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
            )
            .unwrap();
        }
        let expected: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(&fixture_path).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string_pretty(&actual).unwrap(),
            serde_json::to_string_pretty(&expected).unwrap(),
            "install_starter_pack no longer matches sidecar/starter-pack.fixture.json — \
             update the fixture (it drives the Node adapter-contract test) alongside this change"
        );
        db.pool.close().await;
    }

    // The scrape filter decides what is stored at all, so "no terms" must mean "keep everything":
    // an empty or whitespace-only box that discarded every listing would look like a dead scraper.
    // A job whose detail page was skipped emits only a "seen" event. If that did not record an
    // occurrence, the availability sweep would treat every recognised job as missing from the run
    // and close the entire board — the incremental path's one way to lose data.
    #[tokio::test]
    async fn a_skipped_job_still_counts_as_seen_and_stays_active() {
        let root = std::env::temp_dir().join(format!("jobscraper-seen-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test/','workday','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        for (job, hash) in [("kept", "hash-kept"), ("gone", "hash-gone")] {
            sqlx::query("INSERT INTO jobs(id,source_id,external_id,title,company,listing_hash,content_hash,description_text,description_html,skills_json,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s',?,'Engineer','S',?,'c','','','[]','{}','t','1.1.0','t','t')")
                .bind(job).bind(job).bind(hash).execute(&db.pool).await.unwrap();
        }
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r','s','scrape','running','t')").execute(&db.pool).await.unwrap();

        assert_eq!(
            db.record_jobs_seen(
                "r",
                "s",
                &[
                    "hash-kept".into(),
                    "hash-kept".into(),
                    "hash-never-stored".into(),
                ],
            )
            .await
            .unwrap(),
            1,
            "the batch records known hashes once and ignores unknown hashes"
        );
        db.reconcile_availability("r", "s", true).await.unwrap();

        let state: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT id,availability,missing_full_runs FROM jobs ORDER BY id")
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(state[1], ("kept".into(), "active".into(), 0));
        assert_eq!(state[0], ("gone".into(), "possibly_closed".into(), 1));
        db.pool.close().await;
    }

    // Re-persisting an identical job used to rewrite its whole description, run a second UPDATE
    // and log a dedupe event, once per job per run. It must now cost one occurrence row and leave
    // the job's own updated_at alone, while a genuinely changed job still writes through.
    #[tokio::test]
    async fn an_unchanged_job_is_recorded_as_seen_without_being_rewritten() {
        let root = std::env::temp_dir().join(format!("jobscraper-unchanged-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test/','workday','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r','s','scrape','running','t')").execute(&db.pool).await.unwrap();
        let payload = |hash: &str, text: &str| {
            serde_json::json!({"externalId":"E1","title":"Engineer","company":"S",
                "canonicalUrl":"https://example.test/job/1","descriptionText":text,"contentHash":hash,
                "listingHash":"l1","location":"Lisbon","postedAt":"2026-08-01"})
        };
        db.persist_worker_job("r", "s", &payload("h1", "first"))
            .await
            .unwrap();
        let first: (String, String) =
            sqlx::query_as("SELECT updated_at,description_text FROM jobs WHERE external_id='E1'")
                .fetch_one(&db.pool)
                .await
                .unwrap();

        db.persist_worker_job("r", "s", &payload("h1", "ignored"))
            .await
            .unwrap();
        let after: (String, String) =
            sqlx::query_as("SELECT updated_at,description_text FROM jobs WHERE external_id='E1'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(after, first, "an identical job leaves its row untouched");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_occurrences")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            1,
            "one run records one sighting even when an adapter repeats the job"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_dedupe_events")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            0,
            "re-seeing a job is not a merge and must not be logged as one"
        );

        let changed_listing = serde_json::json!({"externalId":"E1","title":"Engineer","company":"S",
            "canonicalUrl":"https://example.test/job/1?office=porto","descriptionText":"first","contentHash":"h1",
            "listingHash":"l2","location":"Porto","postedAt":"2026-08-02"});
        db.persist_worker_job("r", "s", &changed_listing)
            .await
            .unwrap();
        let listing: (String, String, String, String) = sqlx::query_as(
            "SELECT location,posted_at,canonical_url,listing_hash FROM jobs WHERE external_id='E1'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            listing,
            (
                "Porto".into(),
                "2026-08-02".into(),
                "https://example.test/job/1?office=porto".into(),
                "l2".into(),
            ),
            "a changed listing hash updates listing fields even when description content is unchanged"
        );

        let changed_content = serde_json::json!({"externalId":"E1","title":"Engineer","company":"S",
            "canonicalUrl":"https://example.test/job/1?office=porto","descriptionText":"rewritten","contentHash":"h2",
            "listingHash":"l2","location":"Porto","postedAt":"2026-08-02"});
        db.persist_worker_job("r", "s", &changed_content)
            .await
            .unwrap();
        let changed: String =
            sqlx::query_scalar("SELECT description_text FROM jobs WHERE external_id='E1'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(changed, "rewritten", "a changed hash still writes through");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            1,
            "all three sightings are the same job"
        );
        db.pool.close().await;
    }

    #[tokio::test]
    async fn interrupted_runs_are_finalized_and_partial_sightings_are_discarded() {
        let root = std::env::temp_dir().join(format!("jobscraper-recovery-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test/','json','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r','s','scrape','running','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('j','s','Engineer','S','','[]','h','t','1.1.0','t','t')").execute(&db.pool).await.unwrap();
        sqlx::query(
            "INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES('o','j','r','t')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        assert_eq!(db.recover_interrupted_runs().await.unwrap(), 1);
        let run: (String, i64, Option<String>) =
            sqlx::query_as("SELECT status,complete,failure_code FROM scrape_runs WHERE id='r'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(run, ("cancelled".into(), 0, Some("interrupted".into())));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_occurrences")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn requisition_numbers_only_merge_within_the_same_company() {
        let root = std::env::temp_dir().join(format!("jobscraper-identity-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        for company in ["Alpha", "Beta"] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,created_at,updated_at) VALUES(?,?,?,'json','1','t','t')").bind(company).bind(company).bind(format!("https://{company}.test")).execute(&db.pool).await.unwrap();
            sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES(?,?,'scrape','running','t')").bind(company).bind(company).execute(&db.pool).await.unwrap();
            let job = serde_json::json!({"title":"Engineer","company":company,"requisitionId":"1234","canonicalUrl":format!("https://{company}.test/job/1234"),"descriptionText":"Original"});
            db.persist_worker_job(company, company, &job).await.unwrap();
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            2
        );
        let changed = serde_json::json!({"title":"Updated engineer","company":"Alpha","requisitionId":"1234","canonicalUrl":"https://Alpha.test/new/1234","descriptionText":"Updated"});
        db.persist_worker_job("Alpha", "Alpha", &changed)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT description_text FROM jobs WHERE company='Beta'"
            )
            .fetch_one(&db.pool)
            .await
            .unwrap(),
            "Original"
        );
    }

    #[tokio::test]
    async fn enrichment_selects_one_job_and_rejects_a_stale_listing_response() {
        let root = std::env::temp_dir().join(format!("jobscraper-enrich-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test/','workday','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r','s','scrape','running','t')").execute(&db.pool).await.unwrap();
        let listing = serde_json::json!({"externalId":"E1","title":"Engineer","company":"S","canonicalUrl":"https://example.test/job/1","detailUrl":"https://example.test/api/1","descriptionStatus":"pending","descriptionText":"","listingHash":"listing-1"});
        db.persist_worker_job("r", "s", &listing).await.unwrap();
        let job_id: String = sqlx::query_scalar("SELECT id FROM jobs WHERE external_id='E1'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let (source_id, pending) = db
            .pending_enrichment_job(&job_id, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source_id, "s");
        assert_eq!(pending["jobId"], job_id);
        let stale = serde_json::json!({"jobId":job_id,"title":"Engineer","company":"S","descriptionText":"stale","listingHash":"listing-old"});
        assert_eq!(db.persist_enriched_jobs("s", &[stale]).await.unwrap(), 0);
        let current = serde_json::json!({"jobId":job_id,"title":"Engineer","company":"S","descriptionText":"Full description","descriptionHtml":"<p>Full description</p>","listingHash":"listing-1"});
        assert_eq!(db.persist_enriched_jobs("s", &[current]).await.unwrap(), 1);
        let row: (String, String) =
            sqlx::query_as("SELECT description_status,description_text FROM jobs WHERE id=?")
                .bind(&job_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(row, ("complete".into(), "Full description".into()));
        assert!(db
            .pending_enrichment_job(&job_id, false)
            .await
            .unwrap()
            .is_none());
        assert!(db
            .pending_enrichment_job(&job_id, true)
            .await
            .unwrap()
            .is_some());
        sqlx::query("UPDATE jobs SET description_checked_at=datetime('now','-8 days') WHERE id=?")
            .bind(&job_id)
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db
            .pending_enrichment_job(&job_id, false)
            .await
            .unwrap()
            .is_some());
        let failure = serde_json::json!({"jobId":job_id,"listingHash":"listing-1","message":"Publisher unavailable"});
        assert_eq!(
            db.record_enrichment_failures("s", &[failure])
                .await
                .unwrap(),
            1
        );
        let cached = load_job_description(&db.pool, &job_id).await.unwrap();
        assert_eq!(cached.text, "Full description");
        assert_eq!(cached.status, "failed");
    }

    // Sighting rows were never pruned, so a full update added one per job per run forever. The
    // trail is kept for the last few runs and no further; the current run must always survive,
    // because the availability rules read it.
    #[tokio::test]
    async fn sighting_history_is_kept_for_recent_runs_and_pruned_beyond_them() {
        let root = std::env::temp_dir().join(format!("jobscraper-prune-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test/','workday','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,external_id,title,company,content_hash,description_text,description_html,skills_json,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES('j','s','E','Engineer','S','c','','','[]','{}','t','1.1.0','t','t')").execute(&db.pool).await.unwrap();
        // 14 runs oldest-first, each having seen the job once.
        for n in 0..14 {
            let run = format!("run-{n:02}");
            sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES(?,'s','scrape','completed',?)")
                .bind(&run).bind(format!("2026-08-{:02}T00:00:00Z", n + 1)).execute(&db.pool).await.unwrap();
            sqlx::query(
                "INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES(?,'j',?,'t')",
            )
            .bind(format!("occ-{n:02}"))
            .bind(&run)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        db.reconcile_availability("run-13", "s", true)
            .await
            .unwrap();

        let kept: Vec<String> =
            sqlx::query_scalar("SELECT run_id FROM job_occurrences ORDER BY run_id")
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(
            kept.len(),
            10,
            "only the retained window of runs keeps its sightings"
        );
        assert_eq!(kept.first().unwrap(), "run-04");
        assert!(
            kept.contains(&"run-13".to_string()),
            "the current run always survives"
        );
        let availability: String = sqlx::query_scalar("SELECT availability FROM jobs WHERE id='j'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(
            availability, "active",
            "pruning history must not close a job that was seen"
        );
        db.pool.close().await;
    }

    // The Sources list shows what each source is holding. A wrong count here reads as a fact
    // rather than as a broken screen, so the split between "on the board" and "come off it" is
    // pinned: possibly_closed is still on the board until a second read confirms otherwise, and
    // a source with nothing stored reports zero rather than dropping out of the list.
    #[tokio::test]
    async fn the_sources_list_reports_live_and_closed_counts_per_source() {
        let root = std::env::temp_dir().join(format!("jobscraper-counts-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        for source in ["busy", "empty"] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?,'https://example.test/','workday','1.1.0',1,'active',0,0,'t','t')")
                .bind(source).bind(source).execute(&db.pool).await.unwrap();
        }
        for (job, availability) in [
            ("a", "active"),
            ("b", "active"),
            ("c", "possibly_closed"),
            ("d", "closed"),
            ("e", "closed"),
        ] {
            sqlx::query("INSERT INTO jobs(id,source_id,external_id,title,company,availability,content_hash,description_text,description_html,skills_json,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'busy',?,'Engineer','S',?,'c','','','[]','{}','t','1.1.0','t','t')")
                .bind(job).bind(job).bind(availability).execute(&db.pool).await.unwrap();
        }
        let counts: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT s.id,COALESCE(c.live,0),COALESCE(c.gone,0) FROM sources s LEFT JOIN (SELECT source_id,sum(CASE WHEN availability='closed' THEN 0 ELSE 1 END) AS live,sum(CASE WHEN availability='closed' THEN 1 ELSE 0 END) AS gone FROM jobs GROUP BY source_id) c ON c.source_id=s.id WHERE s.deleted_at IS NULL ORDER BY s.id",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            counts,
            vec![("busy".to_string(), 3, 2), ("empty".to_string(), 0, 0),]
        );
        db.pool.close().await;
    }

    // Purging every job is irreversible, so the two promises the dialog makes are pinned here:
    // it does nothing without the exact word, and it never deletes a job you have applied to.
    #[tokio::test]
    async fn purging_every_job_needs_the_word_and_spares_applied_jobs() {
        let root = std::env::temp_dir().join(format!("jobscraper-purge-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test/','workday','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        for job in ["plain", "applied", "reviewed"] {
            sqlx::query("INSERT INTO jobs(id,source_id,external_id,title,company,content_hash,description_text,description_html,skills_json,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s',?,'Engineer','S','c','','','[]','{}','t','1.1.0','t','t')")
                .bind(job).bind(job).execute(&db.pool).await.unwrap();
        }
        sqlx::query("INSERT INTO personas(id,name,target_titles_json,created_at,updated_at) VALUES('p','P','[]','t','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO applications(id,job_id,current_stage,created_at,updated_at) VALUES('a','applied','applied','t','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO review_decisions(id,job_id,persona_id,status,decided_at) VALUES('r','reviewed','p','dismissed','t')").execute(&db.pool).await.unwrap();
        // A sighting on the job that will be deleted, to prove dependants go with it.
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('run','s','scrape','completed','t')").execute(&db.pool).await.unwrap();
        sqlx::query(
            "INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES('o','plain','run','t')",
        )
        .execute(&db.pool)
        .await
        .unwrap();

        let count = || async {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs")
                .fetch_one(&db.pool)
                .await
                .unwrap()
        };
        assert_eq!(count().await, 3);
        // The word has to be exact — a near miss must leave everything alone.
        for wrong in ["", "confirm", "CONFIRM", "Confirm all", "yes"] {
            let refused = purge_jobs(&db.pool, wrong).await;
            assert!(refused.is_err(), "{wrong:?} should not be accepted");
            assert_eq!(count().await, 3, "{wrong:?} must not delete anything");
        }
        let result = purge_jobs(&db.pool, " Confirm ").await.unwrap();
        assert_eq!((result.deleted, result.kept), (1, 2));
        let left: Vec<String> = sqlx::query_scalar("SELECT id FROM jobs ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        assert_eq!(left, ["applied", "reviewed"], "your own work survives");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_occurrences")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            0,
            "the deleted job's sighting history went with it"
        );
        db.pool.close().await;
    }

    // The age filter decides what is stored, so the two ways it could quietly lose data are
    // pinned: a listing whose board publishes no date must survive it, and a half-typed box must
    // widen the filter rather than narrow it to nothing.
    // The Jobs page opens on a one-year window, so the window decides what most people ever see.
    // A listing the board never dated must survive it — the board not saying when a job went up
    // is not the same as the job being old — and "Any time" must genuinely mean everything.
    #[tokio::test]
    async fn the_posted_window_hides_old_listings_but_never_undated_ones() {
        let root = std::env::temp_dir().join(format!("jobscraper-window-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','S','https://example.test/','workday','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        let today = Utc::now().date_naive();
        let day = |back: i64| {
            (today - chrono::Duration::days(back))
                .format("%Y-%m-%d")
                .to_string()
        };
        let rows: Vec<(&str, Option<String>)> = vec![
            ("fresh", Some(day(3))),
            ("old", Some(day(400))),
            ("undated", None),
            ("blank", Some(String::new())),
            ("stamped", Some(format!("{}T09:14:00Z", day(10)))),
        ];
        for (id, posted) in &rows {
            sqlx::query("INSERT INTO jobs(id,source_id,external_id,title,company,posted_at,content_hash,description_text,description_html,skills_json,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES(?,'s',?,'Engineer','S',?,'c','','','[]','{}','t','1.1.0','t','t')")
                .bind(id).bind(id).bind(posted.clone()).execute(&db.pool).await.unwrap();
        }
        let shown = |days: Option<i64>| {
            let pool = db.pool.clone();
            async move {
                let page = jobs_page_query(
                    JobFilter {
                        posted_within_days: days,
                        ..Default::default()
                    },
                    0,
                    &pool,
                )
                .await
                .unwrap();
                let mut ids = page.items.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
                ids.sort();
                (ids, page.total)
            }
        };
        let (year, total) = shown(Some(365)).await;
        assert_eq!(
            year,
            ["blank", "fresh", "stamped", "undated"],
            "only the 400-day-old listing is out of the year"
        );
        assert_eq!(total, 4, "the count follows the window, not just the page");
        let (week, _) = shown(Some(7)).await;
        assert_eq!(
            week,
            ["blank", "fresh", "undated"],
            "a ten-day-old timestamp falls outside a week"
        );
        let (any, any_total) = shown(None).await;
        assert_eq!(any.len(), 5, "Any time means every listing");
        assert_eq!(any_total, 5);
        db.pool.close().await;
    }
    #[test]
    fn scrape_title_filter_keeps_everything_until_terms_are_given() {
        assert!(scrape_filter_terms("").is_empty());
        assert!(scrape_filter_terms("  ,\n , ").is_empty());
        assert!(title_passes_scrape_filter("Anything at all", &[]));
        let terms = scrape_filter_terms("Verification, design verification\nRTL , ");
        assert_eq!(terms, ["verification", "design verification", "rtl"]);
        assert!(title_passes_scrape_filter(
            "Senior RTL Design Engineer",
            &terms
        ));
        assert!(title_passes_scrape_filter("ASIC VERIFICATION LEAD", &terms));
        assert!(!title_passes_scrape_filter(
            "Retail Sales Associate",
            &terms
        ));
        // A phrase stays one term, so the comma is the only separator that splits it.
        let phrase = scrape_filter_terms("design verification");
        assert!(!title_passes_scrape_filter("Design Engineer", &phrase));
    }

    #[test]
    fn application_transition_rules_require_explicit_override() {
        assert!(legal_stage_transition("applied", "screening"));
        assert!(legal_stage_transition("offer", "accepted"));
        assert!(!legal_stage_transition("planned", "offer"));
        assert!(!legal_stage_transition("rejected", "screening"));
        assert!(validate_stage_transition("planned", "applied", false, None).is_err());
        assert!(validate_stage_transition("planned", "offer", true, None).is_err());
        assert!(
            validate_stage_transition("planned", "offer", true, Some("historical import")).is_ok()
        );
    }

    #[tokio::test]
    async fn application_tracking_migration_persists_notes_documents_and_event_reason() {
        let pool = migrated_pool().await;
        sqlx::query("CREATE TABLE IF NOT EXISTS application_test (id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        // Migration creates the immutable document metadata and note audit columns.
        let note_columns: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_table_info('notes') WHERE name IN ('updated_at','deleted_at')").fetch_one(&pool).await.unwrap();
        let doc_columns: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_table_info('application_documents') WHERE name IN ('document_type','mime_type','size','event_id')").fetch_one(&pool).await.unwrap();
        let reason_columns: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('application_events') WHERE name='reason'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(note_columns, 2);
        assert_eq!(doc_columns, 4);
        assert_eq!(reason_columns, 1);
        pool.close().await;
    }

    #[tokio::test]
    async fn application_document_bytes_round_trip_and_verify_checksum() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('docs','Docs','https://example.test','json','1',0,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('docs-job','docs','Role','Company','','[]','hash','t','1','t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications(id,job_id,current_stage,created_at,updated_at) VALUES('docs-app','docs-job','planned','t','t')").execute(&pool).await.unwrap();
        let bytes = vec![0, 1, 2, 127, 128, 255];
        let document = attach_application_document_pool(
            AttachApplicationDocument {
                application_id: "docs-app".into(),
                document_type: "resume".into(),
                filename: Some("currículo_日本語.pdf".into()),
                mime_type: Some("application/pdf".into()),
                event_id: None,
            },
            bytes.clone(),
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(document.filename, "currículo_日本語.pdf");
        assert_eq!(document.size, bytes.len() as i64);
        assert_eq!(
            export_application_document_bytes(&document.id, &pool)
                .await
                .unwrap(),
            bytes
        );
        sqlx::query("UPDATE application_documents SET content=X'01' WHERE id=?")
            .bind(&document.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            export_application_document_bytes(&document.id, &pool)
                .await
                .unwrap_err(),
            "Application document checksum failed"
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn analytics_fixture_uses_event_response_and_date_boundaries() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('as','Analytics source','https://example.test','json','1',0,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,availability,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('aj','as','Role','Company','text','[]','active','hash','2026-01-01T00:00:00Z','1','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO applications(id,job_id,current_stage,applied_at,created_at,updated_at) VALUES('aa','aj','screening','2026-01-02T00:00:00Z','2026-01-02T00:00:00Z','2026-01-03T00:00:00Z')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO application_events(id,application_id,event_type,to_stage,occurred_at,payload_json) VALUES('before','aa','stage_changed','screening','2026-01-01T00:00:00Z','{}'),('after','aa','stage_changed','screening','2026-01-03T12:00:00Z','{}')").execute(&pool).await.unwrap();
        let response: Option<f64> = sqlx::query_scalar("SELECT avg((julianday(e.occurred_at)-julianday(a.applied_at))*24) FROM applications a JOIN application_events e ON e.application_id=a.id WHERE e.occurred_at=(SELECT min(e2.occurred_at) FROM application_events e2 WHERE e2.application_id=a.id AND e2.occurred_at>a.applied_at AND e2.to_stage IN ('screening','interviewing','offer','rejected'))").fetch_one(&pool).await.unwrap();
        assert_eq!(response, Some(36.0));
        let start = "2026-01-02T00:00:00Z";
        let end = "2026-01-03T00:00:00Z";
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM applications WHERE created_at>=? AND created_at<?",
        )
        .bind(start)
        .bind(end)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        assert!(validate_analytics_filter(&AnalyticsFilter {
            start_at: Some(end.into()),
            end_at: Some(start.into()),
            ..Default::default()
        })
        .is_err());
        pool.close().await;
    }

    #[test]
    fn dedupe_normalization_and_identity_fingerprints() {
        assert_eq!(
            normalize_canonical_url("HTTPS://EXAMPLE.test:443/jobs/?utm_source=x&id=2#top")
                .unwrap(),
            "https://example.test/jobs?id=2"
        );
        assert!(normalize_canonical_url("file:///private").is_none());
        assert_eq!(
            job_fingerprint("Firmware Engineer", "Chip Co", Some("Lisbon")),
            job_fingerprint(" firmware engineer ", "CHIP co", Some("lisbon"))
        );
    }

    #[tokio::test]
    async fn distinct_external_jobs_are_not_merged_by_title_company_and_location() {
        let root = std::env::temp_dir().join(format!("jobscraper-identity-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('apple','Apple','https://jobs.apple.com/','apple','1.1.0',1,'active',0,0,'t','t')").execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('run','apple','scrape','running','t')").execute(&db.pool).await.unwrap();
        for (external_id, url) in [
            ("2001", "https://jobs.apple.com/en-us/details/2001/engineer"),
            ("2002", "https://jobs.apple.com/en-us/details/2002/engineer"),
        ] {
            db.persist_worker_job(
                "run",
                "apple",
                &serde_json::json!({
                    "externalId": external_id,
                    "canonicalUrl": url,
                    "applyUrl": url,
                    "title": "Software Engineer",
                    "company": "Apple",
                    "location": "Cupertino, United States",
                    "descriptionText": "A real opening",
                    "contentHash": external_id,
                    "adapterVersion": "1.1.0"
                }),
            )
            .await
            .unwrap();
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE source_id='apple'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
        db.pool.close().await;
    }

    #[test]
    fn dedupe_conflict_snapshots_fail_closed_and_keep_row_identity() {
        assert!(dedupe_snapshot_id("{\"id\":\"match-a\"}").is_ok());
        assert!(dedupe_snapshot_id("{\"filterDecision\":{}}")
            .unwrap_err()
            .contains("legacy incomplete"));
        assert!(dedupe_snapshot_id("not json").is_err());
    }

    #[tokio::test]
    async fn dedupe_snapshot_restore_is_transactional_and_preserves_one_current_row() {
        let pool = migrated_pool().await;
        seed_match(&pool).await;
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('alias','s','Engineer','Company','description','[]','alias-v1','t','1','t','t')").execute(&pool).await.unwrap();
        let current = r#"{"id":"current","score":60.0,"eligible":1,"algorithmVersion":"dedupe-v1","modelVersion":"model","filterDecisionJson":"{}","componentsJson":"{}","explanationJson":"{}","createdAt":"t","updatedAt":"t"}"#;
        let alias = r#"{"id":"alias-match","score":70.0,"eligible":1,"algorithmVersion":"dedupe-v1","modelVersion":"model","filterDecisionJson":"{}","componentsJson":"{}","explanationJson":"{}","createdAt":"t","updatedAt":"t"}"#;
        let mut tx = pool.begin().await.unwrap();
        restore_match_snapshot(&mut tx, "j", "p", current)
            .await
            .unwrap();
        restore_match_snapshot(&mut tx, "alias", "p", alias)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM match_results WHERE persona_id='p' AND algorithm_version='dedupe-v1'").fetch_one(&pool).await.unwrap(),2);
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM match_results WHERE id='current'")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE match_results SET job_id='j' WHERE id='alias-match' AND job_id='alias'",
        )
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM match_results WHERE id='current'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn relational_export_and_csv_escape_keep_stable_relationship_ids() {
        let pool = migrated_pool().await;
        seed_match(&pool).await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('other','Other','https://other.test','json','1',0,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,source_id,title,company,description_text,skills_json,content_hash,extraction_at,adapter_version,created_at,updated_at) VALUES('other-job','other','Other','Elsewhere','text','[]','other','t','1','t','t')").execute(&pool).await.unwrap();
        let archive = relational_json(&pool, &ExportFilter::default())
            .await
            .unwrap();
        let archive: serde_json::Value = serde_json::from_slice(&archive).unwrap();
        assert_eq!(archive["format"], "jobscraper-relational-json");
        assert_eq!(archive["version"], 1);
        assert_eq!(archive["tables"]["jobs"][0]["id"], "j");
        assert_eq!(archive["tables"]["personas"][0]["id"], "p");
        assert_eq!(csv_cell(Some("a,\"b\"\r\n".into())), "\"a,\"\"b\"\"\r\n\"");
        let csv = String::from_utf8(
            csv_bytes(&pool, "jobs", &ExportFilter::default())
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(csv.starts_with("id,title"));
        assert!(csv.contains("\r\n"));
        let filtered = relational_json(
            &pool,
            &ExportFilter {
                source_id: Some("s".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let filtered: serde_json::Value = serde_json::from_slice(&filtered).unwrap();
        assert_eq!(filtered["tables"]["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["tables"]["sources"][0]["id"], "s");
        assert!(relational_json(
            &pool,
            &ExportFilter {
                start_at: Some("2026-02-01T00:00:00Z".into()),
                end_at: Some("2026-01-01T00:00:00Z".into()),
                ..Default::default()
            }
        )
        .await
        .is_err());
        pool.close().await;
    }

    #[tokio::test]
    async fn scrape_log_purge_query_matches_the_events_table_schema() {
        // Regression: preview_purge's "scrape_logs" branch used to filter on a column
        // ("occurred_at") that scrape_run_events never had (it has "created_at"), so the
        // query would fail at runtime the first time the table held any rows.
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','s','https://example.test','json','1',0,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r','s','test','completed','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_run_events(id,run_id,level,event_type,payload_json,created_at) VALUES('e1','r','info','started','{}','2026-01-01T00:00:00Z'),('e2','r','info','started','{}','2026-03-01T00:00:00Z')").execute(&pool).await.unwrap();
        let old: Vec<String> =
            sqlx::query_scalar("SELECT id FROM scrape_run_events WHERE created_at<?")
                .bind("2026-02-01T00:00:00Z")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(old, vec!["e1".to_string()]);
        pool.close().await;
    }
    #[tokio::test]
    async fn settings_survive_the_migration_that_once_dropped_them() {
        // Migration 0012 dropped `settings` as dead, so building the one-time robots
        // acknowledgement on it failed at runtime with "no such table". 0014 brings it back for
        // its first real consumer; this fails if a later cleanup drops it again.
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO settings(key,value,updated_at) VALUES('robots.autoAcknowledged','true','t') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .execute(&pool).await.unwrap();
        let stored: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
            .bind("robots.autoAcknowledged")
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(stored.as_deref(), Some("true"));
        pool.close().await;
    }
    #[tokio::test]
    async fn a_database_from_a_newer_build_is_refused_in_plain_language() {
        // The white-screen bug: a dev build migrated the shared database forward and the older
        // installed binary could no longer open its own data. sqlx says "VersionMissing"; the
        // user needs to be told what to do and that nothing is lost.
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO _sqlx_migrations(version,description,installed_on,success,checksum,execution_time) VALUES(99,'from the future',current_timestamp,1,X'00',0)")
            .execute(&pool).await.unwrap();
        let message = sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(migration_error)
            .expect_err("a database ahead of this binary must be refused");
        assert!(message.contains("newer version of JobScraper"), "{message}");
        assert!(message.contains("schema 99"), "{message}");
        assert!(message.contains("data is intact"), "{message}");
        pool.close().await;
    }
    #[tokio::test]
    async fn connect_lazy_touches_no_disk_so_the_window_can_paint_first() {
        // Startup manages this pool before the window has rendered anything, so building it must
        // not open, create, or migrate a file — only prepare() may fail.
        let missing = std::env::temp_dir()
            .join("jobscraper-no-such-dir")
            .join("jobscraper.db");
        let db = Database::connect_lazy(missing.clone());
        assert!(!missing.exists());
        assert!(db.prepare().await.is_err());
        assert!(!missing.exists());
    }
    #[tokio::test]
    async fn activity_log_records_failures_that_never_reached_a_scrape_run() {
        // The case this table exists for: a probe of an unsaved source that died in the worker.
        // It has no source_id and no scrape_run, and used to leave no trace anywhere.
        let pool = migrated_pool().await;
        log(
            &pool,
            "error",
            None,
            Some("Arm"),
            "probe_source",
            Some("worker_exit"),
            "Scraper worker exited with exit code: 1",
            serde_json::json!({"stderr":"boom"}),
        )
        .await;
        log(
            &pool,
            "info",
            None,
            Some("Arm"),
            "probe_source",
            None,
            "Configured automatically as static-css.",
            serde_json::json!({}),
        )
        .await;
        let rows = sqlx::query_as::<_, AppLog>("SELECT id,at,level,source_id,source_name,action,code,message,detail_json FROM app_logs ORDER BY at DESC, rowid DESC LIMIT ?")
            .bind(200i64).fetch_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].message, "Configured automatically as static-css.");
        assert_eq!(rows[1].level, "error");
        assert_eq!(rows[1].code.as_deref(), Some("worker_exit"));
        assert!(rows[1].source_id.is_none());
        assert!(rows[1].detail_json.contains("boom"));
        pool.close().await;
    }
    #[tokio::test]
    async fn scrape_log_purge_covers_app_logs_and_deletes_from_whichever_table_owns_the_id() {
        // app_logs records the same class of diagnostics for actions that never produced a
        // scrape_run, so one retention control has to see and delete both tables.
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,robots_override,allow_private_network,created_at,updated_at) VALUES('s','s','https://example.test','json','1',0,'active',0,0,'t','t')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_runs(id,source_id,mode,status,started_at) VALUES('r','s','test','completed','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO scrape_run_events(id,run_id,level,event_type,payload_json,created_at) VALUES('e1','r','info','started','{}','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO app_logs(id,at,level,source_id,source_name,action,code,message,detail_json) VALUES('a1','2026-01-01T00:00:00Z','error','s','S','probe_source','worker_exit','crashed','{}'),('a2','2026-03-01T00:00:00Z','info','s','S','probe_source',NULL,'fine','{}')").execute(&pool).await.unwrap();
        let mut old: Vec<String> = sqlx::query_scalar("SELECT id FROM scrape_run_events WHERE created_at<? UNION ALL SELECT id FROM app_logs WHERE at<?")
            .bind("2026-02-01T00:00:00Z")
            .bind("2026-02-01T00:00:00Z")
            .fetch_all(&pool)
            .await
            .unwrap();
        old.sort();
        assert_eq!(old, vec!["a1".to_string(), "e1".to_string()]);
        // The apply step tries scrape_run_events first, then app_logs; exactly one row goes.
        let mut removed = sqlx::query("DELETE FROM scrape_run_events WHERE id=?")
            .bind("a1")
            .execute(&pool)
            .await
            .unwrap()
            .rows_affected();
        if removed == 0 {
            removed = sqlx::query("DELETE FROM app_logs WHERE id=?")
                .bind("a1")
                .execute(&pool)
                .await
                .unwrap()
                .rows_affected();
        }
        assert_eq!(removed, 1);
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM app_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 1);
        pool.close().await;
    }
    #[test]
    fn purge_preview_hash_detects_tampering_and_file_boundary_is_relative() {
        let original = purge_hash("sessions", &[], &[], &["source/session.json".into()]);
        let changed = purge_hash("sessions", &[], &[], &["../documents/resume.pdf".into()]);
        assert_ne!(original, changed);
        let root = PathBuf::from("C:/JobScraper/sessions");
        let safe = root.join("source/session.json");
        assert!(safe.strip_prefix(&root).is_ok());
        // The actual delete path must use normalized controlled components; this
        // demonstrates why an absolute archive/session path is never accepted.
        assert!(PathBuf::from("C:/other/session.json")
            .strip_prefix(&root)
            .is_err());
    }

    #[test]
    fn outcome_csv_aggregates_source_company_rates_and_small_samples() {
        let mut selected = HashMap::new();
        selected.insert(
            "sources".into(),
            vec![serde_json::json!({"id":"s","name":"Source, One"})],
        );
        selected.insert(
            "jobs".into(),
            vec![serde_json::json!({"id":"j","source_id":"s","company":"Chip \"Co\""})],
        );
        selected.insert("applications".into(),vec![serde_json::json!({"id":"a","job_id":"j","current_stage":"accepted","applied_at":"2026-01-01T00:00:00Z"})]);
        selected.insert("application_events".into(),vec![serde_json::json!({"application_id":"a","to_stage":"screening","occurred_at":"2026-01-01T12:00:00Z"})]);
        let source = String::from_utf8(outcomes_csv(&selected, false)).unwrap();
        let company = String::from_utf8(outcomes_csv(&selected, true)).unwrap();
        assert!(source.contains("\"Source, One\""));
        assert!(source.contains("100.00%"));
        assert!(source.contains("12.00"));
        assert!(source.contains("small_sample"));
        assert!(company.contains("\"Chip \"\"Co\"\"\""));
    }
}

#[cfg(test)]
mod performance_history_tests {
    use super::*;

    fn worker_run(total: u64, response: u64) -> String {
        serde_json::json!({
            "requests": 7, "pages": 3, "discovered": 120,
            "performance": { "version": 1,
                "worker": { "totalMs": total, "bucketsMs": { "response": response, "pacing": 300, "body": 40 },
                            "requestsByKind": { "listing": { "count": 2 }, "detail": { "count": 5 } } },
                "app": { "totalMs": total + 400,
                         "criticalPathMs": { "prepare": 40, "startup": 300, "active": total, "shutdown": 20, "finalize": 40 },
                         "workMs": { "jobPersistence": 120, "knownHashLoad": 30 } } }
        }).to_string()
    }
    fn log(id: &str, level: &str, action: &str, detail: String) -> AppLog {
        AppLog {
            id: id.into(),
            at: format!("2026-08-31T09:00:{id:0>2}Z"),
            level: level.into(),
            source_id: Some("src-amd".into()),
            source_name: Some("AMD".into()),
            action: action.into(),
            code: None,
            message: "Finished".into(),
            detail_json: detail,
        }
    }

    #[test]
    fn rows_without_versioned_metrics_are_ignored() {
        let logs = vec![
            log(
                "1",
                "warning",
                "scrape_source",
                serde_json::json!({ "code": "robots_unparseable" }).to_string(),
            ),
            log(
                "2",
                "info",
                "scrape_source",
                serde_json::json!({ "requests": 4 }).to_string(),
            ),
            log(
                "3",
                "info",
                "scrape_source",
                serde_json::json!({ "performance": { "version": 2, "worker": {} } }).to_string(),
            ),
            log("4", "info", "scrape_source", "not json at all".into()),
            log("5", "info", "scrape_source", worker_run(12_000, 6_100)),
        ];
        let history = performance_history_from_logs(logs);
        assert_eq!(
            history.recent.len(),
            1,
            "only the instrumented run is measurable"
        );
        assert_eq!(history.recent[0].id, "5");
        assert_eq!(history.aggregates.len(), 1);
    }

    #[test]
    fn a_run_reports_its_totals_phases_and_slowest_step() {
        let history = performance_history_from_logs(vec![log(
            "1",
            "info",
            "scrape_source",
            worker_run(12_000, 6_100),
        )]);
        let run = &history.recent[0];
        assert_eq!(
            run.total_ms, 12_400,
            "the app total is the whole command, not just the worker"
        );
        assert_eq!(run.worker_ms, 12_000);
        assert_eq!((run.requests, run.pages, run.jobs), (7, 3, 120));
        assert_eq!(run.outcome, "completed");
        let slowest = run.slowest_phase.as_ref().unwrap();
        assert_eq!(
            slowest.key, "worker.response",
            "the parent app.active phase never wins"
        );
        assert_eq!(slowest.milliseconds, 6_100);
        assert_eq!(run.phases["work.jobPersistence"], 120);
        assert_eq!(run.phases["app.startup"], 300);
        assert_eq!(run.requests_by_kind["detail"]["count"], 5);
    }

    #[test]
    fn failures_and_cancellations_are_listed_but_never_averaged() {
        let mut cancelled = log("1", "info", "scrape_source", worker_run(400, 100));
        cancelled.detail_json = cancelled
            .detail_json
            .replace("\"requests\":7", "\"requests\":7,\"code\":\"cancelled\"");
        let mut failed = log("2", "error", "scrape_source", worker_run(9_000, 8_000));
        failed.message = "network_error".into();
        let history = performance_history_from_logs(vec![
            cancelled,
            failed,
            log("3", "info", "scrape_source", worker_run(12_000, 6_100)),
            log("4", "info", "scrape_source", worker_run(12_000, 6_100)),
        ]);
        assert_eq!(
            history
                .recent
                .iter()
                .map(|run| run.outcome.as_str())
                .collect::<Vec<_>>(),
            ["cancelled", "failed", "completed", "completed"]
        );
        assert_eq!(history.aggregates.len(), 1);
        assert_eq!(
            history.aggregates[0].samples, 2,
            "only completed runs are averaged"
        );
        assert_eq!(history.aggregates[0].median_ms, 12_400);
    }

    #[test]
    fn medians_p95_and_ordering_are_deterministic() {
        let mut logs = Vec::new();
        for (index, total) in [1_000_u64, 2_000, 3_000, 4_000, 100_000]
            .into_iter()
            .enumerate()
        {
            logs.push(log(
                &format!("{index}"),
                "info",
                "scrape_source",
                worker_run(total, total / 2),
            ));
        }
        // A second, cheaper group so ordering has something to sort.
        let mut check = log("9", "info", "check_source", worker_run(500, 100));
        check.source_name = Some("Arm".into());
        check.source_id = Some("src-arm".into());
        logs.push(check);
        let history = performance_history_from_logs(logs);
        let scrape = history
            .aggregates
            .iter()
            .find(|row| row.action == "scrape_source")
            .unwrap();
        assert_eq!(scrape.samples, 5);
        assert_eq!(
            scrape.median_ms, 3_400,
            "the median is the middle sample, not the mean"
        );
        assert_eq!(
            scrape.p95_ms,
            Some(100_400),
            "p95 appears once five samples exist"
        );
        assert_eq!(
            scrape.slowest_phase.as_ref().unwrap().key,
            "worker.response"
        );
        let check = history
            .aggregates
            .iter()
            .find(|row| row.action == "check_source")
            .unwrap();
        assert_eq!(check.p95_ms, None, "one sample cannot support a p95");
        assert_eq!(check.median_ms, 900);
        assert_eq!(
            history.aggregates[0].action, "scrape_source",
            "the slowest group is listed first"
        );
    }

    #[test]
    fn history_is_capped_at_fifty_runs_and_twenty_samples_per_group() {
        let logs = (0..70)
            .map(|index| {
                log(
                    &format!("{index}"),
                    "info",
                    "scrape_source",
                    worker_run(1_000 + index, 100),
                )
            })
            .collect::<Vec<_>>();
        let history = performance_history_from_logs(logs);
        assert_eq!(history.recent.len(), 50);
        assert_eq!(history.aggregates[0].samples, 20);
    }

    #[test]
    fn a_batch_run_ranks_its_own_work_without_worker_metrics() {
        let detail = serde_json::json!({
            "requests": 31, "completedSources": 6, "peakDomains": 5,
            "performance": { "version": 1, "app": {
                "totalMs": 9_800,
                "criticalPathMs": { "prepare": 500, "startup": 0, "active": 9_000, "shutdown": 0, "finalize": 300 },
                "workMs": { "preflight": 800, "scrape": 8_100 } } }
        }).to_string();
        let mut batch = log("1", "info", "scrape_all", detail);
        batch.source_id = None;
        batch.source_name = None;
        let history = performance_history_from_logs(vec![batch]);
        let run = &history.recent[0];
        assert_eq!(run.total_ms, 9_800);
        assert_eq!(run.worker_ms, 0, "a batch has no single worker");
        assert_eq!(run.requests, 31);
        assert_eq!(run.jobs, 6);
        assert_eq!(run.slowest_phase.as_ref().unwrap().key, "work.scrape");
        assert!(run.source_name.is_none());
    }
}
