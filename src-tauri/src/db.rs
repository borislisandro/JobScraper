use crate::notifications::{
    is_managed_task_path, scoped_orphans, Reminder as ScheduledReminder, Scheduler,
    WindowsTaskScheduler,
};
use crate::{
    domain::{Application, Persona, PersonaInput, Source, SourceInput, StageInput},
    AppState,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use tauri::{Emitter, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;
use uuid::Uuid;

pub type ApiResult<T> = Result<T, String>;
static RESCORE_CANCELLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static RESCORE_ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
fn cancelled_runs() -> &'static Mutex<HashSet<String>> {
    RESCORE_CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}
fn active_runs() -> &'static Mutex<HashSet<String>> {
    RESCORE_ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}
#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
    pub root: PathBuf,
}
fn now() -> String {
    Utc::now().to_rfc3339()
}
fn ghost_due_from(occurred_at: &str, days: i64) -> ApiResult<String> {
    chrono::DateTime::parse_from_rfc3339(occurred_at)
        .map_err(|_| "Application timestamp must be UTC RFC3339".to_string())
        .map(|value| (value.with_timezone(&Utc) + chrono::Duration::days(days)).to_rfc3339())
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
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
        let starters = [
            ("Microchip", "https://careers.microchip.com/", "workday"),
            (
                "Analog Devices",
                "https://analogdevices.wd1.myworkdayjobs.com/",
                "workday",
            ),
            (
                "Broadcom",
                "https://broadcom.wd1.myworkdayjobs.com/",
                "workday",
            ),
            ("Intel", "https://jobs.intel.com/", "workday"),
            ("STMicroelectronics", "https://careers.st.com/", "eightfold"),
            (
                "NVIDIA",
                "https://nvidia.wd5.myworkdayjobs.com/",
                "eightfold",
            ),
            ("GlobalFoundries", "https://gf.com/careers", "eightfold"),
            ("Micron", "https://careers.micron.com/", "eightfold"),
            ("Qualcomm", "https://careers.qualcomm.com/", "eightfold"),
            ("Arm", "https://careers.arm.com/", "icims"),
            ("AMD", "https://careers.amd.com/", "icims"),
            ("Cisco", "https://jobs.cisco.com/", "phenom"),
            ("Apple", "https://jobs.apple.com/", "custom-api"),
            ("MediaTek", "https://www.mediatek.com/careers", "custom-api"),
            ("u-blox", "https://www.u-blox.com/en/careers", "custom-api"),
            (
                "Google",
                "https://www.google.com/about/careers/applications/jobs/results",
                "custom-api",
            ),
            (
                "SK hynix",
                "https://www.skhynix.com/eng/careers/",
                "custom-api",
            ),
            (
                "Marvell careers (reference)",
                "https://www.marvell.com/company/careers.html",
                "reference",
            ),
        ];
        for (name, url, adapter) in starters {
            let source_id = id();
            let t = now();
            let kind = if adapter == "reference" {
                "reference"
            } else {
                "active"
            };
            sqlx::query("INSERT OR IGNORE INTO sources(id,name,base_url,adapter_id,adapter_version,enabled,kind,disabled_reason,robots_override,allow_private_network,created_at,updated_at) VALUES(?,?,?,?,?,0,?,?,0,0,?,?)")
    .bind(&source_id).bind(name).bind(url).bind(adapter).bind("1.0.0").bind(kind).bind(if kind=="reference" {Some("Reference-only source: it is never scraped.")} else {Some("Starter source is disabled until you review and enable it.")}).bind(&t).bind(&t).execute(&self.pool).await.map_err(|e| e.to_string())?;
        }
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
        let canonical = payload.get("canonicalUrl").and_then(|v| v.as_str());
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
        let existing:Option<String>=sqlx::query_scalar("SELECT id FROM jobs WHERE (source_id=? AND external_id=?) OR (? IS NOT NULL AND canonical_url=?) LIMIT 1").bind(source_id).bind(external).bind(canonical).bind(canonical).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        let actual = existing.unwrap_or(job_id);
        sqlx::query("INSERT INTO jobs(id,source_id,external_id,canonical_url,apply_url,title,company,location,work_mode,description_text,description_html,posted_at,closing_at,salary_min,salary_max,salary_currency,salary_period,salary_confidence,seniority,skills_json,content_hash,provenance_json,extraction_at,adapter_version,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET canonical_url=excluded.canonical_url,apply_url=excluded.apply_url,title=excluded.title,company=excluded.company,location=excluded.location,work_mode=excluded.work_mode,description_text=excluded.description_text,description_html=excluded.description_html,skills_json=excluded.skills_json,content_hash=excluded.content_hash,provenance_json=excluded.provenance_json,extraction_at=excluded.extraction_at,updated_at=excluded.updated_at")
  .bind(&actual).bind(source_id).bind(external).bind(canonical).bind(payload.get("applyUrl").and_then(|v|v.as_str())).bind(title).bind(company).bind(payload.get("location").and_then(|v|v.as_str())).bind(payload.get("workMode").and_then(|v|v.as_str())).bind(description).bind(payload.get("descriptionHtml").and_then(|v|v.as_str()).map(|s|s.chars().take(250_000).collect::<String>())).bind(payload.get("postedAt").and_then(|v|v.as_str())).bind(payload.get("closingAt").and_then(|v|v.as_str())).bind(payload.get("salaryMin").and_then(|v|v.as_f64())).bind(payload.get("salaryMax").and_then(|v|v.as_f64())).bind(payload.get("salaryCurrency").and_then(|v|v.as_str())).bind(payload.get("salaryPeriod").and_then(|v|v.as_str())).bind(payload.get("salaryConfidence").and_then(|v|v.as_str())).bind(payload.get("seniority").and_then(|v|v.as_str())).bind(payload.get("skills").cloned().unwrap_or_else(||serde_json::json!([])).to_string()).bind(hash).bind(payload.get("provenance").cloned().unwrap_or_else(||serde_json::json!({})).to_string()).bind(&t).bind(payload.get("adapterVersion").and_then(|v|v.as_str()).unwrap_or("1.0.0")).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT INTO job_occurrences(id,job_id,run_id,seen_at) VALUES(?,?,?,?)")
            .bind(id())
            .bind(&actual)
            .bind(run_id)
            .bind(&t)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
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
    sqlx::query_as("SELECT id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,created_at,updated_at FROM personas ORDER BY name").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
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
    sqlx::query("INSERT INTO personas(id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,target_titles_json=excluded.target_titles_json,include_keywords_json=excluded.include_keywords_json,include_keyword_mode=excluded.include_keyword_mode,exclude_keywords_json=excluded.exclude_keywords_json,location=excluded.location,work_mode=excluded.work_mode,seniority=excluded.seniority,salary_min=excluded.salary_min,threshold=excluded.threshold,unknown_policy=excluded.unknown_policy,updated_at=excluded.updated_at")
 .bind(&persona_id).bind(&input.name).bind(serde_json::to_string(&input.target_titles).unwrap()).bind(serde_json::to_string(&input.include_keywords).unwrap()).bind(&input.include_keyword_mode).bind(serde_json::to_string(&input.exclude_keywords).unwrap()).bind(&input.location).bind(&input.work_mode).bind(&input.seniority).bind(input.salary_min).bind(input.threshold.clamp(0.0,100.0)).bind(&input.unknown_policy).bind(&t).bind(&t).execute(&mut *tx).await.map_err(|e|e.to_string())?;
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
    sqlx::query_as("SELECT id,name,target_titles_json,include_keywords_json,include_keyword_mode,exclude_keywords_json,location,work_mode,seniority,salary_min,threshold,unknown_policy,created_at,updated_at FROM personas WHERE id=?").bind(persona_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
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
    sqlx::query_as("SELECT a.id,a.job_id,a.persona_id,a.current_stage,a.recruiter,a.rejection_reason,a.applied_at,a.created_at,a.updated_at,j.title,j.company FROM applications a LEFT JOIN jobs j ON j.id=a.job_id ORDER BY a.updated_at DESC").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
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
    sqlx::query_as("SELECT a.id,a.job_id,a.persona_id,a.current_stage,a.recruiter,a.rejection_reason,a.applied_at,a.created_at,a.updated_at,j.title,j.company FROM applications a LEFT JOIN jobs j ON j.id=a.job_id WHERE a.id=?").bind(application_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
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
    if input.stage == "applied" {
        return Err("Use Open application and explicit Yes confirmation to record applied".into());
    }
    let mut tx = state.db.pool.begin().await.map_err(|e| e.to_string())?;
    let current: String = sqlx::query_scalar("SELECT current_stage FROM applications WHERE id=?")
        .bind(&input.application_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Application was not found".to_string())?;
    let occurred = input.occurred_at.unwrap_or_else(now);
    let applied = if input.stage == "applied" {
        Some(occurred.clone())
    } else {
        None
    };
    sqlx::query("UPDATE applications SET current_stage=?, applied_at=COALESCE(applied_at,?),updated_at=? WHERE id=?").bind(&input.stage).bind(applied).bind(&occurred).bind(&input.application_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO application_events(id,application_id,event_type,from_stage,to_stage,occurred_at,payload_json) VALUES(?,?, 'stage_changed',?,?,?,?,?)").bind(id()).bind(&input.application_id).bind(&current).bind(&input.stage).bind(&occurred).bind(input.payload.unwrap_or_else(||serde_json::json!({})).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
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
    sqlx::query_as("SELECT a.id,a.job_id,a.persona_id,a.current_stage,a.recruiter,a.rejection_reason,a.applied_at,a.created_at,a.updated_at,j.title,j.company FROM applications a LEFT JOIN jobs j ON j.id=a.job_id WHERE a.id=?").bind(input.application_id).fetch_one(&state.db.pool).await.map_err(|e|e.to_string())
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
    sqlx::query_as("SELECT id,application_id,stage,scheduled_at,completed_at,notes,created_at,updated_at FROM interviews WHERE application_id=? ORDER BY scheduled_at")
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
        sqlx::query("UPDATE interviews SET stage=?,scheduled_at=?,notes=?,updated_at=? WHERE id=?")
            .bind(&input.stage)
            .bind(&input.scheduled_at)
            .bind(&input.notes)
            .bind(&t)
            .bind(&interview_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE reminders SET status='cancelled' WHERE entity_id=? AND reminder_type='interview' AND status='pending'")
            .bind(&interview_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    } else {
        sqlx::query("INSERT INTO interviews(id,application_id,stage,scheduled_at,notes,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
            .bind(&interview_id).bind(&input.application_id).bind(&input.stage).bind(&input.scheduled_at).bind(&input.notes).bind(&t).bind(&t)
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
    sqlx::query_as("SELECT id,application_id,stage,scheduled_at,completed_at,notes,created_at,updated_at FROM interviews WHERE id=?")
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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Analytics {
    total: u64,
    by_stage: Vec<Count>,
    applied_to_response_hours: Option<f64>,
    response_samples: u64,
}
#[derive(Serialize)]
struct Count {
    name: String,
    count: i64,
}
#[tauri::command]
pub async fn analytics(state: State<'_, Arc<AppState>>) -> ApiResult<Analytics> {
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM applications")
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    let by_stage = sqlx::query(
        "SELECT current_stage,count(*) AS count FROM applications GROUP BY current_stage",
    )
    .fetch_all(&state.db.pool)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|r| Count {
        name: r.get(0),
        count: r.get(1),
    })
    .collect();
    let row=sqlx::query("SELECT avg((julianday(e.occurred_at)-julianday(a.applied_at))*24),count(*) FROM applications a JOIN application_events e ON e.application_id=a.id WHERE a.applied_at IS NOT NULL AND e.occurred_at>a.applied_at AND e.to_stage IN ('screening','interviewing','offer','rejected')").fetch_one(&state.db.pool).await.map_err(|e|e.to_string())?;
    Ok(Analytics {
        total: total as u64,
        by_stage,
        applied_to_response_hours: row.get(0),
        response_samples: row.get::<i64, _>(1) as u64,
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
}
