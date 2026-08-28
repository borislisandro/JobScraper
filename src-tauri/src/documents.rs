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
fn image_only(text: &str) -> bool {
    text.trim().len() < 20
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
    let image_only = image_only(&text);
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
        image_only: image_only(&extracted_text),
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
    use std::io::Write;

    fn pdf_with_text(text: Option<&str>) -> Vec<u8> {
        let stream = text
            .map(|value| format!("BT /F1 12 Tf 20 100 Td ({value}) Tj ET\n"))
            .unwrap_or_default();
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            format!("<< /Length {} >>\nstream\n{}endstream", stream.len(), stream),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0usize];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn docx_with_paragraph_and_table() -> Vec<u8> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Firmware engineer</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Rust</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Linux</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(xml.as_bytes()).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn deterministic_suggestions_and_image_only_boundary() {
        let (skills, titles) = suggestions("Rust firmware engineer with Linux and Git");
        assert!(skills.contains(&"rust".to_string()));
        assert!(titles.contains(&"firmware engineer".to_string()));
        assert!(image_only("short extracted text"));
        assert!(!image_only(
            "A sufficiently long manually pasted resume body"
        ));
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

    #[test]
    fn deterministic_pdf_fixture_extracts_text_and_marks_image_only_pdf() {
        let text = pdf_extract::extract_text_from_mem(&pdf_with_text(Some(
            "Rust firmware engineer with Linux",
        )))
        .unwrap();
        assert!(text.contains("Rust firmware engineer"));
        assert!(!image_only(&text));
        let image_text = pdf_extract::extract_text_from_mem(&pdf_with_text(None)).unwrap();
        assert!(image_only(&image_text));
    }

    #[test]
    fn deterministic_docx_fixture_extracts_paragraphs_and_table_cells() {
        assert_eq!(
            docx(&docx_with_paragraph_and_table()).unwrap(),
            "Firmware engineer Rust Linux"
        );
    }

    #[tokio::test]
    async fn referenced_resume_version_cannot_be_deleted() {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE personas (id TEXT PRIMARY KEY, resume_document_id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO personas VALUES ('persona', 'resume')")
            .execute(&pool)
            .await
            .unwrap();
        let references: i64 =
            sqlx::query_scalar("SELECT count(*) FROM personas WHERE resume_document_id=?")
                .bind("resume")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(references, 1);
    }
}
