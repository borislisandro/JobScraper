use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub adapter_id: String,
    pub enabled: bool,
    pub kind: String,
    pub disabled_reason: Option<String>,
    pub config_json: serde_json::Value,
    pub robots_override: bool,
    pub allow_private_network: bool,
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub enabled: bool,
    pub kind: String,
    pub disabled_reason: Option<String>,
    pub robots_override: bool,
    pub last_success_at: Option<String>,
    /// What this source is currently holding: `job_count` is what is still on its board, and
    /// `closed_count` is what was read from it and has since come off. Counted for the Sources
    /// list, which is the one place a source's yield is worth seeing next to its name.
    pub job_count: i64,
    pub closed_count: i64,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub work_mode: Option<String>,
    pub canonical_url: Option<String>,
    pub apply_url: Option<String>,
    pub description_text: String,
    pub description_status: String,
    pub posted_at: Option<String>,
    pub salary_min: Option<f64>,
    pub salary_max: Option<f64>,
    pub salary_currency: Option<String>,
    pub seniority: Option<String>,
    pub availability: String,
    pub created_at: String,
    pub updated_at: String,
    pub score: Option<f64>,
    pub eligible: Option<bool>,
    pub reasons: Option<String>,
    /// Stage of the newest application for this job; None when the job was never saved.
    pub application_stage: Option<String>,
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub id: String,
    pub job_id: String,
    pub persona_id: Option<String>,
    pub current_stage: String,
    pub recruiter: Option<String>,
    pub recruiter_name: Option<String>,
    pub recruiter_email: Option<String>,
    pub recruiter_phone: Option<String>,
    pub source_attribution: Option<String>,
    pub rejection_reason: Option<String>,
    pub rejection_category: Option<String>,
    pub withdrawn_reason: Option<String>,
    pub accepted_at: Option<String>,
    pub applied_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub title: Option<String>,
    pub company: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInput {
    pub application_id: String,
    pub stage: String,
    pub occurred_at: Option<String>,
    pub payload: Option<serde_json::Value>,
    pub reason: Option<String>,
    #[serde(default)]
    pub manual_override: bool,
}
