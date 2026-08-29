//! Versioned, checksummed backups and crash-recoverable restores.
//!
//! A restore is deliberately a two-process operation: the UI validates and
//! stages an archive, then the next process swaps it before SQLite is opened.
//! Browser sessions, model files, logs and caches are never part of either
//! archive or replacement set.
use crate::{db::ApiResult, AppState};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::State;

pub const BACKUP_VERSION: u32 = 1;
/// Bump when a backup can no longer be restored by this build.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;
const INTENT_FILE: &str = "restore-intent.json";
const STATUS_FILE: &str = "restore-last.json";

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct Manifest {
    pub version: u32,
    pub schema_version: i64,
    pub database_sha256: String,
    pub includes_documents: bool,
    pub excludes: Vec<String>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct RestoreIntent {
    pub version: u32,
    pub archive_sha256: String,
    pub manifest_sha256: String,
    pub schema_version: i64,
    pub staging_path: String,
    /// ready, applying, or applied. `applying` is a durable swap journal.
    pub state: String,
    pub phase: String,
    pub safety_snapshot: Option<String>,
    pub rollback_database: Option<String>,
    pub rollback_documents: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct RestoreStageResult {
    pub restart_required: bool,
    pub staging_path: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct StartupRestoreResult {
    pub applied: bool,
    pub restart_required: bool,
    pub message: String,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn manifest_bytes(manifest: &Manifest) -> Result<Vec<u8>, String> {
    serde_json::to_vec(manifest).map_err(|e| e.to_string())
}

pub fn manifest(db: &[u8], schema_version: i64) -> Manifest {
    Manifest {
        version: BACKUP_VERSION,
        schema_version,
        database_sha256: hash(db),
        includes_documents: true,
        excludes: vec![
            "sessions/".into(),
            "keys/".into(),
            "models/".into(),
            "logs/".into(),
            "cache/".into(),
            "temp/".into(),
        ],
        files: vec![FileEntry {
            path: "snapshot/jobscraper.sqlite".into(),
            size: db.len() as u64,
            sha256: hash(db),
        }],
    }
}

fn safe_archive_path(value: &str) -> Result<(), String> {
    let p = Path::new(value);
    if value.is_empty()
        || p.is_absolute()
        || value.contains('\\')
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("Unsafe archive path".into());
    }
    Ok(())
}

fn controlled(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| "Document path is outside the controlled documents directory")?;
    if relative.as_os_str().is_empty() {
        return Err("Invalid document path".into());
    }
    Ok(relative.to_path_buf())
}

fn add_file(manifest: &mut Manifest, archive_path: String, bytes: &[u8]) -> Result<(), String> {
    safe_archive_path(&archive_path)?;
    if manifest
        .files
        .iter()
        .any(|entry| entry.path == archive_path)
    {
        return Err("Duplicate normalized document path".into());
    }
    manifest.files.push(FileEntry {
        path: archive_path,
        size: bytes.len() as u64,
        sha256: hash(bytes),
    });
    Ok(())
}

pub fn validate(manifest: &Manifest, db: &[u8]) -> Result<(), String> {
    if manifest.version != BACKUP_VERSION {
        return Err(format!("Unsupported backup version {}", manifest.version));
    }
    if manifest.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported backup schema {}",
            manifest.schema_version
        ));
    }
    if manifest.database_sha256 != hash(db) {
        return Err("Backup checksum mismatch".into());
    }
    let mut paths = BTreeSet::new();
    for file in &manifest.files {
        safe_archive_path(&file.path)?;
        if !paths.insert(file.path.clone()) {
            return Err("Duplicate normalized manifest path".into());
        }
    }
    Ok(())
}

pub fn write_archive(
    destination: &Path,
    db: &[u8],
    schema_version: i64,
    documents: &[(PathBuf, Vec<u8>)],
) -> Result<(), String> {
    let mut manifest = manifest(db, schema_version);
    for (path, bytes) in documents {
        add_file(
            &mut manifest,
            format!("documents/{}", path.to_string_lossy().replace('\\', "/")),
            bytes,
        )?;
    }
    let file = fs::File::create(destination).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("manifest.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(&manifest_bytes(&manifest)?)
        .map_err(|e| e.to_string())?;
    zip.start_file("snapshot/jobscraper.sqlite", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(db).map_err(|e| e.to_string())?;
    for (path, bytes) in documents {
        zip.start_file(
            format!("documents/{}", path.to_string_lossy().replace('\\', "/")),
            options,
        )
        .map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Fully validates the ZIP before any archive content is trusted.
pub fn read_archive(bytes: &[u8]) -> Result<(Manifest, Vec<u8>), String> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "Invalid backup archive")?;
    let mut raw_manifest = Vec::new();
    zip.by_name("manifest.json")
        .map_err(|_| "Backup manifest missing")?
        .read_to_end(&mut raw_manifest)
        .map_err(|e| e.to_string())?;
    let manifest: Manifest =
        serde_json::from_slice(&raw_manifest).map_err(|_| "Backup manifest invalid")?;
    let mut db = Vec::new();
    zip.by_name("snapshot/jobscraper.sqlite")
        .map_err(|_| "Backup snapshot missing")?
        .read_to_end(&mut db)
        .map_err(|e| e.to_string())?;
    validate(&manifest, &db)?;
    let expected: BTreeSet<String> = std::iter::once("manifest.json".into())
        .chain(manifest.files.iter().map(|f| f.path.clone()))
        .collect();
    let mut seen = BTreeSet::new();
    for i in 0..zip.len() {
        let file = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().to_string();
        safe_archive_path(&name)?;
        if !expected.contains(&name) {
            return Err("Unexpected archive file".into());
        }
        if !seen.insert(name) {
            return Err("Duplicate archive path".into());
        }
    }
    if seen != expected {
        return Err("Missing manifest file".into());
    }
    for entry in &manifest.files {
        let mut file = zip
            .by_name(&entry.path)
            .map_err(|_| "Missing manifest file")?;
        let mut body = Vec::new();
        file.read_to_end(&mut body).map_err(|e| e.to_string())?;
        if body.len() as u64 != entry.size || hash(&body) != entry.sha256 {
            return Err("Archive file checksum or size mismatch".into());
        }
    }
    Ok((manifest, db))
}

fn intent_path(root: &Path) -> PathBuf {
    root.join(INTENT_FILE)
}
fn status_path(root: &Path) -> PathBuf {
    root.join(STATUS_FILE)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};
    if !destination.exists() {
        return fs::rename(source, destination).map_err(|e| e.to_string());
    }
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // MoveFileExW replaces a file on this volume without delete-then-rename gap.
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|e| e.to_string())
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    replace_file(&temporary, path)
}
fn write_status(root: &Path, result: &StartupRestoreResult) {
    let _ = atomic_json(&status_path(root), result);
}
fn safe_staging(root: &Path, staged: &Path) -> Result<PathBuf, String> {
    let relative = controlled(root, staged)?;
    if relative
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        != Some("restore-staging")
    {
        return Err("Restore staging must remain under restore-staging".into());
    }
    Ok(relative)
}
fn archive_to_stage_path(stage: &Path, archive_path: &str) -> Result<PathBuf, String> {
    safe_archive_path(archive_path)?;
    if archive_path == "snapshot/jobscraper.sqlite" {
        return Ok(stage.join("jobscraper.db"));
    }
    let relative = archive_path
        .strip_prefix("documents/")
        .ok_or("Unexpected archive payload")?;
    safe_archive_path(relative)?;
    Ok(stage.join("documents").join(relative))
}

fn extract_archive(bytes: &[u8], root: &Path) -> Result<(Manifest, PathBuf), String> {
    let (manifest, _) = read_archive(bytes)?;
    let stage = root
        .join("restore-staging")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|_| "Invalid backup archive")?;
        for entry in &manifest.files {
            let destination = archive_to_stage_path(&stage, &entry.path)?;
            let mut source = zip
                .by_name(&entry.path)
                .map_err(|_| "Missing manifest file")?;
            let mut body = Vec::new();
            source.read_to_end(&mut body).map_err(|e| e.to_string())?;
            if body.len() as u64 != entry.size || hash(&body) != entry.sha256 {
                return Err("Archive changed during extraction".into());
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&destination, &body).map_err(|e| e.to_string())?;
            let rehashed = fs::read(&destination).map_err(|e| e.to_string())?;
            if rehashed.len() as u64 != entry.size || hash(&rehashed) != entry.sha256 {
                return Err("Staged file checksum mismatch".into());
            }
        }
        fs::create_dir_all(stage.join("documents")).map_err(|e| e.to_string())?;
        fs::write(stage.join("manifest.json"), manifest_bytes(&manifest)?)
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result.map(|_| (manifest, stage))
}

fn validate_staged(intent: &RestoreIntent, stage: &Path) -> Result<Manifest, String> {
    let manifest_bytes =
        fs::read(stage.join("manifest.json")).map_err(|_| "Staged manifest missing")?;
    if hash(&manifest_bytes) != intent.manifest_sha256 {
        return Err("Staged manifest hash mismatch".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "Staged manifest invalid")?;
    if manifest.version != intent.version || manifest.schema_version != intent.schema_version {
        return Err("Staged manifest version mismatch".into());
    }
    let database = fs::read(stage.join("jobscraper.db")).map_err(|_| "Staged database missing")?;
    validate(&manifest, &database)?;
    for entry in &manifest.files {
        let path = archive_to_stage_path(stage, &entry.path)?;
        let content = fs::read(path).map_err(|_| "Staged manifest file missing")?;
        if content.len() as u64 != entry.size || hash(&content) != entry.sha256 {
            return Err("Staged manifest file checksum mismatch".into());
        }
    }
    Ok(manifest)
}

fn collect_documents(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    fn visit(
        base: &Path,
        current: &Path,
        result: &mut Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<(), String> {
        for item in fs::read_dir(current).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let path = item.path();
            if path.is_dir() {
                visit(base, &path, result)?;
            } else if path.is_file() {
                let relative = controlled(base, &path)?;
                result.push((relative, fs::read(path).map_err(|e| e.to_string())?));
            }
        }
        Ok(())
    }
    let mut result = Vec::new();
    if root.exists() {
        visit(root, root, &mut result)?;
    }
    Ok(result)
}
fn safety_snapshot(root: &Path, schema: i64) -> Result<PathBuf, String> {
    let backups = root.join("backups");
    fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
    let db = fs::read(root.join("jobscraper.db"))
        .map_err(|_| "Live database missing for safety snapshot")?;
    let docs_root = root.join("documents");
    fs::create_dir_all(&docs_root).map_err(|e| e.to_string())?;
    let destination = backups.join(format!(
        "pre-restore-{}.zip",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    write_archive(&destination, &db, schema, &collect_documents(&docs_root)?)?;
    Ok(destination)
}
fn duplicate_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        fs::remove_dir_all(destination).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for item in fs::read_dir(source).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let target = destination.join(item.file_name());
        if item.path().is_dir() {
            duplicate_tree(&item.path(), &target)?;
        } else {
            fs::copy(item.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn rollback(root: &Path, intent: &RestoreIntent) -> Result<(), String> {
    let live_db = root.join("jobscraper.db");
    let live_docs = root.join("documents");
    let old_db = intent.rollback_database.as_ref().map(PathBuf::from);
    let old_docs = intent.rollback_documents.as_ref().map(PathBuf::from);
    if let Some(old) = old_db.filter(|p| p.exists()) {
        if live_db.exists() {
            fs::remove_file(&live_db).map_err(|e| e.to_string())?;
        }
        fs::rename(old, &live_db).map_err(|e| e.to_string())?;
    }
    if let Some(old) = old_docs.filter(|p| p.exists()) {
        if live_docs.exists() {
            fs::remove_dir_all(&live_docs).map_err(|e| e.to_string())?;
        }
        fs::rename(old, &live_docs).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn apply_intent(
    root: &Path,
    mut intent: RestoreIntent,
    inject_after_db: bool,
) -> Result<StartupRestoreResult, String> {
    let staged = PathBuf::from(&intent.staging_path);
    safe_staging(root, &staged)?;
    let staged_db = staged.join("jobscraper.db");
    let staged_docs = staged.join("documents");
    if !staged_db.exists() || !staged_docs.exists() {
        return Err("Validated restore staging is incomplete".into());
    }
    validate_staged(&intent, &staged)?;
    if intent.state == "applying" {
        if intent.phase == "documents_replaced"
            && root.join("jobscraper.db").exists()
            && root.join("documents").exists()
        {
            if let Some(old) = &intent.rollback_database {
                let _ = fs::remove_file(old);
            }
            if let Some(old) = &intent.rollback_documents {
                let _ = fs::remove_dir_all(old);
            }
            let _ = fs::remove_dir_all(&staged);
            let _ = fs::remove_file(intent_path(root));
            let result = StartupRestoreResult {
                applied: true,
                restart_required: false,
                message: "Recovered completed restore after interruption.".into(),
            };
            write_status(root, &result);
            return Ok(result);
        }
        rollback(root, &intent)?;
        intent.state = "ready".into();
        intent.phase = "staged".into();
        atomic_json(&intent_path(root), &intent)?;
    }
    let safety = safety_snapshot(root, intent.schema_version)?;
    let token = uuid::Uuid::new_v4();
    let old_db = root.join(format!(".restore-previous-{token}.sqlite"));
    let old_docs = root.join(format!(".restore-previous-documents-{token}"));
    let apply_db = staged.join("apply-jobscraper.db");
    let apply_docs = staged.join("apply-documents");
    fs::copy(&staged_db, &apply_db).map_err(|e| e.to_string())?;
    duplicate_tree(&staged_docs, &apply_docs)?;
    intent.state = "applying".into();
    intent.phase = "prepared".into();
    intent.safety_snapshot = Some(safety.display().to_string());
    intent.rollback_database = Some(old_db.display().to_string());
    intent.rollback_documents = Some(old_docs.display().to_string());
    atomic_json(&intent_path(root), &intent)?;
    let swap: Result<(), String> = (|| {
        fs::rename(root.join("jobscraper.db"), &old_db).map_err(|e| e.to_string())?;
        fs::rename(root.join("documents"), &old_docs).map_err(|e| e.to_string())?;
        intent.phase = "live_moved".into();
        atomic_json(&intent_path(root), &intent)?;
        fs::rename(&apply_db, root.join("jobscraper.db")).map_err(|e| e.to_string())?;
        intent.phase = "database_replaced".into();
        atomic_json(&intent_path(root), &intent)?;
        if inject_after_db {
            return Err("Injected failure after database replacement".into());
        }
        fs::rename(&apply_docs, root.join("documents")).map_err(|e| e.to_string())?;
        intent.phase = "documents_replaced".into();
        atomic_json(&intent_path(root), &intent)?;
        Ok(())
    })();
    if let Err(error) = swap {
        let rollback_result = rollback(root, &intent);
        let diagnostic = StartupRestoreResult {
            applied: false,
            restart_required: true,
            message: format!(
                "Restore rolled back: {error}; {}",
                rollback_result
                    .as_ref()
                    .err()
                    .map(String::as_str)
                    .unwrap_or("rollback succeeded")
            ),
        };
        write_status(root, &diagnostic);
        // A successful rollback restores a coherent live DB/documents pair, so
        // startup may continue and let the user retry the retained intent.
        rollback_result?;
        return Ok(diagnostic);
    }
    let _ = fs::remove_file(&old_db);
    let _ = fs::remove_dir_all(&old_docs);
    let _ = fs::remove_dir_all(&staged);
    let _ = fs::remove_file(intent_path(root));
    let result = StartupRestoreResult {
        applied: true,
        restart_required: false,
        message: "Validated restore applied before opening SQLite.".into(),
    };
    write_status(root, &result);
    Ok(result)
}

/// Called by Tauri setup before `Database::open`.
pub fn apply_pending_restore(root: &Path) -> Result<StartupRestoreResult, String> {
    let intent_file = intent_path(root);
    if !intent_file.exists() {
        return Ok(StartupRestoreResult {
            applied: false,
            restart_required: false,
            message: "No pending restore.".into(),
        });
    }
    let result = (|| {
        let intent: RestoreIntent =
            serde_json::from_slice(&fs::read(&intent_file).map_err(|e| e.to_string())?)
                .map_err(|_| "Restore intent is invalid")?;
        if intent.version != BACKUP_VERSION {
            return Err("Restore intent version is incompatible".into());
        }
        apply_intent(root, intent, false)
    })();
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let value = StartupRestoreResult {
                applied: false,
                restart_required: true,
                message: error.clone(),
            };
            write_status(root, &value);
            // Do not open SQLite after an unrecoverable partial replacement.
            Err(error)
        }
    }
}
fn stage_archive_bytes(root: &Path, bytes: &[u8]) -> Result<RestoreStageResult, String> {
    fs::create_dir_all(root.join("restore-staging")).map_err(|e| e.to_string())?;
    if intent_path(root).exists() {
        return Err("A validated restore is already pending".into());
    }
    let (manifest, stage) = extract_archive(bytes, root)?;
    let intent = RestoreIntent {
        version: BACKUP_VERSION,
        archive_sha256: hash(bytes),
        manifest_sha256: hash(&manifest_bytes(&manifest)?),
        schema_version: manifest.schema_version,
        staging_path: stage.display().to_string(),
        state: "ready".into(),
        phase: "staged".into(),
        safety_snapshot: None,
        rollback_database: None,
        rollback_documents: None,
    };
    if let Err(error) = atomic_json(&intent_path(root), &intent) {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    Ok(RestoreStageResult {
        restart_required: true,
        staging_path: stage.display().to_string(),
        message: "Restore archive validated and staged. Restart to apply it before SQLite opens."
            .into(),
    })
}

#[tauri::command]
pub fn validate_restore_archive(archive_base64: String) -> ApiResult<Manifest> {
    let bytes = STANDARD
        .decode(archive_base64)
        .map_err(|_| "Invalid backup bytes")?;
    Ok(read_archive(&bytes)?.0)
}
#[tauri::command]
pub async fn create_backup_archive(state: State<'_, Arc<AppState>>) -> ApiResult<String> {
    let dir = state.db.root.join("backups");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let snapshot = dir.join(format!("snapshot-{stamp}.sqlite"));
    sqlx::query("VACUUM INTO ?")
        .bind(snapshot.to_string_lossy().to_string())
        .execute(&state.db.pool)
        .await
        .map_err(|e| e.to_string())?;
    let result = async {
        let bytes = fs::read(&snapshot).map_err(|e| e.to_string())?;
        let schema: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&state.db.pool)
            .await
            .unwrap_or(CURRENT_SCHEMA_VERSION);
        let root = state.db.root.join("documents");
        let mut documents = Vec::new();
        for row in sqlx::query("SELECT path FROM resume_documents")
            .fetch_all(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?
        {
            let stored: String = row.get(0);
            let relative = controlled(&root, Path::new(&stored))?;
            documents.push((relative, fs::read(stored).map_err(|e| e.to_string())?));
        }
        for row in sqlx::query("SELECT id,filename,content FROM application_documents")
            .fetch_all(&state.db.pool)
            .await
            .map_err(|e| e.to_string())?
        {
            let id: String = row.get(0);
            let name: String = row.get(1);
            let body: Vec<u8> = row.get(2);
            documents.push((
                PathBuf::from("application").join(format!("{id}-{name}")),
                body,
            ));
        }
        let archive = dir.join(format!("jobscraper-{stamp}.zip"));
        write_archive(&archive, &bytes, schema, &documents)?;
        Ok(archive.display().to_string())
    }
    .await;
    let _ = fs::remove_file(snapshot);
    result
}
#[tauri::command]
pub fn stage_restore_archive(
    archive_base64: String,
    state: State<'_, Arc<AppState>>,
) -> ApiResult<RestoreStageResult> {
    let bytes = STANDARD
        .decode(archive_base64)
        .map_err(|_| "Invalid backup bytes")?;
    stage_archive_bytes(&state.db.root, &bytes)
}
#[tauri::command]
pub fn restore_status(state: State<'_, Arc<AppState>>) -> ApiResult<Option<StartupRestoreResult>> {
    let file = status_path(&state.db.root);
    if !file.exists() {
        return Ok(None);
    }
    serde_json::from_slice(&fs::read(file).map_err(|e| e.to_string())?)
        .map(Some)
        .map_err(|_| "Restore status is invalid".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_zip(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(name, options).unwrap();
            zip.write_all(&content).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("jobscraper-backup-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("documents/resume")).unwrap();
        fs::write(root.join("jobscraper.db"), b"live-db").unwrap();
        fs::write(root.join("documents/resume/live.txt"), b"live-document").unwrap();
        fs::create_dir_all(root.join("sessions")).unwrap();
        fs::write(root.join("sessions/keep.txt"), b"keep").unwrap();
        root
    }
    fn archive(root: &Path) -> Vec<u8> {
        let file = root.join("input.zip");
        write_archive(
            &file,
            b"restored-db",
            CURRENT_SCHEMA_VERSION,
            &[(
                PathBuf::from("resume/new.txt"),
                b"restored-document".to_vec(),
            )],
        )
        .unwrap();
        fs::read(file).unwrap()
    }
    #[test]
    fn round_trip_and_validation_rejects_wrong_schema() {
        let root = root();
        let bytes = archive(&root);
        assert_eq!(
            read_archive(&bytes).unwrap().0.schema_version,
            CURRENT_SCHEMA_VERSION
        );
        assert!(validate(&manifest(b"sqlite", CURRENT_SCHEMA_VERSION + 1), b"sqlite").is_err());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn staging_and_atomic_apply_preserve_excluded_directories() {
        let root = root();
        let bytes = archive(&root);
        let staged = stage_archive_bytes(&root, &bytes).unwrap();
        assert!(Path::new(&staged.staging_path)
            .join("documents/resume/new.txt")
            .exists());
        let result = apply_pending_restore(&root).unwrap();
        assert!(result.applied, "{}", result.message);
        assert_eq!(
            fs::read(root.join("jobscraper.db")).unwrap(),
            b"restored-db"
        );
        assert_eq!(
            fs::read(root.join("documents/resume/new.txt")).unwrap(),
            b"restored-document"
        );
        assert_eq!(fs::read(root.join("sessions/keep.txt")).unwrap(), b"keep");
        assert!(fs::read_dir(root.join("backups")).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("pre-restore-")));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn injected_failure_rolls_back_and_is_restart_idempotent() {
        let root = root();
        let bytes = archive(&root);
        stage_archive_bytes(&root, &bytes).unwrap();
        let intent: RestoreIntent =
            serde_json::from_slice(&fs::read(intent_path(&root)).unwrap()).unwrap();
        assert!(!apply_intent(&root, intent, true).unwrap().applied);
        assert_eq!(fs::read(root.join("jobscraper.db")).unwrap(), b"live-db");
        assert_eq!(
            fs::read(root.join("documents/resume/live.txt")).unwrap(),
            b"live-document"
        );
        assert!(intent_path(&root).exists());
        assert!(apply_pending_restore(&root).unwrap().applied);
        assert!(apply_pending_restore(&root)
            .unwrap()
            .message
            .contains("No pending"));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn traversal_duplicate_missing_extra_and_checksum_are_rejected() {
        let root = root();
        let bytes = archive(&root);
        let (mut archive_manifest, _) = read_archive(&bytes).unwrap();
        archive_manifest
            .files
            .push(archive_manifest.files[0].clone());
        assert!(validate(&archive_manifest, b"restored-db").is_err());
        assert!(safe_archive_path("documents/../escape").is_err());
        assert!(safe_archive_path("C:/escape").is_err());
        let mut corrupt = bytes.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        assert!(read_archive(&corrupt).is_err());
        let clean_manifest = manifest(b"restored-db", CURRENT_SCHEMA_VERSION);
        let manifest_json = manifest_bytes(&clean_manifest).unwrap();
        assert!(read_archive(&raw_zip(vec![
            ("manifest.json", manifest_json.clone()),
            ("snapshot/jobscraper.sqlite", b"restored-db".to_vec()),
            ("unexpected.txt", b"x".to_vec()),
        ]))
        .is_err());
        // zip 2.x rejects duplicate entry names while writing. That is the first
        // boundary; read_archive also tracks names for archives made elsewhere.
        let cursor = std::io::Cursor::new(Vec::new());
        let mut duplicate = zip::ZipWriter::new(cursor);
        duplicate
            .start_file(
                "snapshot/jobscraper.sqlite",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        duplicate.write_all(b"restored-db").unwrap();
        assert!(duplicate
            .start_file(
                "snapshot/jobscraper.sqlite",
                zip::write::SimpleFileOptions::default()
            )
            .is_err());
        assert!(read_archive(&raw_zip(vec![("manifest.json", manifest_json)])).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staged_tampering_rejects_apply_before_live_state_changes() {
        let root = root();
        let staged = stage_archive_bytes(&root, &archive(&root)).unwrap();
        fs::write(
            Path::new(&staged.staging_path).join("jobscraper.db"),
            b"changed",
        )
        .unwrap();
        assert!(apply_pending_restore(&root).is_err());
        assert_eq!(fs::read(root.join("jobscraper.db")).unwrap(), b"live-db");
        let _ = fs::remove_dir_all(root);
    }
}
