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
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaInput {
    pub id: Option<String>,
    pub name: String,
    pub target_titles: Vec<String>,
    pub include_keywords: Vec<String>,
    #[serde(default = "default_include_keyword_mode")]
    pub include_keyword_mode: String,
    pub exclude_keywords: Vec<String>,
    pub location: Option<String>,
    pub work_mode: Option<String>,
    pub seniority: Option<String>,
    pub salary_min: Option<f64>,
    pub threshold: f64,
    pub unknown_policy: String,
    pub confirmed_skills: Vec<String>,
    pub resume_document_id: Option<String>,
}
fn default_include_keyword_mode() -> String {
    "any".into()
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub target_titles_json: String,
    pub include_keywords_json: String,
    pub include_keyword_mode: String,
    pub exclude_keywords_json: String,
    pub location: Option<String>,
    pub work_mode: Option<String>,
    pub seniority: Option<String>,
    pub salary_min: Option<f64>,
    pub threshold: f64,
    pub unknown_policy: String,
    pub resume_document_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub id: String,
    pub job_id: String,
    pub persona_id: Option<String>,
    pub current_stage: String,
    pub recruiter: Option<String>,
    pub rejection_reason: Option<String>,
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
}
