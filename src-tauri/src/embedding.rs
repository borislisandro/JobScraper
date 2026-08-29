//! Offline-only BGE loader. FastEmbed v6 `try_new_from_user_defined` receives
//! bytes from the packaged resource; it never reaches hf-hub.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
pub const MODEL_ID: &str = "BAAI/bge-small-en-v1.5";
pub const MODEL_VERSION: &str = "bge-small-en-v1.5/384/v6";
pub const DIMENSIONS: usize = 384;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStatus {
    pub model: String,
    pub dimensions: usize,
    pub ready: bool,
    pub expected_path: String,
    pub downloads_disabled: bool,
}
#[derive(Debug, Serialize)]
#[serde(tag = "code", content = "message")]
pub enum ModelResourceError {
    Missing(String),
    Corrupt(String),
}

fn model_dir(root: &Path) -> PathBuf {
    root.join("models").join("bge-small-en-v1.5")
}
fn resource_files(root: &Path) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf, PathBuf), String> {
    let dir = model_dir(root);
    let files = [
        "model.onnx",
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ];
    let paths: Vec<_> = files.iter().map(|name| dir.join(name)).collect();
    if let Some(missing) = paths.iter().find(|p| !p.is_file()) {
        return Err(serde_json::to_string(&ModelResourceError::Missing(format!(
            "Bundled BGE resource missing: {}",
            missing.display()
        )))
        .unwrap());
    }
    Ok((
        paths[0].clone(),
        paths[1].clone(),
        paths[2].clone(),
        paths[3].clone(),
        paths[4].clone(),
    ))
}
#[tauri::command]
pub fn embedding_status(
    state: tauri::State<'_, std::sync::Arc<crate::AppState>>,
) -> EmbeddingStatus {
    let expected = model_dir(&state.model_root);
    EmbeddingStatus {
        model: MODEL_ID.into(),
        dimensions: DIMENSIONS,
        ready: resource_files(&state.model_root).is_ok(),
        expected_path: expected.display().to_string(),
        downloads_disabled: true,
    }
}
pub fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
pub fn content_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(normalize_text(value).as_bytes()))
}
pub fn chunk_text(text: &str) -> Vec<String> {
    let tokens: Vec<_> = normalize_text(text)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < tokens.len() {
        let end = (start + 350).min(tokens.len());
        chunks.push(tokens[start..end].join(" "));
        if end == tokens.len() {
            break;
        }
        start += 300;
    }
    chunks
}
pub fn l2_normalize(mut vector: Vec<f32>) -> Result<Vec<f32>, String> {
    if vector.len() != DIMENSIONS || vector.iter().any(|value| !value.is_finite()) {
        return Err(serde_json::to_string(&ModelResourceError::Corrupt(
            "BGE output is not 384 finite f32 values".into(),
        ))
        .unwrap());
    }
    let norm = vector
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return Err(serde_json::to_string(&ModelResourceError::Corrupt(
            "BGE output has zero norm".into(),
        ))
        .unwrap());
    }
    for value in &mut vector {
        *value /= norm as f32;
    }
    Ok(vector)
}
pub fn f32_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|v| v.to_le_bytes()).collect()
}
pub fn blob_f32(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() != DIMENSIONS * 4 {
        return Err("Embedding BLOB has invalid length".into());
    }
    let values = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect::<Vec<_>>();
    if values.iter().any(|v| !v.is_finite()) {
        return Err("Embedding BLOB has non-finite value".into());
    }
    Ok(values)
}
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum()
}
pub fn mean_top_three(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len().min(3);
    if n == 0 {
        0.0
    } else {
        values[..n].iter().sum::<f64>() / n as f64
    }
}
pub fn embed_packaged(root: &Path, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
    let (onnx, tokenizer, config, special, tokenizer_config) = resource_files(root)?;
    let model = fastembed::UserDefinedEmbeddingModel::new(
        std::fs::read(onnx).map_err(|e| e.to_string())?,
        fastembed::TokenizerFiles {
            tokenizer_file: std::fs::read(tokenizer).map_err(|e| e.to_string())?,
            config_file: std::fs::read(config).map_err(|e| e.to_string())?,
            special_tokens_map_file: std::fs::read(special).map_err(|e| e.to_string())?,
            tokenizer_config_file: std::fs::read(tokenizer_config).map_err(|e| e.to_string())?,
        },
    )
    .with_pooling(
        fastembed::TextEmbedding::get_default_pooling_method(
            &fastembed::EmbeddingModel::BGESmallENV15,
        )
        .ok_or("BGE pooling unavailable")?,
    );
    let mut engine = fastembed::TextEmbedding::try_new_from_user_defined(
        model,
        fastembed::InitOptionsUserDefined::new(),
    )
    .map_err(|e| serde_json::to_string(&ModelResourceError::Corrupt(e.to_string())).unwrap())?;
    let mut all = Vec::new();
    for batch in texts.chunks(32) {
        all.extend(
            engine
                .embed(batch.to_vec(), None)
                .map_err(|e| e.to_string())?,
        );
    }
    all.into_iter().map(l2_normalize).collect()
}
#[tauri::command]
pub fn embedding_smoke(
    state: tauri::State<'_, std::sync::Arc<crate::AppState>>,
) -> Result<EmbeddingStatus, String> {
    let output = embed_packaged(&state.model_root, vec!["offline package smoke".into()])?;
    if output.len() != 1 || output[0].len() != DIMENSIONS {
        return Err("Bundled BGE smoke returned an invalid vector count or dimension".into());
    }
    Ok(EmbeddingStatus {
        model: MODEL_ID.into(),
        dimensions: DIMENSIONS,
        ready: true,
        expected_path: model_dir(&state.model_root).display().to_string(),
        downloads_disabled: true,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunks_overlap_and_bound() {
        let input = (0..701)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = chunk_text(&input);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].split_whitespace().count(), 350);
        assert_eq!(
            chunks[0].split_whitespace().last(),
            chunks[1].split_whitespace().nth(49)
        );
    }
    #[test]
    fn blob_round_trip_and_corruption() {
        let v = vec![1.0f32 / (DIMENSIONS as f32).sqrt(); DIMENSIONS];
        assert_eq!(blob_f32(&f32_blob(&v)).unwrap().len(), DIMENSIONS);
        assert!(blob_f32(&[0; 3]).is_err());
        let mut nan = f32_blob(&vec![0.0; DIMENSIONS]);
        nan[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(blob_f32(&nan).is_err());
        assert!(l2_normalize(vec![0.0; DIMENSIONS]).is_err());
    }
    #[test]
    fn top_three() {
        assert!((mean_top_three(vec![0.1, 0.9, 0.7, 0.8]) - 0.8).abs() < 1e-9);
    }
    #[test]
    #[ignore = "requires prepared offline release model"]
    fn packaged_bge_smoke_is_384_dimensions() {
        let root = std::env::var("JOBSCRAPER_MODEL_RESOURCE_ROOT")
            .expect("set JOBSCRAPER_MODEL_RESOURCE_ROOT to packaged resource root");
        let vectors = embed_packaged(Path::new(&root), vec!["offline package smoke".into()])
            .expect("prepared model must load without a network request");
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].len(), DIMENSIONS);
    }
}
