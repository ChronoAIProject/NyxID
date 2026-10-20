//! Optional owner/operator Pod telemetry uses trusted identity and a separately
//! configured kubelet TLS/audience credential. It is never lifecycle admission
//! or workspace-usage authority. An absent collector leaves Pod usage unknown;
//! normal Machine operations and Node workspace reports remain available.
//! Never use nodes/proxy or workload URLs.
use super::remote_machine_kubernetes_identity::KubernetesWorkloadVerifier;
use crate::{
    errors::{AppError, AppResult},
    models::{
        remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine},
        remote_machine_storage::{PlatformStorageObservation, StorageComponent},
    },
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use nyxid_machine::managed::Binding;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

const MAX_SUMMARY: usize = 8 * 1024 * 1024;
const MAX_PODS: usize = 1000;
const MAX_AGE: i64 = 90;
// Bound expensive platform reads independently of the number of owner tabs.
static COLLECTION_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

fn unavailable() -> AppError {
    AppError::NodeOffline("platform storage telemetry unavailable".into())
}
fn invalid() -> AppError {
    AppError::MachineAuthorityStale
}
fn timestamp(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}
fn stale(sample: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    sample > now + chrono::Duration::seconds(30)
        || sample < now - chrono::Duration::seconds(MAX_AGE)
}
fn name(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 253
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

struct Target {
    binding: Binding,
    runtime_id: String,
    namespace: String,
    pod_name: String,
    node_name: String,
    containers: BTreeMap<String, (String, DateTime<Utc>)>,
    disk_volumes: BTreeSet<String>,
}

fn target(machine: &ManagedMachine, runtime_id: String, pod: &Value) -> AppResult<Target> {
    let binding = Binding {
        node_id: machine.node_id.clone(),
        sandbox_uid: machine.sandbox_uid.clone().ok_or_else(invalid)?,
        pod_uid: machine.pod_uid.clone().ok_or_else(invalid)?,
        runtime_generation: machine
            .runtime_generation
            .try_into()
            .map_err(|_| invalid())?,
        credential_epoch: machine.credential_epoch.try_into().map_err(|_| invalid())?,
        writer_fence: machine.writer_fence.clone(),
        operation_id: machine.active_operation_id.clone().ok_or_else(invalid)?,
        delivery_id: machine.bootstrap_delivery_id.clone().ok_or_else(invalid)?,
    };
    binding.validate().map_err(|_| invalid())?;
    let m = &pod["metadata"];
    let labels = &m["labels"];
    if m["uid"] != binding.pod_uid
        || m["namespace"] != machine.sandbox_namespace
        || !m["deletionTimestamp"].is_null()
        || labels["nyxid.dev/machine-id"] != machine.node_id
        || labels["nyxid.dev/managed"] != "true"
        || labels["nyxid.dev/runtime-generation"].as_str()
            != Some(machine.runtime_generation.to_string().as_str())
        || labels["nyxid.dev/credential-epoch"].as_str()
            != Some(machine.credential_epoch.to_string().as_str())
        || labels["nyxid.dev/writer-fence"] != machine.writer_fence
    {
        return Err(invalid());
    }
    let owners = m["ownerReferences"].as_array().ok_or_else(invalid)?;
    let controllers: Vec<_> = owners.iter().filter(|v| v["controller"] == true).collect();
    if controllers.len() != 1
        || controllers[0]["kind"] != "Sandbox"
        || controllers[0]["apiVersion"] != "agents.x-k8s.io/v1beta1"
        || controllers[0]["uid"] != binding.sandbox_uid
    {
        return Err(invalid());
    }
    let pod_name = m["name"]
        .as_str()
        .filter(|s| name(s))
        .ok_or_else(invalid)?
        .to_owned();
    let node_name = pod["spec"]["nodeName"]
        .as_str()
        .filter(|s| name(s))
        .ok_or_else(invalid)?
        .to_owned();
    let specs = pod["spec"]["containers"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 16)
        .ok_or_else(invalid)?;
    let statuses = pod["status"]["containerStatuses"]
        .as_array()
        .filter(|v| v.len() <= 16)
        .ok_or_else(invalid)?;
    let mut containers = BTreeMap::new();
    for spec in specs {
        let n = spec["name"]
            .as_str()
            .filter(|s| name(s))
            .ok_or_else(invalid)?;
        let matches: Vec<_> = statuses.iter().filter(|s| s["name"] == n).collect();
        if matches.len() != 1 {
            return Err(invalid());
        }
        let status = matches[0];
        let id = status["containerID"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 512)
            .ok_or_else(invalid)?;
        let started = timestamp(&status["state"]["running"]["startedAt"]).ok_or_else(invalid)?;
        if containers
            .insert(n.to_owned(), (id.to_owned(), started))
            .is_some()
        {
            return Err(invalid());
        }
    }
    // Unexpected init/ephemeral containers can consume local storage; refuse a
    // complete projection of a shape which was not the admitted fixed profile.
    if pod["spec"]["initContainers"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
        || pod["spec"]["ephemeralContainers"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    {
        return Err(invalid());
    }
    let volumes = pod["spec"]["volumes"]
        .as_array()
        .filter(|v| v.len() <= 32)
        .ok_or_else(invalid)?;
    let mut disk_volumes = BTreeSet::new();
    let mut workspace = false;
    for volume in volumes {
        let n = volume["name"]
            .as_str()
            .filter(|s| name(s))
            .ok_or_else(invalid)?;
        if n == "workspace" {
            workspace = volume["persistentVolumeClaim"]["claimName"].as_str()
                == machine.pvc_name.as_deref()
                && machine.pvc_name.is_some();
            continue;
        }
        if let Some(empty) = volume["emptyDir"].as_object() {
            match empty.get("medium").and_then(Value::as_str).unwrap_or("") {
                "Memory" => {}
                "" => {
                    disk_volumes.insert(n.to_owned());
                }
                _ => return Err(invalid()),
            }
        } else if volume["projected"].is_null()
            && volume["secret"].is_null()
            && volume["configMap"].is_null()
        {
            return Err(invalid());
        }
    }
    if !workspace {
        return Err(invalid());
    }
    Ok(Target {
        binding,
        runtime_id,
        namespace: machine.sandbox_namespace.clone(),
        pod_name,
        node_name,
        containers,
        disk_volumes,
    })
}

fn metric(
    v: &Value,
    kind: String,
    now: DateTime<Utc>,
    components: &mut Vec<StorageComponent>,
    missing: &mut Vec<String>,
) -> Option<u64> {
    let Some(bytes) = v["usedBytes"].as_u64().filter(|v| *v <= i64::MAX as u64) else {
        missing.push(format!("{kind}:used_bytes_missing"));
        return None;
    };
    let Some(sampled) = timestamp(&v["time"]) else {
        missing.push(format!("{kind}:sample_time_missing"));
        return None;
    };
    if stale(sampled, now) {
        missing.push(format!("{kind}:stale"));
    }
    components.push(StorageComponent {
        kind,
        used_bytes: bytes,
        sampled_at: sampled,
    });
    Some(bytes)
}

fn parse_summary(
    target: &Target,
    summary: &Value,
    now: DateTime<Utc>,
) -> AppResult<PlatformStorageObservation> {
    if summary["node"]["nodeName"] != target.node_name {
        return Err(invalid());
    }
    let pods = summary["pods"]
        .as_array()
        .filter(|v| v.len() <= MAX_PODS)
        .ok_or_else(invalid)?;
    let matches: Vec<_> = pods
        .iter()
        .filter(|p| p["podRef"]["uid"] == target.binding.pod_uid)
        .collect();
    if matches.len() != 1 {
        return Err(invalid());
    }
    let pod = matches[0];
    if pod["podRef"]["namespace"] != target.namespace || pod["podRef"]["name"] != target.pod_name {
        return Err(invalid());
    }
    let mut components = vec![];
    let mut missing = vec![];
    let mut sum = Some(0u64);
    let containers = pod["containers"]
        .as_array()
        .filter(|v| v.len() <= 16)
        .ok_or_else(invalid)?;
    for (n, (_, started)) in &target.containers {
        let rows: Vec<_> = containers.iter().filter(|c| c["name"] == *n).collect();
        if rows.len() != 1 {
            missing.push("container_metrics_missing_or_ambiguous".into());
            sum = None;
            continue;
        }
        let c = rows[0];
        if timestamp(&c["startTime"]) != Some(*started) {
            return Err(invalid());
        }
        for k in ["rootfs", "logs"] {
            let value = metric(
                &c[k],
                format!("container_{k}"),
                now,
                &mut components,
                &mut missing,
            );
            if components.last().is_some_and(|m| m.sampled_at < *started) {
                return Err(invalid());
            }
            sum = sum.zip(value).and_then(|(a, b)| a.checked_add(b));
        }
    }
    if containers.len() != target.containers.len() {
        return Err(invalid());
    }
    let volumes = pod["volume"].as_array().filter(|v| v.len() <= 32);
    for name in &target.disk_volumes {
        let matches: Vec<_> = volumes
            .into_iter()
            .flatten()
            .filter(|v| v["name"] == *name)
            .collect();
        let value = if matches.len() == 1 {
            metric(
                matches[0],
                "disk_empty_dir".into(),
                now,
                &mut components,
                &mut missing,
            )
        } else {
            missing.push("disk_empty_dir_metrics_missing_or_ambiguous".into());
            None
        };
        sum = sum.zip(value).and_then(|(a, b)| a.checked_add(b));
    }
    let aggregate = metric(
        &pod["ephemeral-storage"],
        "pod_ephemeral_aggregate".into(),
        now,
        &mut components,
        &mut missing,
    );
    if aggregate.is_some()
        && target.containers.values().any(|(_, started)| {
            timestamp(&pod["ephemeral-storage"]["time"]).is_some_and(|sample| sample < *started)
        })
    {
        return Err(invalid());
    }
    let (source, used) = if let Some(v) = aggregate {
        ("kubelet_pod_ephemeral_aggregate", Some(v))
    } else {
        // Components are useful diagnostics but omit Pod-level overhead. Their
        // sum must never occupy the whole-Pod usage field.
        ("kubelet_components_partial", None)
    };
    // The aggregate already includes rootfs/logs/emptyDir/Pod-level overhead.
    // Never add components or workspace/PVC/tmpfs to it. No Pod available bytes.
    if aggregate.is_none() {
        missing.push("pod_level_logs_hosts_coverage_unknown".into());
    }
    if sum.is_none() {
        missing.push("component_sum_incomplete_or_overflow".into());
    }
    missing.sort();
    missing.dedup();
    Ok(PlatformStorageObservation {
        binding: target.binding.clone(),
        runtime_id: target.runtime_id.clone(),
        pod_name: target.pod_name.clone(),
        node_name: target.node_name.clone(),
        container_instances: target
            .containers
            .values()
            .map(|(id, _)| id.clone())
            .collect(),
        source: source.into(),
        used_bytes: used,
        complete: missing.is_empty(),
        components,
        missing_reasons: missing,
        received_at: now,
    })
}

fn configured_endpoints(raw: Option<&str>) -> AppResult<Option<BTreeMap<String, reqwest::Url>>> {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    if raw.len() > 64 * 1024 {
        return Err(unavailable());
    }
    let map: BTreeMap<String, String> = serde_json::from_str(raw).map_err(|_| unavailable())?;
    if map.is_empty() || map.len() > 256 {
        return Err(unavailable());
    }
    let mut endpoints = BTreeMap::new();
    for (node, url) in map {
        let url: reqwest::Url = url.parse().map_err(|_| unavailable())?;
        if !name(&node)
            || url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(unavailable());
        }
        endpoints.insert(node, url);
    }
    Ok(Some(endpoints))
}

pub struct KubeletCollector {
    client: reqwest::Client,
    endpoints: BTreeMap<String, reqwest::Url>,
    token_file: PathBuf,
    audience: String,
}
impl KubeletCollector {
    /// No endpoints means no optional collector, before CA/token/audience or
    /// Kubernetes state is accessed. Partial configured collectors still fail.
    pub fn from_environment() -> AppResult<Option<Self>> {
        let raw = std::env::var("NYXID_REMOTE_MACHINE_KUBELET_ENDPOINTS").ok();
        let Some(endpoints) = configured_endpoints(raw.as_deref())? else {
            return Ok(None);
        };
        let ca_file = std::fs::File::open(
            std::env::var("NYXID_REMOTE_MACHINE_KUBELET_CA_FILE").map_err(|_| unavailable())?,
        )
        .map_err(|_| unavailable())?;
        let metadata = ca_file.metadata().map_err(|_| unavailable())?;
        if !metadata.is_file() || metadata.len() > 256 * 1024 {
            return Err(unavailable());
        }
        let mut ca = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(ca_file, 256 * 1024 + 1), &mut ca)
            .map_err(|_| unavailable())?;
        if ca.len() > 256 * 1024 {
            return Err(unavailable());
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(4))
            .add_root_certificate(reqwest::Certificate::from_pem(&ca).map_err(|_| unavailable())?)
            .build()
            .map_err(|_| unavailable())?;
        let audience =
            std::env::var("NYXID_REMOTE_MACHINE_KUBELET_AUDIENCE").map_err(|_| unavailable())?;
        if audience.is_empty() || audience.len() > 256 {
            return Err(unavailable());
        }
        Ok(Some(Self {
            client,
            endpoints,
            token_file: PathBuf::from(
                std::env::var("NYXID_REMOTE_MACHINE_KUBELET_TOKEN_FILE")
                    .map_err(|_| unavailable())?,
            ),
            audience,
        }))
    }
    async fn summary(&self, node: &str) -> AppResult<Value> {
        let endpoint = self
            .endpoints
            .get(node)
            .ok_or_else(unavailable)?
            .join("/stats/summary")
            .map_err(|_| unavailable())?;
        let file = tokio::fs::File::open(&self.token_file)
            .await
            .map_err(|_| unavailable())?;
        let metadata = file.metadata().await.map_err(|_| unavailable())?;
        if !metadata.is_file() || metadata.len() > 64 * 1024 {
            return Err(unavailable());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o022 != 0 {
                return Err(unavailable());
            }
        }
        let mut token = Zeroizing::new(Vec::new());
        file.take(64 * 1024 + 1)
            .read_to_end(&mut token)
            .await
            .map_err(|_| unavailable())?;
        if token.len() > 64 * 1024 {
            return Err(unavailable());
        }
        let token = std::str::from_utf8(&token)
            .map_err(|_| unavailable())?
            .trim();
        // Actual authentication is by the kubelet. This local audience/expiry
        // check prevents accidentally forwarding an API-server-only credential.
        let payload = token
            .split('.')
            .nth(1)
            .and_then(|p| URL_SAFE_NO_PAD.decode(p).ok())
            .and_then(|p| serde_json::from_slice::<Value>(&p).ok())
            .ok_or_else(unavailable)?;
        if !(payload["aud"].as_str() == Some(&self.audience)
            || payload["aud"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == &self.audience)))
            || payload["exp"]
                .as_i64()
                .is_none_or(|e| e <= Utc::now().timestamp())
        {
            return Err(unavailable());
        }
        let mut response = self
            .client
            .get(endpoint)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(c) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + c.len() > MAX_SUMMARY {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&c);
        }
        serde_json::from_slice(&bytes).map_err(|_| unavailable())
    }
}

pub fn observation_stale(value: &PlatformStorageObservation, now: DateTime<Utc>) -> bool {
    stale(value.received_at, now)
        || value.components.is_empty()
        || value.components.iter().any(|c| stale(c.sampled_at, now))
}

/// Uses the same ManagedMachine row for the full runtime CAS; a replacement
/// invalidates this sample and cannot receive a prior Pod's storage observation.
async fn record(
    db: &Database,
    m: &ManagedMachine,
    o: &PlatformStorageObservation,
) -> AppResult<()> {
    let b = &o.binding;
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let result:AppResult<()>=async {
        let n=db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc!{"_id":&b.node_id,"is_active":true,"machine.runtime_id":&o.runtime_id},doc!{"$set":{"platform_storage_seen_at":bson::DateTime::from_chrono(o.received_at)}}).session(&mut session).await?;
        if n.matched_count!=1{return Err(invalid())}
        let changed=db.collection::<Document>(MACHINES).update_one(doc!{"_id":&m.node_id,"owner_id":&m.owner_id,"sandbox_uid":&b.sandbox_uid,"pod_uid":&b.pod_uid,"pvc_uid":&m.pvc_uid,"runtime_generation":b.runtime_generation as i64,"credential_epoch":b.credential_epoch as i64,"writer_fence":&b.writer_fence,"resource_state":{"$nin":["deleting","deleted"]},"$or":[{"platform_storage.received_at":{"$lt":bson::DateTime::from_chrono(o.received_at)}},{"platform_storage":null}]},doc!{"$set":{"platform_storage":bson::to_bson(o).map_err(|_|invalid())?}}).session(&mut session).await?;
        if changed.matched_count!=1{return Err(invalid())}Ok(())
    }.await;
    match result {
        Ok(()) => {
            session.commit_transaction().await?;
            Ok(())
        }
        Err(e) => {
            let _ = session.abort_transaction().await;
            Err(e)
        }
    }
}

pub async fn refresh_owner(db: &Database, owner: &str, node: &str) -> AppResult<()> {
    let Some(collector) = KubeletCollector::from_environment()? else {
        return Ok(());
    };
    let _permit = COLLECTION_SLOTS.try_acquire().map_err(|_| unavailable())?;
    let verifier = KubernetesWorkloadVerifier::from_environment()?;
    let machine = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(
            doc! {"_id":node,"owner_id":owner,"resource_state":{"$nin":["deleting","deleted"]}},
        )
        .await?
        .ok_or_else(invalid)?;
    let row = db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id":node,"is_active":true})
        .projection(doc! {"machine.runtime_id":1})
        .await?
        .ok_or_else(invalid)?;
    let runtime = row
        .get_document("machine")
        .ok()
        .and_then(|m| m.get_str("runtime_id").ok())
        .ok_or_else(invalid)?
        .to_owned();
    let selector = url::form_urlencoded::byte_serialize(
        format!("nyxid.dev/machine-id={node},nyxid.dev/managed=true").as_bytes(),
    )
    .collect::<String>();
    let list = verifier
        .read_json(&format!(
            "/api/v1/namespaces/nyxid-machine/pods?limit=100&labelSelector={selector}"
        ))
        .await?;
    if list["metadata"]["continue"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
    {
        return Err(invalid());
    }
    let rows = list["items"]
        .as_array()
        .filter(|a| a.len() <= 100)
        .ok_or_else(invalid)?;
    let matches: Vec<_> = rows
        .iter()
        .filter(|p| p["metadata"]["uid"].as_str() == machine.pod_uid.as_deref())
        .collect();
    if matches.len() != 1 {
        return Err(invalid());
    }
    let target = target(&machine, runtime, matches[0])?;
    let summary = collector.summary(&target.node_name).await?;
    let observation = parse_summary(&target, &summary, Utc::now())?;
    // Re-read the exact live Pod after collection. Mongo runtime CAS alone
    // cannot observe a container restart before its next Node WSS handshake.
    let pod = verifier
        .read_json(&format!(
            "/api/v1/namespaces/{}/pods/{}",
            target.namespace, target.pod_name
        ))
        .await?;
    let after = self::target(&machine, target.runtime_id.clone(), &pod)?;
    if after.containers != target.containers
        || after.node_name != target.node_name
        || after.disk_volumes != target.disk_volumes
    {
        return Err(invalid());
    }
    record(db, &machine, &observation).await
}

pub async fn owner_observation(
    db: &Database,
    owner: &str,
    node: &str,
) -> AppResult<Option<PlatformStorageObservation>> {
    let row=db.collection::<Document>(MACHINES).find_one(doc!{"_id":node,"owner_id":owner,"resource_state":{"$nin":["deleting","deleted"]}}).projection(doc!{"platform_storage":1,"runtime_generation":1,"credential_epoch":1,"pod_uid":1,"writer_fence":1}).await?.ok_or_else(invalid)?;
    let Some(value) = row.get("platform_storage") else {
        return Ok(None);
    };
    let mut o: PlatformStorageObservation =
        bson::from_bson(value.clone()).map_err(|_| invalid())?;
    // Legacy rows stored the diagnostic component sum in used_bytes. Keep the
    // original sample/provenance but never project that sum as whole-Pod use.
    if o.source == "kubelet_components_partial" {
        o.used_bytes = None;
    }
    if row.get_i64("runtime_generation").ok() != Some(o.binding.runtime_generation as i64)
        || row.get_i64("credential_epoch").ok() != Some(o.binding.credential_epoch as i64)
        || row.get_str("pod_uid").ok() != Some(o.binding.pod_uid.as_str())
        || row.get_str("writer_fence").ok() != Some(o.binding.writer_fence.as_str())
    {
        return Ok(None);
    }
    let current = db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id":node,"machine.runtime_id":&o.runtime_id})
        .projection(doc! {"_id":1})
        .await?;
    if current.is_none() {
        return Ok(None);
    }
    Ok(Some(o))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn optional_collector_absence_does_not_require_credentials_or_fake_endpoints() {
        assert!(configured_endpoints(None).unwrap().is_none());
        assert!(configured_endpoints(Some("  ")).unwrap().is_none());
        let endpoints =
            configured_endpoints(Some(r#"{"node-one":"https://kubelet.example:10250/"}"#))
                .unwrap()
                .unwrap();
        assert_eq!(endpoints.len(), 1);
        assert!(configured_endpoints(Some("{}")).is_err());
        assert!(
            configured_endpoints(Some(r#"{"node-one":"http://kubelet.example:10250/"}"#)).is_err()
        );
    }

    #[test]
    fn target_requires_canonical_string_generation_and_epoch_labels() {
        use crate::services::remote_machine_service::CreateMachineInput;
        use uuid::Uuid;

        let now = Utc::now();
        let mut machine = CreateMachineInput::fixed("owner", true, "sha256:image")
            .preview(&Uuid::new_v4().to_string(), now);
        machine.sandbox_uid = Some(Uuid::new_v4().to_string());
        machine.pod_uid = Some(Uuid::new_v4().to_string());
        machine.pvc_name = Some("workspace".into());
        machine.runtime_generation = 3;
        machine.credential_epoch = 8;
        machine.active_operation_id = Some(Uuid::new_v4().to_string());
        machine.bootstrap_delivery_id = Some(Uuid::new_v4().to_string());
        let pod = json!({
            "metadata": {
                "uid": machine.pod_uid,
                "namespace": machine.sandbox_namespace,
                "name": "machine-runtime",
                "labels": {
                    "nyxid.dev/machine-id": machine.node_id,
                    "nyxid.dev/managed": "true",
                    "nyxid.dev/runtime-generation": "3",
                    "nyxid.dev/credential-epoch": "8",
                    "nyxid.dev/writer-fence": machine.writer_fence
                },
                "ownerReferences": [{
                    "controller": true,
                    "kind": "Sandbox",
                    "apiVersion": "agents.x-k8s.io/v1beta1",
                    "uid": machine.sandbox_uid
                }]
            },
            "spec": {
                "nodeName": "worker-one",
                "containers": [{"name": "node"}],
                "volumes": [{
                    "name": "workspace",
                    "persistentVolumeClaim": {"claimName": machine.pvc_name}
                }]
            },
            "status": {"containerStatuses": [{
                "name": "node",
                "containerID": "containerd://current",
                "state": {"running": {"startedAt": now.to_rfc3339()}}
            }]}
        });
        assert!(target(&machine, "runtime".into(), &pod).is_ok());
        for (label, value) in [
            ("nyxid.dev/runtime-generation", machine.runtime_generation),
            ("nyxid.dev/credential-epoch", machine.credential_epoch),
        ] {
            for invalid in [
                json!(value),
                json!(format!("0{value}")),
                json!(format!("+{value}")),
                json!(format!(" {value}")),
                json!((value + 1).to_string()),
                Value::Bool(false),
                Value::Null,
            ] {
                let mut changed = pod.clone();
                changed["metadata"]["labels"][label] = invalid;
                assert!(matches!(
                    target(&machine, "runtime".into(), &changed),
                    Err(AppError::MachineAuthorityStale)
                ));
            }
            let mut missing = pod.clone();
            missing["metadata"]["labels"]
                .as_object_mut()
                .unwrap()
                .remove(label);
            assert!(matches!(
                target(&machine, "runtime".into(), &missing),
                Err(AppError::MachineAuthorityStale)
            ));
        }
    }

    #[test]
    fn aggregate_from_prior_container_attempt_is_not_current_coverage() {
        let (t, mut summary, now) = fixture();
        let started = t.containers.values().next().unwrap().1;
        summary["pods"][0]["ephemeral-storage"]["time"] =
            json!((started - chrono::Duration::milliseconds(1)).to_rfc3339());
        assert!(parse_summary(&t, &summary, now).is_err());
    }

    #[tokio::test]
    async fn authenticated_collector_rotates_credentials_and_bounds_failures() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        };
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let token_file = dir.path().join("token");
        let token = |id: &str, aud: &str| {
            format!(
                "e30.{}.local-test-signature",
                URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(
                        &json!({"aud":[aud],"exp":Utc::now().timestamp()+120,"jti":id})
                    )
                    .unwrap()
                )
            )
        };
        let collector = KubeletCollector {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_millis(200))
                .build()
                .unwrap(),
            endpoints: BTreeMap::from([("node-name".into(), server.uri().parse().unwrap())]),
            token_file: token_file.clone(),
            audience: "kubelet-stats".into(),
        };
        for id in ["first", "rotated"] {
            let credential = token(id, "kubelet-stats");
            std::fs::write(&token_file, &credential).unwrap();
            server.reset().await;
            Mock::given(method("GET"))
                .and(path("/stats/summary"))
                .and(header("authorization", format!("Bearer {credential}")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"node":{"nodeName":"node-name"},"pods":[]})),
                )
                .expect(1)
                .mount(&server)
                .await;
            assert!(collector.summary("node-name").await.is_ok());
            server.verify().await;
        }
        std::fs::write(&token_file, token("wrong", "apiserver-only")).unwrap();
        server.reset().await;
        assert!(collector.summary("node-name").await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
        std::fs::write(&token_file, token("test", "kubelet-stats")).unwrap();
        for response in [
            ResponseTemplate::new(401),
            ResponseTemplate::new(403),
            ResponseTemplate::new(200).set_body_string("x".repeat(MAX_SUMMARY + 1)),
            ResponseTemplate::new(200).set_delay(Duration::from_secs(1)),
        ] {
            server.reset().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            assert!(collector.summary("node-name").await.is_err());
        }
        assert!(collector.summary("unselected-node").await.is_err());
    }

    #[tokio::test]
    async fn platform_sample_cas_and_node_report_freshness_are_independent() {
        let (db, _, binding, _) = super::super::remote_machine_bootstrap::tests::fixture().await;
        let machine = db
            .collection::<ManagedMachine>(MACHINES)
            .find_one(doc! {"_id":&binding.node_id})
            .await
            .unwrap()
            .unwrap();
        let runtime = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&binding.node_id},
                doc! {"$set":{"is_active":true,"machine.runtime_id":&runtime}},
            )
            .await
            .unwrap();
        let (t, v, now) = fixture();
        let mut observation = parse_summary(&t, &v, now).unwrap();
        observation.binding = binding;
        observation.runtime_id = runtime.clone();
        record(&db, &machine, &observation).await.unwrap();
        assert!(
            owner_observation(&db, &machine.owner_id, &machine.node_id)
                .await
                .unwrap()
                .is_some()
        );
        observation.received_at += chrono::Duration::milliseconds(1);
        observation.binding.runtime_generation += 1;
        assert!(record(&db, &machine, &observation).await.is_err());
        db.collection::<Document>(MACHINES).update_one(doc!{"_id":&machine.node_id},doc!{"$set":{"usage_received_at":bson::DateTime::now(),"platform_storage.received_at":bson::DateTime::from_chrono(now-chrono::Duration::seconds(91))}}).await.unwrap();
        let old = owner_observation(&db, &machine.owner_id, &machine.node_id)
            .await
            .unwrap()
            .unwrap();
        assert!(observation_stale(&old, now));
        db.collection::<Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&machine.node_id},
                doc! {"$set":{"machine.runtime_id":uuid::Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
        assert!(
            owner_observation(&db, &machine.owner_id, &machine.node_id)
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }
    fn fixture() -> (Target, Value, DateTime<Utc>) {
        let now = Utc::now();
        let start = now - chrono::Duration::seconds(60);
        let t = Target {
            binding: Binding {
                node_id: uuid::Uuid::new_v4().to_string(),
                sandbox_uid: "sandbox".into(),
                pod_uid: "pod".into(),
                runtime_generation: 1,
                credential_epoch: 1,
                writer_fence: "writer".into(),
                operation_id: uuid::Uuid::new_v4().to_string(),
                delivery_id: uuid::Uuid::new_v4().to_string(),
            },
            runtime_id: "runtime".into(),
            namespace: "nyxid-machine".into(),
            pod_name: "pod-name".into(),
            node_name: "node-name".into(),
            containers: BTreeMap::from([("node".into(), ("containerd://instance".into(), start))]),
            disk_volumes: BTreeSet::from(["tmp".into(), "home".into()]),
        };
        let metric = |n| json!({"usedBytes":n,"time":now.to_rfc3339()});
        let v = json!({"node":{"nodeName":"node-name"},"pods":[{"podRef":{"uid":"pod","namespace":"nyxid-machine","name":"pod-name"},"containers":[{"name":"node","startTime":start.to_rfc3339(),"rootfs":metric(10),"logs":metric(20)}],"volume":[{"name":"tmp","usedBytes":30,"time":now.to_rfc3339()},{"name":"home","usedBytes":40,"time":now.to_rfc3339()},{"name":"workspace","usedBytes":999999,"time":now.to_rfc3339()},{"name":"identity","usedBytes":999999,"time":now.to_rfc3339()}],"ephemeral-storage":metric(110)}]});
        (t, v, now)
    }
    #[test]
    fn aggregate_not_added_to_components_and_no_pvc_tmpfs() {
        let (t, v, n) = fixture();
        let o = parse_summary(&t, &v, n).unwrap();
        assert_eq!(o.used_bytes, Some(110));
        assert!(o.complete);
        assert_eq!(o.components.len(), 5);
        assert!(!observation_stale(&o, n));
    }
    #[test]
    fn fresh_aggregate_cannot_hide_stale_component() {
        let (t, mut v, n) = fixture();
        v["pods"][0]["containers"][0]["logs"]["time"] =
            json!((n - chrono::Duration::seconds(91)).to_rfc3339());
        let mut t = t;
        t.containers.get_mut("node").unwrap().1 = n - chrono::Duration::seconds(120);
        v["pods"][0]["containers"][0]["startTime"] = json!(t.containers["node"].1.to_rfc3339());
        let o = parse_summary(&t, &v, n).unwrap();
        assert!(!o.complete);
        assert!(observation_stale(&o, n));
    }
    #[test]
    fn missing_aggregate_retains_components_but_whole_pod_usage_is_unknown() {
        let (t, mut v, n) = fixture();
        v["pods"][0]["ephemeral-storage"] = Value::Null;
        let o = parse_summary(&t, &v, n).unwrap();
        assert_eq!(o.used_bytes, None);
        assert_eq!(o.components.len(), 4);
        assert_eq!(o.components.iter().map(|c| c.used_bytes).sum::<u64>(), 100);
        assert!(!o.complete);
    }
    #[test]
    fn replaced_pod_host_and_container_start_rejected() {
        let (t, mut v, n) = fixture();
        v["pods"][0]["podRef"]["uid"] = json!("old-pod");
        assert!(parse_summary(&t, &v, n).is_err());
        let (t, mut v, n) = fixture();
        v["node"]["nodeName"] = json!("other");
        assert!(parse_summary(&t, &v, n).is_err());
        let (t, mut v, n) = fixture();
        v["pods"][0]["containers"][0]["startTime"] = json!(n.to_rfc3339());
        assert!(parse_summary(&t, &v, n).is_err());
    }
    #[test]
    fn missing_logs_and_overflow_remain_unknown() {
        let (t, mut v, n) = fixture();
        v["pods"][0]["containers"][0]["logs"]["usedBytes"] = json!(u64::MAX);
        let o = parse_summary(&t, &v, n).unwrap();
        assert!(!o.complete);
        assert_eq!(o.used_bytes, Some(110));
    }
}

#[cfg(test)]
#[path = "remote_machine_storage_telemetry_tls_tests.rs"]
mod tls_tests;
