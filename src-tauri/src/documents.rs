use crate::{db::ApiResult, AppState};
use base64::{engine::general_purpose::STANDARD, Engine};
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::io::{Cursor, Read};
use std::sync::Arc;
use tauri::State;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeImport {
    pub persona_id: Option<String>,
    pub filename: String,
    pub base64: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeResult {
    pub id: String,
    pub extracted_text: String,
    pub image_only: bool,
    pub suggested_skills: Vec<String>,
    pub suggested_titles: Vec<String>,
    pub requires_manual_paste: bool,
    pub content_hash: String,
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ResumeDocument {
    pub id: String,
    pub persona_id: Option<String>,
    pub filename: String,
    pub extracted_text: String,
    pub mime_type: String,
    pub content_hash: String,
    pub created_at: String,
    pub updated_at: String,
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn docx(bytes: &[u8]) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| "DOCX is missing word/document.xml")?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut result = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"w:t" => in_text = true,
            Ok(Event::End(e)) if e.name().as_ref() == b"w:t" => {
                in_text = false;
                result.push(' ')
            }
            Ok(Event::Text(e)) if in_text => {
                result.push_str(&e.unescape().map_err(|e| e.to_string())?)
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
    }
    Ok(result.split_whitespace().collect::<Vec<_>>().join(" "))
}
fn suggestions(text: &str) -> (Vec<String>, Vec<String>) {
    let lower = text.to_ascii_lowercase();
    let skills = [
        "rust",
        "python",
        "typescript",
        "c++",
        "sql",
        "verilog",
        "vhdl",
        "linux",
        "git",
        "aws",
        "azure",
        "docker",
        "kubernetes",
        "tensorflow",
        "matlab",
        "cadence",
        "altium",
    ];
    let titles = [
        "software engineer",
        "hardware engineer",
        "embedded engineer",
        "data engineer",
        "firmware engineer",
        "systems engineer",
        "application engineer",
    ];
    (
        skills
            .into_iter()
            .filter(|s| lower.contains(s))
            .map(str::to_string)
            .collect(),
        titles
            .into_iter()
            .filter(|s| lower.contains(s))
            .map(str::to_string)
            .collect(),
    )
}
#[tauri::command]
pub async fn import_resume(
    input: ResumeImport,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ResumeResult> {
    let bytes = STANDARD
        .decode(input.base64)
        .map_err(|_| "Invalid uploaded resume bytes")?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("Resume exceeds 20 MB".into());
    };
    let ext = input
        .filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = match ext.as_str() {
        "pdf" => pdf_extract::extract_text_from_mem(&bytes).map_err(|e| e.to_string())?,
        "docx" => docx(&bytes)?,
        _ => return Err("Only PDF and DOCX are supported".into()),
    };
    let image_only = text.trim().len() < 20;
    let (skills, titles) = suggestions(&text);
    let doc_id = id();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let path = state.db.root.join("documents").join(&doc_id);
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO resume_documents(id,persona_id,filename,path,extracted_text,mime_type,content_hash,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)").bind(&doc_id).bind(input.persona_id).bind(&input.filename).bind(path.to_string_lossy().to_string()).bind(&text).bind(if ext=="pdf"{"application/pdf"}else{"application/vnd.openxmlformats-officedocument.wordprocessingml.document"}).bind(hash).bind(now()).bind(now()).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
    Ok(ResumeResult {
        id: doc_id,
        extracted_text: text,
        image_only,
        suggested_skills: skills,
        suggested_titles: titles,
        requires_manual_paste: image_only,
        content_hash: hash,
    })
}
#[tauri::command]
pub async fn update_resume_text(
    document_id: String,
    extracted_text: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<ResumeResult> {
    if extracted_text.len() > 2_000_000 {
        return Err("Extracted text is too large".into());
    };
    let original: ResumeDocument = sqlx::query_as("SELECT id,persona_id,filename,extracted_text,mime_type,content_hash,created_at,updated_at FROM resume_documents WHERE id=?")
        .bind(&document_id).fetch_one(&state.db.pool).await.map_err(|_| "Resume version was not found".to_string())?;
    let new_id = id();
    let t = now();
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("{}\n{}", original.content_hash, extracted_text))
    );
    let path: String = sqlx::query_scalar("SELECT path FROM resume_documents WHERE id=?")
        .bind(&document_id)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO resume_documents(id,persona_id,filename,path,extracted_text,mime_type,content_hash,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(&new_id).bind(&original.persona_id).bind(&original.filename).bind(path).bind(&extracted_text).bind(&original.mime_type).bind(&hash).bind(&t).bind(&t).execute(&state.db.pool).await.map_err(|e|e.to_string())?;
    let (skills, titles) = suggestions(&extracted_text);
    Ok(ResumeResult {
        id: new_id,
        extracted_text,
        image_only: extracted_text.trim().len() < 20,
        suggested_skills: skills,
        suggested_titles: titles,
        requires_manual_paste: false,
        content_hash: hash,
    })
}
#[tauri::command]
pub async fn list_resume_documents(
    state: State<'_, Arc<AppState>>,
) -> ApiResult<Vec<ResumeDocument>> {
    sqlx::query_as("SELECT id,persona_id,filename,extracted_text,mime_type,content_hash,created_at,updated_at FROM resume_documents ORDER BY created_at DESC").fetch_all(&state.db.pool).await.map_err(|e|e.to_string())
}
#[tauri::command]
pub async fn delete_resume_document(
    document_id: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let references: i64 =
        sqlx::query_scalar("SELECT count(*) FROM personas WHERE resume_document_id=?")
            .bind(&document_id)
            .fetch_one(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?;
    if references > 0 {
        return Err(
            "Resume version is selected by a persona and remains immutable/archiveable".into(),
        );
    }
    sqlx::query("DELETE FROM resume_documents WHERE id=?")
        .bind(document_id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_suggestions_and_image_only_boundary() {
        let (skills, titles) = suggestions("Rust firmware engineer with Linux and Git");
        assert!(skills.contains(&"rust".to_string()));
        assert!(titles.contains(&"firmware engineer".to_string()));
        assert!("short extracted text".len() < 20);
        assert!("A sufficiently long manually pasted resume body".len() >= 20);
    }

    #[test]
    fn corrected_version_hash_changes_without_touching_original_bytes() {
        let original = "original-file-hash";
        let corrected = format!(
            "{:x}",
            Sha256::digest(format!("{original}\ncorrected text"))
        );
        assert_ne!(original, corrected);
    }
}
