//! Per-source Playwright storage state encryption. Credentials are never collected or stored.
use crate::{db::ApiResult, AppState};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tauri::State;
fn key() -> ApiResult<[u8; 32]> {
    let entry =
        keyring::Entry::new("JobScraper", "browser-storage-state").map_err(|e| e.to_string())?;
    let encoded = match entry.get_password() {
        Ok(v) => v,
        Err(_) => {
            let value = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
            entry.set_password(&value).map_err(|e| e.to_string())?;
            value
        }
    };
    let digest = Sha256::digest(encoded.as_bytes());
    Ok(digest.into())
}
fn nonce(source: &str) -> [u8; 12] {
    let h = Sha256::digest(format!("JobScraper/session/{source}").as_bytes());
    let mut n = [0; 12];
    n.copy_from_slice(&h[..12]);
    n
}
fn file(root: &std::path::Path, source: &str) -> ApiResult<std::path::PathBuf> {
    if !uuid::Uuid::parse_str(source).is_ok() {
        return Err("Invalid source id".into());
    }
    Ok(root.join("sessions").join(format!("{source}.state")))
}
#[tauri::command]
pub fn save_browser_session(
    source_id: String,
    storage_state_base64: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<()> {
    let plain = STANDARD
        .decode(storage_state_base64)
        .map_err(|_| "Invalid browser storage state")?;
    save_bytes(&state.db.root, &source_id, plain)
}
pub fn save_bytes(root: &std::path::Path, source_id: &str, plain: Vec<u8>) -> ApiResult<()> {
    if plain.len() > 10 * 1024 * 1024 {
        return Err("Browser storage state exceeds 10 MB".into());
    };
    let cipher = Aes256Gcm::new_from_slice(&key()?).map_err(|e| e.to_string())?;
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce(source_id)), plain.as_ref())
        .map_err(|_| "Could not encrypt browser storage state")?;
    std::fs::write(file(root, source_id)?, encrypted).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn load_browser_session(root: &std::path::Path, source: &str) -> ApiResult<Option<Vec<u8>>> {
    let path = file(root, source)?;
    if !path.exists() {
        return Ok(None);
    };
    let cipher = Aes256Gcm::new_from_slice(&key()?).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    cipher
        .decrypt(Nonce::from_slice(&nonce(source)), bytes.as_ref())
        .map(Some)
        .map_err(|_| "Saved browser session cannot be decrypted")
}
#[tauri::command]
pub fn has_browser_session(source_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<bool> {
    Ok(file(&state.db.root, &source_id)?.exists())
}
#[tauri::command]
pub fn delete_browser_session(source_id: String, state: State<'_, Arc<AppState>>) -> ApiResult<()> {
    let path = file(&state.db.root, &source_id)?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?
    }
    Ok(())
}
