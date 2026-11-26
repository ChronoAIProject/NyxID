//! Best-effort chained process identity, independent of listener admission.
use chrono::{DateTime, Utc};
use mongodb::bson::doc;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::Instant;

use crate::{AppState, models::security_replica::COLLECTION_NAME};

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(1);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Deserialize)]
struct PreviousGeneration {
    generation_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    observed_at: DateTime<Utc>,
}

/// Called once after both listener binds. Never awaited by the serving path.
pub(crate) fn listeners_bound(
    state: &AppState,
    bound_at: DateTime<Utc>,
) -> tokio::task::JoinHandle<()> {
    let db = state.db.clone();
    let identity = state.replica_identity.clone();
    let started = Instant::now();
    let commit = std::env::var("NYXID_GIT_HASH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| env!("NYXID_GIT_HASH").to_owned());
    state.shutdown.spawn_cleanup(async move {
        // Registration/heartbeats use one row per generation. Never overwrite or
        // delete another generation; the last observation is evidence, not proof
        // that its process was alive when this process bound its listeners.
        let observations = db.collection::<PreviousGeneration>(COLLECTION_NAME);
        let lookup = observations
            .find_one(doc! {"instance_name": &identity.instance_name, "generation_id": {"$ne": &identity.generation_id}, "observed_at": {"$type":"date"}})
            .sort(doc! {"observed_at": -1, "_id": -1})
            .projection(doc! {"_id":0,"generation_id":1,"observed_at":1})
            .max_time(Duration::from_millis(750));
        let (previous, status) = match tokio::time::timeout_at(started + LOOKUP_TIMEOUT, async { lookup.await }).await {
            Ok(Ok(Some(row))) => (Some(row), "found"),
            Ok(Ok(None)) => (None, "none"),
            _ => (None, "unavailable"),
        };
        let metadata = serde_json::json!({
            "generation": identity.generation_id,
            "instance_name": identity.instance_name,
            "build_commit": commit,
            "listeners_bound_at": bound_at,
            "previous_lookup": status,
            "previous_generation": previous.map(|row| serde_json::json!({"generation":row.generation_id,"last_heartbeat_at":row.observed_at})),
        });
        tracing::info!(target:"nyxid::shutdown", event="backend_process_started", metadata=%metadata, "Backend process listeners bound");
        super::drain::snapshot_audit(db, "backend_process_started", metadata, started + TOTAL_TIMEOUT).await;
    })
}
