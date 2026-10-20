use crate::{AppState, errors::AppResult, mw::auth::AuthUser};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;

#[derive(Serialize)]
pub struct VolumeView {
    capacity_bytes: Option<u64>,
    used_bytes: u64,
    available_bytes: Option<u64>,
    measurement: String,
    complete: bool,
}
impl From<nyxid_machine::managed::UsageVolume> for VolumeView {
    fn from(v: nyxid_machine::managed::UsageVolume) -> Self {
        Self {
            capacity_bytes: v.capacity_bytes,
            used_bytes: v.used_bytes,
            available_bytes: v.available_bytes,
            measurement: v.measurement,
            complete: v.complete,
        }
    }
}
#[derive(Serialize)]
pub struct UsageView {
    available: bool,
    stale: bool,
    received_at: Option<String>,
    sampled_at: Option<String>,
    workspace: Option<VolumeView>,
    temporary: Option<VolumeView>,
    platform_storage: Option<PlatformStorageView>,
    disk_full: bool,
    write_limit_bytes: usize,
    assistant_arguments_json_limit_bytes: usize,
    edit_file_limit_bytes: usize,
    recovery: &'static str,
}
#[derive(Serialize)]
pub struct StorageComponentView {
    kind: String,
    used_bytes: u64,
    sampled_at: String,
}
#[derive(Serialize)]
pub struct PlatformStorageView {
    source: String,
    used_bytes: Option<u64>,
    capacity_bytes: Option<u64>,
    available_bytes: Option<u64>,
    complete: bool,
    stale: bool,
    received_at: String,
    components: Vec<StorageComponentView>,
    missing_reasons: Vec<String>,
}
impl From<crate::models::remote_machine_storage::PlatformStorageObservation>
    for PlatformStorageView
{
    fn from(o: crate::models::remote_machine_storage::PlatformStorageObservation) -> Self {
        let stale = crate::services::remote_machine_storage_telemetry::observation_stale(
            &o,
            chrono::Utc::now(),
        );
        Self {
            source: o.source,
            used_bytes: o.used_bytes,
            capacity_bytes: None,
            available_bytes: None,
            complete: o.complete,
            stale,
            received_at: o.received_at.to_rfc3339(),
            components: o
                .components
                .into_iter()
                .map(|c| StorageComponentView {
                    kind: c.kind,
                    used_bytes: c.used_bytes,
                    sampled_at: c.sampled_at.to_rfc3339(),
                })
                .collect(),
            missing_reasons: o.missing_reasons,
        }
    }
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(node): Path<String>,
) -> AppResult<Json<UsageView>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let observed = crate::services::remote_machine_usage::owner_observation(
        &state.db,
        &auth.user_id.to_string(),
        &node,
    )
    .await?;
    // Explicit owner usage read only; no platform reads in auth/proxy/MCP.
    // A failed refresh preserves the previous source's own timestamps.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::services::remote_machine_storage_telemetry::refresh_owner(
            &state.db,
            &auth.user_id.to_string(),
            &node,
        ),
    )
    .await;
    let platform_storage = crate::services::remote_machine_storage_telemetry::owner_observation(
        &state.db,
        &auth.user_id.to_string(),
        &node,
    )
    .await?
    .map(PlatformStorageView::from);
    Ok(Json(match observed {
        Some(o) => UsageView {
            platform_storage,
            available: true,
            stale: o.stale,
            received_at: Some(o.received_at.to_rfc3339()),
            sampled_at: chrono::DateTime::from_timestamp_millis(o.usage.sampled_at_ms)
                .map(|time| time.to_rfc3339()),
            workspace: Some(o.usage.workspace.into()),
            temporary: Some(o.usage.temporary.into()),
            disk_full: o.usage.disk_full,
            write_limit_bytes: 1024 * 1024,
            assistant_arguments_json_limit_bytes: 128 * 1024,
            edit_file_limit_bytes: 16 * 1024 * 1024,
            recovery: "The fixed workspace cannot expand. Read files, cancel jobs, or use an authorized shell command to remove your files; then retry the write. The platform does not automatically delete files.",
        },
        None => UsageView {
            platform_storage,
            available: false,
            stale: true,
            received_at: None,
            sampled_at: None,
            workspace: None,
            temporary: None,
            disk_full: false,
            write_limit_bytes: 1024 * 1024,
            assistant_arguments_json_limit_bytes: 128 * 1024,
            edit_file_limit_bytes: 16 * 1024 * 1024,
            recovery: "Usage has not been reported by the current runtime. Workspace capacity is fixed; temporary storage has no fixed allocation. Absence of a report does not mean either is empty.",
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sourced_pod_use_never_invents_an_allocated_capacity() {
        use crate::models::remote_machine_storage::{PlatformStorageObservation, StorageComponent};
        let now = chrono::Utc::now();
        let id = || uuid::Uuid::new_v4().to_string();
        let view = PlatformStorageView::from(PlatformStorageObservation {
            binding: nyxid_machine::managed::Binding {
                node_id: id(),
                sandbox_uid: id(),
                pod_uid: id(),
                runtime_generation: 1,
                credential_epoch: 1,
                writer_fence: id(),
                operation_id: id(),
                delivery_id: id(),
            },
            runtime_id: id(),
            pod_name: "controlled-pod".into(),
            node_name: "worker".into(),
            container_instances: vec![],
            source: "kubelet_summary".into(),
            used_bytes: Some(123),
            complete: true,
            components: vec![StorageComponent {
                kind: "pod_ephemeral_aggregate".into(),
                used_bytes: 123,
                sampled_at: now,
            }],
            missing_reasons: vec![],
            received_at: now,
        });
        let json = serde_json::to_value(view).unwrap();
        assert!(json["capacity_bytes"].is_null());
        assert!(json["available_bytes"].is_null());
        assert_eq!(json["used_bytes"], 123);
        assert!(json["components"][0]["sampled_at"].is_string());
        let volume = VolumeView::from(nyxid_machine::managed::UsageVolume {
            capacity_bytes: None,
            used_bytes: 45,
            available_bytes: None,
            measurement: "node_paths_lower_bound".into(),
            complete: false,
        });
        let json = serde_json::to_value(volume).unwrap();
        assert!(json["capacity_bytes"].is_null());
        assert_eq!(json["complete"], false);
        assert_eq!(json["used_bytes"], 45);
    }
}
