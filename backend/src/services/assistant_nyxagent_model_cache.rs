//! Shared owner-scoped model metadata. Discovery owns refresh; image turns can
//! read the same entry even when their transport has no account-level loader.
use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct ModelInfo {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub inference_credential: Option<InferenceCredential>,
    pub description: Option<String>,
    pub input_modalities: Option<Vec<String>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InferenceCredential {
    Thread,
    Platform,
}

type ModelCache = HashMap<String, (Instant, Option<Vec<ModelInfo>>)>;
pub(crate) static MODELS: LazyLock<Mutex<ModelCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) async fn load(
    db: &mongodb::Database,
    owner: &str,
    fetch: impl Future<Output = crate::errors::AppResult<axum::response::Response>>,
) -> Option<Vec<ModelInfo>> {
    let key = format!("{}:{owner}", db.name());
    if let Some((_, cached)) = MODELS
        .lock()
        .await
        .get(&key)
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
    {
        return cached.clone();
    }
    let rows = async {
        let response = tokio::time::timeout(Duration::from_secs(10), fetch)
            .await
            .ok()?
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = tokio::time::timeout(
            Duration::from_secs(10),
            axum::body::to_bytes(response.into_body(), 65536),
        )
        .await
        .ok()?
        .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let rows: Vec<_> = value["data"]
            .as_array()?
            .iter()
            .filter_map(ModelInfo::parse)
            .take(50)
            .collect();
        (!rows.is_empty()).then_some(rows)
    }
    .await;
    let mut cache = MODELS.lock().await;
    if cache.len() >= 64 {
        cache.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(60));
        if cache.len() >= 64
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (time, _))| *time)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
    }
    cache.insert(key, (Instant::now(), rows.clone()));
    rows
}

/// Outer None is a cache miss; inner None is cached unknown/failed discovery.
pub(super) async fn image_input(
    db: &mongodb::Database,
    owner: &str,
    model: &str,
) -> Option<Option<bool>> {
    let key = format!("{}:{owner}", db.name());
    let cache = MODELS.lock().await;
    let (_, models) = cache
        .get(&key)
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))?;
    Some(
        models
            .as_ref()
            .and_then(|models| models.iter().find(|row| row.id == model))
            .and_then(|row| row.input_modalities.as_ref())
            .map(|modalities| modalities.iter().any(|value| value == "image")),
    )
}

/// Keep upstream display metadata; the routing alias is not a model name.
impl ModelInfo {
    pub(crate) fn parse(row: &serde_json::Value) -> Option<Self> {
        let id = row["id"]
            .as_str()
            .filter(|id| super::assistant_nyxagent::valid_legacy_model(id))?;
        let label = ["display_name", "label", "name"]
            .into_iter()
            .filter_map(|key| row[key].as_str())
            .map(str::trim)
            .find(|name| !name.is_empty());
        Some(Self {
            id: id.into(),
            label: label
                .unwrap_or_else(|| {
                    if id == super::assistant_nyxagent::DEFAULT_MODEL {
                        "Default model"
                    } else {
                        id.trim_start_matches("nyxagent/")
                    }
                })
                .into(),
            available: row["available"].as_bool() != Some(false),
            inference_credential: match row["inference_credential"].as_str() {
                Some("thread") => Some(InferenceCredential::Thread),
                Some("platform") => Some(InferenceCredential::Platform),
                _ => None,
            },
            description: row["description"].as_str().map(str::to_owned),
            input_modalities: input_modalities(row),
        })
    }
}

// Malformed optional metadata is unknown, never proof that images are unsupported.
fn input_modalities(row: &serde_json::Value) -> Option<Vec<String>> {
    let values = row["input_modalities"]
        .as_array()
        .filter(|values| values.len() <= 16)?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| {
                    !value.is_empty()
                        && value.len() <= 32
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                })
                .map(str::to_owned)
        })
        .collect()
}
