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
    pub description: Option<String>,
    pub input_modalities: Option<Vec<String>>,
}

type ModelCache = HashMap<String, (Instant, Option<Vec<ModelInfo>>)>;
pub(crate) static MODELS: LazyLock<Mutex<ModelCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
