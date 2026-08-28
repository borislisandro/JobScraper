//! Per-source Playwright storage state encryption. Credentials are never collected or stored.
use crate::{db::ApiResult, AppState};
use aes_gcm::{
    aead::{generic_array::GenericArray, rand_core::RngCore, Aead, KeyInit, OsRng, Payload},
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
const FORMAT: &[u8; 4] = b"JS2\0";
fn context(source: &str) -> Vec<u8> {
    format!("JobScraper/session/v2/{source}").into_bytes()
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
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(
            GenericArray::from_slice(&nonce),
            Payload {
                msg: plain.as_ref(),
                aad: &context(source_id),
            },
        )
        .map_err(|_| "Could not encrypt browser storage state")?;
    let mut stored = FORMAT.to_vec();
    stored.extend_from_slice(&nonce);
    stored.extend_from_slice(&encrypted);
    std::fs::write(file(root, source_id)?, stored).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn load_browser_session(root: &std::path::Path, source: &str) -> ApiResult<Option<Vec<u8>>> {
    let path = file(root, source)?;
    if !path.exists() {
        return Ok(None);
    };
    let cipher = Aes256Gcm::new_from_slice(&key()?).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if !bytes.starts_with(FORMAT) {
        return Err("Legacy browser session format rejected; capture a new session".into());
    }
    if bytes.len() <= FORMAT.len() + 12 {
        return Err("Browser session blob is truncated".into());
    }
    let nonce = &bytes[FORMAT.len()..FORMAT.len() + 12];
    let ciphertext = &bytes[FORMAT.len() + 12..];
    cipher
        .decrypt(
            GenericArray::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &context(source),
            },
        )
        .map(Some)
        .map_err(|_| "Saved browser session cannot be decrypted or belongs to another source")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v2_format_uses_random_nonce_and_authenticates_source() {
        let root =
            std::env::temp_dir().join(format!("jobscraper-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        let one = "00000000-0000-4000-8000-000000000001";
        let two = "00000000-0000-4000-8000-000000000002";
        save_bytes(&root, one, b"first".to_vec()).unwrap();
        let first = std::fs::read(file(&root, one).unwrap()).unwrap();
        save_bytes(&root, one, b"second".to_vec()).unwrap();
        let second = std::fs::read(file(&root, one).unwrap()).unwrap();
        assert_ne!(&first[4..16], &second[4..16]);
        assert_eq!(
            load_browser_session(&root, one).unwrap(),
            Some(b"second".to_vec())
        );
        std::fs::write(file(&root, two).unwrap(), second).unwrap();
        assert!(load_browser_session(&root, two).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
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
