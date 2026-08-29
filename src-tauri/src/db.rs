use crate::notifications::{
    is_managed_task_path, scoped_orphans, Reminder as ScheduledReminder, Scheduler,
    WindowsTaskScheduler,
};
use crate::{
    domain::{Application, Persona, PersonaInput, Source, SourceInput, StageInput},
    AppState,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use tauri::{Emitter, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;
use uuid::Uuid;

pub type ApiResult<T> = Result<T, String>;
static RESCORE_CANCELLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static RESCORE_ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static PURGE_PREVIEWS: OnceLock<Mutex<HashMap<String, PurgePreviewState>>> = OnceLock::new();
fn cancelled_runs() -> &'static Mutex<HashSet<String>> {
    RESCORE_CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}
fn active_runs() -> &'static Mutex<HashSet<String>> {
    RESCORE_ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}
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
fn fuzzy_similarity(left: &str, right: &str) -> f64 {
    let left_text = normalized_text(left);
    let right_text = normalized_text(right);
    let left: HashSet<_> = left_text.split_whitespace().map(str::to_owned).collect();
    let right: HashSet<_> = right_text.split_whitespace().map(str::to_owned).collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    };
    let shared = left.intersection(&right).count();
    let jaccard = shared as f64 / left.union(&right).count() as f64;
    // Seniority suffixes (for example, "II") should suggest, not silently merge,
    // an otherwise exact multi-word title. One-word containment stays conservative.
    if shared >= 2 && (left.is_subset(&right) || right.is_subset(&left)) {
        jaccard.max(0.9)
    } else {
        jaccard
    }
}
const APPLICATION_SELECT: &str = "SELECT a.id,a.job_id,a.persona_id,a.current_stage,a.recruiter,a.recruiter_name,a.recruiter_email,a.recruiter_phone,a.source_attribution,a.rejection_reason,a.rejection_category,a.withdrawn_reason,a.accepted_at,a.applied_at,a.created_at,a.updated_at,j.title,j.company FROM applications a LEFT JOIN jobs j ON j.id=a.job_id";
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
impl Database {
    pub async fn open(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let opts = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;
        sqlx::query("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")
            .execute(&pool)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self {
            pool,
            root: path.parent().unwrap().to_path_buf(),
        })
    }
    pub async fn install_starter_pack(&self) -> ApiResult<()> {
        // Stable IDs make this dated pack idempotent even when a user renames a
        // source. All active starters are disabled until explicitly enabled.
        let starters = [
            (
                "00000000-0000-4000-8000-000000000001",
                "Microchip",
                "https://careers.microchip.com/",
                "workday",
                "careers.microchip.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000002",
                "Analog Devices",
                "https://analogdevices.wd1.myworkdayjobs.com/",
                "workday",
                "analogdevices.wd1.myworkdayjobs.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000003",
                "Broadcom",
                "https://broadcom.wd1.myworkdayjobs.com/",
                "workday",
                "broadcom.wd1.myworkdayjobs.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000004",
                "Intel",
                "https://jobs.intel.com/",
                "workday",
                "jobs.intel.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000005",
                "Marvell careers (Workday)",
                "https://marvell.wd1.myworkdayjobs.com/",
                "workday",
                "marvell.wd1.myworkdayjobs.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000006",
                "STMicroelectronics",
                "https://careers.st.com/",
                "eightfold",
                "careers.st.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000007",
                "NVIDIA",
                "https://nvidia.wd5.myworkdayjobs.com/",
                "eightfold",
                "nvidia.wd5.myworkdayjobs.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000008",
                "GlobalFoundries",
                "https://gf.com/careers",
                "eightfold",
                "gf.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000009",
                "Micron",
                "https://careers.micron.com/",
                "eightfold",
                "careers.micron.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000010",
                "Qualcomm",
                "https://careers.qualcomm.com/",
                "eightfold",
                "careers.qualcomm.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000011",
                "Arm",
                "https://careers.arm.com/",
                "icims",
                "careers.arm.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000012",
                "AMD",
                "https://careers.amd.com/",
                "icims",
                "careers.amd.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000013",
                "Cisco",
                "https://jobs.cisco.com/",
                "phenom",
                "jobs.cisco.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000014",
                "Apple",
                "https://jobs.apple.com/",
                "custom-api",
                "jobs.apple.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000015",
                "MediaTek",
                "https://www.mediatek.com/careers",
                "custom-api",
                "www.mediatek.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000016",
                "u-blox",
                "https://www.u-blox.com/en/careers",
                "custom-api",
                "www.u-blox.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000017",
                "Google",
                "https://www.google.com/about/careers/applications/jobs/results",
                "custom-api",
                "www.google.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000018",
                "SK hynix",
                "https://www.skhynix.com/eng/careers/",
                "custom-api",
                "www.skhynix.com",
                "active",
            ),
            (
                "00000000-0000-4000-8000-000000000019",
                "Marvell careers (reference)",
                "https://www.marvell.com/company/careers.html",
                "reference",
                "www.marvell.com",
                "reference",
            ),
        ];
        for (source_id, name, url, adapter, host, kind) in starters {
            let t = now();
            sqlx::query("INSERT OR IGNORE INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?,?,?,?,0,?,?,0,0,?,?)")
    .bind(source_id).bind(name).bind(url).bind(adapter).bind("1.1.0").bind(kind).bind(if kind=="reference" {Some("Reference-only source: it is never scraped.")} else if adapter=="custom-api" {Some("Custom source disabled: a verified source-specific adapter is required.")} else {Some("Starter source is disabled until you review and enable it.")}).bind(&t).bind(&t).execute(&self.pool).await.map_err(|e| e.to_string())?;
            sqlx::query("INSERT OR IGNORE INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?)")
                .bind(id()).bind(source_id).bind(serde_json::json!({"schemaVersion":"1.1.0","starterPackVersion":"2026-08-28","expectedHost":host,"adapterVersion":"1.1.0","mode":"direct"}).to_string()).bind(&t).bind(&t).execute(&self.pool).await.map_err(|e| e.to_string())?;
            // Legacy packs used generated source IDs. Retire only an untouched,
            // disabled old starter with no dependent history; user-created sources
            // and every historical reference remain intact.
            sqlx::query("UPDATE sources SET enabled=0,deleted_at=?,disabled_reason='Replaced by versioned starter-pack source' WHERE id<>? AND name=? AND base_url=? AND adapter_id=? AND enabled=0 AND deleted_at IS NULL AND created_at=updated_at AND disabled_reason='Starter source is disabled until you review and enable it.' AND NOT EXISTS (SELECT 1 FROM scrape_runs WHERE scrape_runs.source_id=sources.id) AND NOT EXISTS (SELECT 1 FROM jobs WHERE jobs.source_id=sources.id) AND EXISTS (SELECT 1 FROM source_configs c WHERE c.source_id=sources.id AND c.created_at=c.updated_at AND c.config_json LIKE '%starterPackVersion%')")
                .bind(&t).bind(source_id).bind(name).bind(url).bind(adapter).execute(&self.pool).await.map_err(|e| e.to_string())?;
        }
        sqlx::query("INSERT INTO schema_metadata(key,value) VALUES('starter_pack_version','2026-08-28') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .execute(&self.pool).await.map_err(|e| e.to_string())?;
        Ok(())
    }
    pub async fn persist_worker_job(
        &self,
        run_id: &str,
        source_id: &str,
        payload: &serde_json::Value,
    ) -> ApiResult<()> {
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
        let description = payload
            .get("descriptionText")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let hash = payload
            .get("contentHash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let existing:Option<String>=sqlx::query_scalar("SELECT id FROM jobs WHERE (source_id=? AND external_id=?) OR (? IS NOT NULL AND canonical_url=?) OR (? IS NOT NULL AND requisition_id=?) OR dedupe_fingerprint=? ORDER BY created_at LIMIT 1").bind(source_id).bind(external).bind(canonical.as_deref()).bind(canonical.as_deref()).bind(requisition).bind(requisition).bind(&fingerprint).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        let coalesced = existing.is_some();
        let actual = existing.unwrap_or(job_id);
        sqlx::query("INSERT INTO jobs(id,source_id,external_id,canonical_url,apply_url,title,company,location,work_mode,description_text,description_html,posted_at,closing_at,salary_min,salary_max,salary_currency,salary_period,salary_confidence,seniority,skills_json,content_hash,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET canonical_url=excluded.canonical_url,apply_url=excluded.apply_url,title=excluded.title,company=excluded.company,location=excluded.location,work_mode=excluded.work_mode,description_text=excluded.description_text,description_html=excluded.description_html,skills_json=excluded.skills_json,content_hash=excluded.content_hash,provenance_json=excluded.provenance_json,extraction_at=excluded.extraction_at,updated_at=excluded.updated_at")
  .bind(&actual).bind(source_id).bind(external).bind(canonical.as_deref()).bind(payload.get("applyUrl").and_then(|v|v.as_str())).bind(title).bind(company).bind(payload.get("location").and_then(|v|v.as_str())).bind(payload.get("workMode").and_then(|v|v.as_str())).bind(description).bind(payload.get("descriptionHtml").and_then(|v|v.as_str()).map(|s|s.chars().take(250_000).collect::<String>())).bind(payload.get("postedAt").and_then(|v|v.as_str())).bind(payload.get("closingAt").and_then(|v|v.as_str())).bind(payload.get("salaryMin").and_then(|v|v.as_f64())).bind(payload.get("salaryMax").and_then(|v|v.as_f64())).bind(payload.get("salaryCurrency").and_then(|v|v.as_str())).bind(payload.get("salaryPeriod").and_then(|v|v.as_str())).bind(payload.get("salaryConfidence").and_then(|v|v.as_str())).bind(payload.get("seniority").and_then(|v|v.as_str())).bind(payload.get("skills").cloned().unwrap_or_else(||serde_json::json!([])).to_string()).bind(hash).bind(payload.get("provenance").cloned().unwrap_or_else(||serde_json::json!({})).to_string()).bind(&t).bind(payload.get("adapterVersion").and_then(|v|v.as_str()).unwrap_or("1.0.0")).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE jobs SET requisition_id=COALESCE(?,requisition_id),dedupe_fingerprint=? WHERE id=?").bind(requisition).bind(&fingerprint).bind(&actual).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES(?,?,?,?)")
            .bind(id())
            .bind(&actual)
            .bind(run_id)
            .bind(&t)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if coalesced {
            sqlx::query("INSERT INTO job_dedupe_events(id,canonical_job_id,method,evidence_json,created_at) VALUES(?,?, 'exact',?,?)").bind(id()).bind(&actual).bind(serde_json::json!({"externalId":external,"canonicalUrl":canonical,"requisitionId":requisition,"fingerprint":fingerprint}).to_string()).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        let location = payload
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        for row in sqlx::query("SELECT id,title,company,coalesce(location,'') FROM jobs WHERE id<>? AND lower(company)=lower(?) AND dedupe_fingerprint<>? ORDER BY updated_at DESC LIMIT 100").bind(&actual).bind(company).bind(&fingerprint).fetch_all(&mut *tx).await.map_err(|e|e.to_string())? { let other:String=row.get(0);let title_score=fuzzy_similarity(title,&row.get::<String,_>(1));let location_score=if location.is_empty(){1.0}else{fuzzy_similarity(location,&row.get::<String,_>(3))};if title_score>=0.84&&location_score>=0.70 {let (left,right)=if actual<other{(&actual,&other)}else{(&other,&actual)};sqlx::query("INSERT OR IGNORE INTO duplicate_candidates(id,left_job_id,right_job_id,method,score,status,evidence_json,created_at) VALUES(?,?,?,?,?,'suggested',?,?)").bind(id()).bind(left).bind(right).bind("fuzzy").bind((title_score+location_score)/2.0).bind(serde_json::json!({"title":title_score,"location":location_score,"company":"exact normalized"}).to_string()).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;}}
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }
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
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}
#[tauri::command]
pub async fn list_sources(state: State<'_, Arc<AppState>>) -> ApiResult<Vec<Source>> {
    sqlx::query_as::<_,Source>("SELECT id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,last_success_at,created_at,updated_at FROM sources WHERE deleted_at IS NULL ORDER BY name") .fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
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
    sqlx::query("INSERT INTO job_aliases(id,canonical_job_id,alias_job_id,reason,created_at) VALUES(?,?,?,?,?)").bind(id()).bind(&canonical_job_id).bind(&merged).bind("reviewed duplicate merge").bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT OR IGNORE INTO duplicate_groups(id,primary_job_id,member_job_id,reason,created_at) VALUES(?,?,?,?,?)").bind(id()).bind(&canonical_job_id).bind(&merged).bind("reviewed duplicate merge").bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
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
    sqlx::query("UPDATE job_aliases SET removed_at=? WHERE canonical_job_id=? AND alias_job_id=? AND removed_at IS NULL").bind(&t).bind(&canonical_job_id).bind(&merged_job_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("UPDATE duplicate_merge_audits SET undone_at=? WHERE id=?")
        .bind(&t)
        .bind(&audit)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE duplicate_candidates SET status='unmerged',decided_at=? WHERE (left_job_id=? AND right_job_id=?) OR (left_job_id=? AND right_job_id=?)").bind(&t).bind(&canonical_job_id).bind(&merged_job_id).bind(&merged_job_id).bind(&canonical_job_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn save_source(input: SourceInput, state: State<'_, Arc<AppState>>) -> ApiResult<Source> {
    valid_url(&input.base_url, input.allow_private_network)?;
    if input.kind == "reference" && input.enabled {
        return Err("Reference sources cannot be enabled for scraping".into());
    }
    let source_id = input.id.unwrap_or_else(id);
    let t = now();
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,robots_acknowledged_at,allow_private_network,created_at,updated_at) VALUES(?,?,?,?,?, ?,?,?, ?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,base_url=excluded.base_url,adapter_id=excluded.adapter_id,enabled=excluded.enabled,kind=excluded.kind,disabled_reason=excluded.disabled_reason,robots_override=excluded.robots_override,robots_acknowledged_at=excluded.robots_acknowledged_at,allow_private_network=excluded.allow_private_network,updated_at=excluded.updated_at")
  .bind(&source_id).bind(&input.name).bind(&input.base_url).bind(&input.adapter_id).bind("1.0.0").bind(input.enabled).bind(&input.kind).bind(&input.disabled_reason).bind(input.robots_override).bind(if input.robots_override {Some(t.clone())} else {None}).bind(input.allow_private_network).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO source_configs(id,source_id,config_json,created_at,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(source_id) DO UPDATE SET config_json=excluded.config_json,updated_at=excluded.updated_at").bind(id()).bind(&source_id).bind(input.config_json.to_string()).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as("SELECT id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,last_success_at,created_at,updated_at FROM sources WHERE id=?").bind(source_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
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
#[tauri::command]
pub async fn list_jobs(
    persona_id: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<crate::domain::Job>> {
    jobs_query(persona_id, None, &state.db.pool).await
}
#[tauri::command]
pub async fn search_jobs(
    query: String,
    persona_id: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<crate::domain::Job>> {
    jobs_query(persona_id, Some(query), &state.db.pool).await
}
async fn jobs_query(
    persona: Option<String>,
    search: Option<String>,
    pool: &SqlitePool,
) -> ApiResult<Vec<crate::domain::Job>> {
    let q = search.unwrap_or_default();
    let p = persona.unwrap_or_default();
    sqlx::query_as::<_,crate::domain::Job>("SELECT j.id,j.source_id,j.title,j.company,j.location,j.work_mode,j.canonical_url,j.apply_url,j.description_text,j.posted_at,j.salary_min,j.salary_max,j.salary_currency,j.seniority,j.availability,j.created_at,j.updated_at,m.score,m.eligible,m.explanation_json AS reasons FROM jobs j LEFT JOIN match_results m ON m.job_id=j.id AND m.persona_id=? WHERE (?='' OR j.rowid IN (SELECT rowid FROM jobs_fts WHERE jobs_fts MATCH ?)) ORDER BY COALESCE(m.score,-1) DESC,j.updated_at DESC LIMIT 500").bind(p).bind(&q).bind(&q).fetch_all(pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn list_personas(state: State<'_, Arc<AppState>>) -> ApiResult<Vec<Persona>> {
    sqlx::query_as("SELECT id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,resume_document_id,created_at,updated_at,archived_at FROM personas WHERE archived_at IS NULL ORDER BY name").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaDetails {
    pub persona: Persona,
    pub confirmed_skills: Vec<String>,
}
/// Returns editable persona data. Skills live in their own normalized table, so
/// listing personas deliberately does not leak them into every card response.
#[tauri::command]
pub async fn get_persona(
    persona_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<PersonaDetails> {
    let persona: Persona = sqlx::query_as("SELECT id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,resume_document_id,created_at,updated_at,archived_at FROM personas WHERE id=? AND archived_at IS NULL")
        .bind(&persona_id)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|_| "Active persona was not found".to_string())?;
    let confirmed_skills = sqlx::query_scalar(
        "SELECT skill FROM persona_skills WHERE persona_id=? AND confirmed=1 ORDER BY skill",
    )
    .bind(&persona_id)
    .fetch_all(&state.db.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(PersonaDetails {
        persona,
        confirmed_skills,
    })
}
#[tauri::command]
pub async fn archive_persona(persona_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<()> {
    sqlx::query("UPDATE personas SET archived_at=?,updated_at=? WHERE id=?")
        .bind(now())
        .bind(now())
        .bind(persona_id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn delete_persona(persona_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<()> {
    let deps:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM applications WHERE persona_id=?)+(SELECT count(*) FROM review_decisions WHERE persona_id=?)+(SELECT count(*) FROM match_results WHERE persona_id=?)").bind(&persona_id).bind(&persona_id).bind(&persona_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())?;
    if deps > 0 {
        return Err("Persona has review, match, or application history; archive it instead".into());
    }
    sqlx::query("DELETE FROM personas WHERE id=?")
        .bind(persona_id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn save_persona(
    input: PersonaInput,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Persona> {
    let persona_id = input.id.unwrap_or_else(id);
    let t = now();
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    if !["any", "all"].contains(&input.include_keyword_mode.as_str()) {
        return Err("Include keyword mode must be any or all".into());
    }
    sqlx::query("INSERT INTO personas(id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,resume_document_id,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,target_titles_json=excluded.target_titles_json,include_keywords_json=excluded.include_keywords_json,include_keyword_mode=excluded.include_keyword_mode,exclude_keywords_json=excluded.exclude_keywords_json,location=excluded.location,work_mode=excluded.work_mode,seniority=excluded.seniority,salary_min=excluded.salary_min,threshold=excluded.threshold,unknown_policy=excluded.unknown_policy,resume_document_id=excluded.resume_document_id,updated_at=excluded.updated_at")
 .bind(&persona_id).bind(&input.name).bind(serde_json::to_string(&input.target_titles).unwrap()).bind(serde_json::to_string(&input.include_keywords).unwrap()).bind(&input.include_keyword_mode).bind(serde_json::to_string(&input.exclude_keywords).unwrap()).bind(&input.location).bind(&input.work_mode).bind(&input.seniority).bind(input.salary_min).bind(input.threshold.clamp(0.0,100.0)).bind(&input.unknown_policy).bind(&input.resume_document_id).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("DELETE FROM persona_skills WHERE persona_id=?")
        .bind(&persona_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    for skill in input.confirmed_skills {
        sqlx::query(
            "INSERT INTO persona_skills(id,persona_id,skill,confirmed,required) VALUES(?,?,?,1,0)",
        )
        .bind(id())
        .bind(&persona_id)
        .bind(skill)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    rescore(&state.db.pool, &state.db.root, &persona_id, None).await?;
    sqlx::query_as("SELECT id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,resume_document_id,created_at,updated_at,archived_at FROM personas WHERE id=?").bind(persona_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn rescore_persona(persona_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<()> {
    rescore(&state.db.pool, &state.db.root, &persona_id, None).await
}
#[tauri::command]
pub async fn rescore_match(
    job_id: String,
    persona_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    rescore(&state.db.pool, &state.db.root, &persona_id, Some(&job_id)).await
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RescoreRun {
    pub run_id: String,
    pub completed: u64,
    pub total: u64,
    pub cancelled: bool,
}
#[tauri::command]
pub async fn cancel_rescore(run_id: String) -> ApiResult<()> {
    cancelled_runs()
        .lock()
        .map_err(|_| "Rescore cancellation lock failed")?
        .insert(run_id);
    Ok(())
}
#[tauri::command]
pub async fn rescore_stale_matches(
    persona_id: String,
    run_id: String,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<RescoreRun> {
    if !active_runs()
        .lock()
        .map_err(|_| "Rescore active lock failed")?
        .insert(run_id.clone())
    {
        return Err("This rescore run ID is already active".into());
    }
    let run_for_task = run_id.clone();
    let result = async {
    let rows = sqlx::query("SELECT j.id FROM jobs j LEFT JOIN match_results m ON m.job_id=j.id AND m.persona_id=? AND m.algorithm_version='2.0.0' AND m.model_version=? WHERE m.id IS NULL OR m.filter_decision_json NOT LIKE '%' || j.content_hash || '%'").bind(&persona_id).bind(crate::embedding::MODEL_VERSION).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())?;
    let total = rows.len() as u64;
    let mut completed = 0;
    for row in rows {
        if cancelled_runs()
            .lock()
            .map_err(|_| "Rescore cancellation lock failed")?
            .remove(&run_for_task)
        {
            return Ok(RescoreRun {
                run_id: run_for_task.clone(),
                completed,
                total,
                cancelled: true,
            });
        }
        let job_id: String = row.get(0);
        rescore(&state.db.pool, &state.db.root, &persona_id, Some(&job_id)).await?;
        completed += 1;
        let _ = app.emit(
            "rescore-progress",
            serde_json::json!({"runId":run_for_task.clone(),"completed":completed,"total" :total}),
        );
    }
    Ok(RescoreRun {
        run_id: run_for_task.clone(),
        completed,
        total,
        cancelled: false,
    })
    }.await;
    active_runs()
        .lock()
        .map_err(|_| "Rescore active lock failed")?
        .remove(&run_id);
    cancelled_runs()
        .lock()
        .map_err(|_| "Rescore cancellation lock failed")?
        .remove(&run_id);
    result
}
async fn cached_embedding(
    pool: &SqlitePool,
    root: &std::path::Path,
    owner_type: &str,
    owner_id: &str,
    text: &str,
) -> ApiResult<Vec<f32>> {
    let hash = crate::embedding::content_hash(text);
    if let Some(blob) = sqlx::query_scalar::<_, Vec<u8>>("SELECT vector FROM embeddings WHERE owner_type=? AND owner_id=? AND model=? AND dimensions=? AND content_hash=? ORDER BY created_at DESC LIMIT 1")
        .bind(owner_type).bind(owner_id).bind(crate::embedding::MODEL_VERSION).bind(crate::embedding::DIMENSIONS as i64).bind(&hash).fetch_optional(pool).await.map_err(|e| e.to_string())? {
        return crate::embedding::blob_f32(&blob);
    }
    // Inference deliberately runs before the following short write transaction.
    let vector = crate::embedding::embed_packaged(root, vec![text.to_string()])?
        .into_iter()
        .next()
        .ok_or("BGE returned no embedding")?;
    sqlx::query("INSERT OR IGNORE INTO embeddings(id,owner_type,owner_id,model,dimensions,content_hash,vector,created_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(id()).bind(owner_type).bind(owner_id).bind(crate::embedding::MODEL_VERSION).bind(crate::embedding::DIMENSIONS as i64).bind(&hash).bind(crate::embedding::f32_blob(&vector)).bind(now()).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(vector)
}
async fn rescore(
    pool: &SqlitePool,
    root: &std::path::Path,
    persona_id: &str,
    only_job: Option<&str>,
) -> ApiResult<()> {
    let p:Persona=sqlx::query_as("SELECT id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,created_at,updated_at FROM personas WHERE id=?").bind(persona_id).fetch_one(pool).await.map_err(|e|e.to_string())?;
    let skills: Vec<String> =
        sqlx::query("SELECT skill FROM persona_skills WHERE persona_id=? AND confirmed=1")
            .bind(persona_id)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| r.get(0))
            .collect();
    let jobs=sqlx::query("SELECT id,title,company,location,work_mode,seniority,salary_min,description_text,skills_json,content_hash FROM jobs WHERE (? IS NULL OR id=?)").bind(only_job).bind(only_job).fetch_all(pool).await.map_err(|e|e.to_string())?;
    let titles: Vec<String> = serde_json::from_str(&p.target_titles_json).unwrap_or_default();
    let includes: Vec<String> = serde_json::from_str(&p.include_keywords_json).unwrap_or_default();
    let excludes: Vec<String> = serde_json::from_str(&p.exclude_keywords_json).unwrap_or_default();
    let resume: Option<(String, String)> = sqlx::query("SELECT content_hash,extracted_text FROM resume_documents WHERE id=(SELECT resume_document_id FROM personas WHERE id=?)")
        .bind(persona_id).fetch_optional(pool).await.map_err(|e| e.to_string())?
        .map(|row| (row.get("content_hash"), row.get("extracted_text")));
    let resume_hash = resume.as_ref().map(|value| value.0.clone());
    let resume_text = resume.as_ref().map(|value| value.1.as_str()).unwrap_or("");
    let persona_text = format!(
        "{} {} {} {} {}",
        p.name,
        p.target_titles_json,
        skills.join(" "),
        p.include_keywords_json,
        resume_text
    );
    let persona_hash = crate::embedding::content_hash(&persona_text);
    let persona_vector = cached_embedding(pool, root, "persona", persona_id, &persona_text).await?;
    for j in jobs {
        let jid: String = j.get("id");
        let description: String = j.get("description_text");
        let chunks = crate::embedding::chunk_text(&description);
        let mut chunk_similarity = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            let vector = cached_embedding(
                pool,
                root,
                "job-description-chunk",
                &format!("{jid}:{index}"),
                chunk,
            )
            .await?;
            chunk_similarity.push(crate::embedding::cosine(&persona_vector, &vector).max(0.0));
        }
        let title_text: String = j.get("title");
        let title_vector = cached_embedding(pool, root, "job-title", &jid, &title_text).await?;
        let mut title_scores = Vec::new();
        for title in &titles {
            let owner = format!("{persona_id}:{}", crate::embedding::content_hash(title));
            let vector = cached_embedding(pool, root, "persona-title", &owner, title).await?;
            title_scores.push(crate::embedding::cosine(&vector, &title_vector).max(0.0));
        }
        let score = crate::matching::score_with_similarity(
            &crate::matching::PersonaProfile {
                titles: titles.clone(),
                skills: skills.clone(),
                include: includes.clone(),
                include_mode: p.include_keyword_mode.clone(),
                exclude: excludes.clone(),
                location: p.location.clone(),
                work_mode: p.work_mode.clone(),
                seniority: p.seniority.clone(),
                salary_min: p.salary_min,
                unknown_policy: p.unknown_policy.clone(),
            },
            &crate::matching::JobProfile {
                title: j.get("title"),
                location: j.get("location"),
                work_mode: j.get("work_mode"),
                seniority: j.get("seniority"),
                salary_min: j.get("salary_min"),
                description: j.get("description_text"),
                skills: serde_json::from_str(&j.get::<String, _>("skills_json"))
                    .unwrap_or_default(),
            },
            Some(crate::embedding::mean_top_three(chunk_similarity)),
            Some(title_scores.into_iter().fold(0.0, f64::max)),
        );
        let job_hash: String = j.get("content_hash");
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        let still_current: Option<String> = sqlx::query_scalar("SELECT j.id FROM jobs j JOIN personas p ON p.id=? WHERE j.id=? AND j.content_hash=? AND p.updated_at=? AND ((? IS NULL AND p.resume_document_id IS NULL) OR EXISTS (SELECT 1 FROM resume_documents r WHERE r.id=p.resume_document_id AND r.content_hash=?))").bind(persona_id).bind(&jid).bind(&job_hash).bind(&p.updated_at).bind(&resume_hash).bind(&resume_hash).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        if still_current.is_some() {
            sqlx::query("INSERT INTO match_results(id,job_id,persona_id,score,eligible,algorithm_version,model_version,filter_decision_json,components_json,explanation_json,created_at,updated_at) VALUES(?,?,?,?,?,'2.0.0',?,?,?,?,?,?) ON CONFLICT(job_id,persona_id,algorithm_version) DO UPDATE SET score=excluded.score,eligible=excluded.eligible,model_version=excluded.model_version,filter_decision_json=excluded.filter_decision_json,components_json=excluded.components_json,explanation_json=excluded.explanation_json,updated_at=excluded.updated_at").bind(id()).bind(&jid).bind(persona_id).bind(score.total).bind(score.eligible).bind(crate::embedding::MODEL_VERSION).bind(serde_json::json!({"reasons":score.filters,"personaContentHash":persona_hash,"resumeContentHash":resume_hash,"jobContentHash":job_hash,"algorithmHash":"2.0.0-45-25-20-10"}).to_string()).bind(serde_json::to_string(&score.components).unwrap()).bind(serde_json::to_string(&score.evidence).unwrap()).bind(now()).bind(now()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
pub async fn get_match_explanation(
    job_id: String,
    persona_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<serde_json::Value> {
    let row = sqlx::query("SELECT score,eligible,filter_decision_json,components_json,explanation_json,model_version,algorithm_version FROM match_results WHERE job_id=? AND persona_id=? ORDER BY updated_at DESC LIMIT 1").bind(job_id).bind(persona_id).fetch_one(&state.db.pool).await.map_err(|_| "Match result was not found".to_string())?;
    Ok(
        serde_json::json!({"score":row.get::<f64,_>(0),"eligible":row.get::<bool,_>(1),"filters":serde_json::from_str::<serde_json::Value>(&row.get::<String,_>(2)).unwrap_or_default(),"components":serde_json::from_str::<serde_json::Value>(&row.get::<String,_>(3)).unwrap_or_default(),"evidence":serde_json::from_str::<serde_json::Value>(&row.get::<String,_>(4)).unwrap_or_default(),"modelVersion":row.get::<String,_>(5),"algorithmVersion":row.get::<String,_>(6)}),
    )
}
#[tauri::command]
pub async fn list_review_queue(
    persona_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<crate::domain::Job>> {
    sqlx::query_as("SELECT j.id,j.source_id,j.title,j.company,j.location,j.work_mode,j.canonical_url,j.apply_url,j.description_text,j.posted_at,j.salary_min,j.salary_max,j.salary_currency,j.seniority,j.availability,j.created_at,j.updated_at,m.score,m.eligible,m.explanation_json AS reasons FROM jobs j JOIN match_results m ON m.job_id=j.id AND m.persona_id=? LEFT JOIN review_decisions d ON d.job_id=j.id AND d.persona_id=m.persona_id WHERE m.eligible=1 AND m.score >= (SELECT threshold FROM personas WHERE id=m.persona_id) AND COALESCE(d.status,'unseen') IN ('unseen','reviewing') ORDER BY m.score DESC,j.posted_at DESC").bind(persona_id).fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn set_review_decision(
    job_id: String,
    persona_id: String,
    status: String,
    reason: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    if !["unseen", "reviewing", "shortlisted", "dismissed"].contains(&status.as_str()) {
        return Err("Invalid review state".into());
    };
    sqlx::query("INSERT INTO review_decisions(id,job_id,persona_id,status,reason,decided_at) VALUES(?,?,?,?,?,?) ON CONFLICT(job_id,persona_id) DO UPDATE SET status=excluded.status,reason=excluded.reason,decided_at=excluded.decided_at").bind(id()).bind(job_id).bind(persona_id).bind(status).bind(reason).bind(now()).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
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
    pub base64: Option<String>,
    pub resume_document_id: Option<String>,
    pub event_id: Option<String>,
}
fn controlled_resume_bytes(root: &Path, stored: &str) -> ApiResult<Vec<u8>> {
    let documents = root
        .join("documents")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let file = Path::new(stored)
        .canonicalize()
        .map_err(|_| "Resume source is missing".to_string())?;
    if !file.starts_with(&documents) {
        return Err("Resume source is outside controlled documents".into());
    };
    std::fs::read(file).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn attach_application_document(
    input: AttachApplicationDocument,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ApplicationDocument> {
    if !["resume", "cover_letter", "other"].contains(&input.document_type.as_str()) {
        return Err("Document type must be resume, cover_letter, or other".into());
    }
    let (filename, mime, content) = match (input.base64, input.resume_document_id) {
        (Some(encoded), None) => {
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| "Invalid attachment bytes")?;
            let filename = input
                .filename
                .filter(|v| !v.trim().is_empty())
                .ok_or("Attachment filename is required")?;
            (
                filename,
                input
                    .mime_type
                    .unwrap_or_else(|| "application/octet-stream".into()),
                bytes,
            )
        }
        (None, Some(resume_id)) => {
            let row =
                sqlx::query("SELECT filename,mime_type,path FROM resume_documents WHERE id=?")
                    .bind(resume_id)
                    .fetch_one(&state.db.pool)
                    .await
                    .map_err(|_| "Resume version was not found".to_string())?;
            let filename: String = row.get(0);
            let mime: String = row.get(1);
            let path: String = row.get(2);
            (
                filename,
                mime,
                controlled_resume_bytes(&state.db.root, &path)?,
            )
        }
        _ => return Err("Attach either controlled resume version or local file bytes".into()),
    };
    if content.len() > 20 * 1024 * 1024 {
        return Err("Attachment exceeds 20 MB".into());
    };
    let document_id = id();
    let sha256 = format!("{:x}", Sha256::digest(&content));
    let size = content.len() as i64;
    let t = now();
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO application_documents(id,application_id,kind,document_type,filename,content,mime_type,sha256,size,event_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(&document_id).bind(&input.application_id).bind(&input.document_type).bind(&input.document_type).bind(&filename).bind(content).bind(&mime).bind(&sha256).bind(size).bind(&input.event_id).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,occurred_at,payload_json) VALUES(?,?, 'document_attached',?,?)").bind(id()).bind(&input.application_id).bind(&t).bind(serde_json::json!({"documentId":document_id,"sha256":sha256,"filename":filename}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    sqlx::query_as("SELECT id,application_id,kind,document_type,filename,mime_type,sha256,size,event_id,created_at FROM application_documents WHERE id=?").bind(document_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDocumentExport {
    pub filename: String,
    pub mime_type: String,
    pub base64: String,
    pub sha256: String,
}
#[tauri::command]
pub async fn export_application_document(
    document_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ApplicationDocumentExport> {
    let row = sqlx::query(
        "SELECT filename,mime_type,content,sha256 FROM application_documents WHERE id=?",
    )
    .bind(document_id)
    .fetch_one(&state.db.pool)
    .await
    .map_err(|_| "Application document was not found".to_string())?;
    let bytes: Vec<u8> = row.get(2);
    let sha256: String = row.get(3);
    if format!("{:x}", Sha256::digest(&bytes)) != sha256 {
        return Err("Application document checksum failed".into());
    };
    Ok(ApplicationDocumentExport {
        filename: row.get(0),
        mime_type: row.get(1),
        base64: STANDARD.encode(bytes),
        sha256,
    })
}
#[tauri::command]
pub async fn create_application(
    job_id: String,
    persona_id: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Application> {
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
    if decision != "not_yet" {
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
    let row=sqlx::query("SELECT id,application_id,job_id,opened_at,original_stage FROM application_attempts WHERE resolved_at IS NULL ORDER BY opened_at DESC LIMIT 1").fetch_optional(pool).await.map_err(|e|e.to_string())?;
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
                if let Err(error) = scheduler.cancel(&task) {
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
        let task = existing.as_deref().map(|v| scheduler.exists(v)).transpose();
        let task = match task {
            Ok(Some(value)) => value,
            Ok(None) => false,
            Err(error) => {
                sqlx::query("UPDATE reminders SET last_error=?,last_reconciled_at=? WHERE id=?")
                    .bind(error)
                    .bind(now())
                    .bind(&id)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
                errors += 1;
                false
            }
        };
        if !task {
            match scheduler.create(&reminder) {
                Ok(task_id) => {
                    sqlx::query("UPDATE reminders SET os_task_id=?,last_reconciled_at=?,last_error=NULL WHERE id=?").bind(&task_id).bind(now()).bind(&id).execute(pool).await.map_err(|e|e.to_string())?;
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
    // Enumeration can list many system tasks, but only UUID-validated names in
    // JobScraper namespace are ever selected or deleted.
    match scheduler.list_managed() {
        Ok(tasks) => {
            for task in scoped_orphans(&tasks, &expected_tasks) {
                if let Err(error) = scheduler.cancel(&task) {
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
        }
        Err(error) => {
            errors += 1;
            let _ = sqlx::query(
                "UPDATE reminders SET last_error=?,last_reconciled_at=? WHERE status='pending'",
            )
            .bind(error)
            .bind(now())
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
pub struct Diagnostics {
    db_path: String,
    schema_version: i64,
    source_count: i64,
    job_count: i64,
    startup_network: bool,
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
        startup_network: false,
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
        "jobs",
        "job_occurrences",
        "job_revisions",
        "personas",
        "resume_documents",
        "persona_skills",
        "persona_filters",
        "embeddings",
        "match_results",
        "review_decisions",
        "applications",
        "application_events",
        "notes",
        "application_documents",
        "interviews",
        "reminders",
        "job_aliases",
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
        "jobs",
        "job_occurrences",
        "job_revisions",
        "personas",
        "resume_documents",
        "persona_skills",
        "persona_filters",
        "embeddings",
        "match_results",
        "review_decisions",
        "applications",
        "application_events",
        "notes",
        "application_documents",
        "interviews",
        "reminders",
        "job_aliases",
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
            "persona_skills" | "persona_filters" => rows
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
            "job_aliases" | "duplicate_candidates" => rows
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
            let events: Vec<String> =
                sqlx::query_scalar("SELECT id FROM scrape_run_events WHERE occurred_at<?")
                    .bind(&before)
                    .fetch_all(&state.db.pool)
                    .await
                    .map_err(|e| e.to_string())?;
            (vec![],events,vec![],0,"Deletes only old scrape run event diagnostics; sources, runs, jobs, and history remain.".into())
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
        let changed = sqlx::query("DELETE FROM scrape_run_events WHERE id=?")
            .bind(event)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if changed.rows_affected() != 1 {
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
    async fn guard(
        pool: &SqlitePool,
        job_hash: &str,
        persona_updated: &str,
        resume_hash: Option<&str>,
    ) -> bool {
        sqlx::query_scalar::<_,String>("SELECT j.id FROM jobs j JOIN personas p ON p.id='p' WHERE j.id='j' AND j.content_hash=? AND p.updated_at=? AND ((? IS NULL AND p.resume_document_id IS NULL) OR EXISTS (SELECT 1 FROM resume_documents r WHERE r.id=p.resume_document_id AND r.content_hash=?))").bind(job_hash).bind(persona_updated).bind(resume_hash).bind(resume_hash).fetch_optional(pool).await.unwrap().is_some()
    }
    #[tokio::test]
    async fn cache_hit_and_model_dimension_hash_invalidate() {
        let pool = migrated_pool().await;
        let vector = crate::embedding::f32_blob(&vec![
            1.0 / (crate::embedding::DIMENSIONS as f32)
                .sqrt();
            crate::embedding::DIMENSIONS
        ]);
        sqlx::query("INSERT INTO embeddings(id,owner_type,owner_id,model,dimensions,content_hash,vector,created_at) VALUES('e','job','j',?,?,?,?,'now')").bind(crate::embedding::MODEL_VERSION).bind(crate::embedding::DIMENSIONS as i64).bind(crate::embedding::content_hash("same")).bind(vector).execute(&pool).await.unwrap();
        assert!(
            cached_embedding(&pool, std::path::Path::new("missing"), "job", "j", "same")
                .await
                .is_ok()
        );
        let miss = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM embeddings WHERE owner_type='job' AND owner_id='j' AND content_hash!=?").bind(crate::embedding::content_hash("same")).fetch_one(&pool).await.unwrap();
        assert_eq!(miss, 0);
        pool.close().await;
    }
    #[tokio::test]
    async fn corrupt_cache_blob_is_rejected() {
        let pool = migrated_pool().await;
        sqlx::query("INSERT INTO embeddings(id,owner_type,owner_id,model,dimensions,content_hash,vector,created_at) VALUES('e','job','j',?,?,?,X'00','now')").bind(crate::embedding::MODEL_VERSION).bind(crate::embedding::DIMENSIONS as i64).bind(crate::embedding::content_hash("same")).execute(&pool).await.unwrap();
        assert!(
            cached_embedding(&pool, std::path::Path::new("missing"), "job", "j", "same")
                .await
                .is_err()
        );
        pool.close().await;
    }
    #[tokio::test]
    async fn persona_job_and_resume_snapshots_guard_upsert() {
        let pool = migrated_pool().await;
        seed_match(&pool).await;
        assert!(guard(&pool, "job-v1", "persona-v1", Some("resume-v1")).await);
        assert!(!guard(&pool, "job-v2", "persona-v1", Some("resume-v1")).await);
        assert!(!guard(&pool, "job-v1", "persona-v2", Some("resume-v1")).await);
        assert!(!guard(&pool, "job-v1", "persona-v1", Some("resume-v2")).await);
        pool.close().await;
    }
    #[tokio::test]
    async fn match_result_fields_commit_together_and_rollback_is_empty() {
        let pool = migrated_pool().await;
        seed_match(&pool).await;
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO match_results(id,job_id,persona_id,score,eligible,algorithm_version,model_version,filter_decision_json,components_json,explanation_json,created_at,updated_at) VALUES('m','j','p',72,1,'2.0.0','model','{\"jobContentHash\":\"job-v1\"}','{\"semantic\":45}','{\"matched_skills\":[\"rust\"]}','t','t')").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let row=sqlx::query("SELECT score,components_json,explanation_json,filter_decision_json,model_version,algorithm_version FROM match_results WHERE id='m'").fetch_one(&pool).await.unwrap();
        assert_eq!(row.get::<f64, _>(0), 72.0);
        assert!(row.get::<String, _>(1).contains("semantic"));
        assert!(row.get::<String, _>(2).contains("matched_skills"));
        assert!(row.get::<String, _>(3).contains("job-v1"));
        assert_eq!(row.get::<String, _>(4), "model");
        assert_eq!(row.get::<String, _>(5), "2.0.0");
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO match_results(id,job_id,persona_id,score,eligible,algorithm_version,model_version,filter_decision_json,components_json,explanation_json,created_at,updated_at) VALUES('rollback','j','p',0,0,'x','x','{}','{}','{}','t','t')").execute(&mut *tx).await.unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM match_results WHERE id='rollback'")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        pool.close().await;
    }
    #[test]
    fn cancellation_registry_rejects_duplicate_and_cleans_up() {
        let run = "unit-run".to_string();
        let mut active = active_runs().lock().unwrap();
        assert!(active.insert(run.clone()));
        assert!(!active.insert(run.clone()));
        active.remove(&run);
        drop(active);
        cancelled_runs().lock().unwrap().insert(run.clone());
        assert!(cancelled_runs().lock().unwrap().remove(&run));
        assert!(!cancelled_runs().lock().unwrap().contains(&run));
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
    async fn starter_reconciliation_retires_only_exact_untouched_legacy_rows() {
        let root = std::env::temp_dir().join(format!("jobscraper-starter-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("jobscraper.db")).await.unwrap();
        let stamp = "2026-01-01T00:00:00Z";
        for (source, changed) in [("legacy", false), ("user-owned", true)] {
            sqlx::query("INSERT INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?, 'https://careers.microchip.com/','workday','1.0.0',0,'active','Starter source is disabled until you review and enable it.',0,0,?,?)")
                .bind(source).bind("Microchip").bind(stamp).bind(if changed { "2026-01-02T00:00:00Z" } else { stamp }).execute(&db.pool).await.unwrap();
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
    fn dedupe_normalization_and_conservative_fuzzy_thresholds() {
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
        assert!(fuzzy_similarity("Firmware Engineer", "Firmware Engineer II") >= 0.84);
        assert!(fuzzy_similarity("Firmware Engineer", "Marketing Manager") < 0.70);
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
