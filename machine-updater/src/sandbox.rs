//! Minimal Agent Sandbox API adapter used by the Remote Nyx Machine manager.
//!
//! The adapter is deliberately small: it submits and observes the
//! `agents.x-k8s.io/v1beta1/Sandbox` custom resource and records the
//! controller's observations. It never creates Pods/PVCs, calls Kubernetes
//! exec, or treats an API response as proof of physical process termination.

use crate::kubernetes_credentials::KubernetesCredentials;
use crate::sandbox_observation::{
    ObservationErrorKind, ObservationResourceKind, ObservationStage, ObservationVerb,
    SandboxObservationError,
};
use anyhow::{Context, Result, anyhow, ensure};
use reqwest::{Client, Method, StatusCode, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fmt, path::PathBuf};

pub const API_GROUP: &str = "agents.x-k8s.io";
pub const API_VERSION: &str = "v1beta1";
pub const KIND: &str = "Sandbox";

fn dns_subdomain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

pub fn validate_storage_class_name(value: &str) -> Result<()> {
    ensure!(
        dns_subdomain(value),
        "StorageClass name must be a DNS subdomain"
    );
    Ok(())
}

fn qualified_provisioner(value: &str) -> bool {
    let name = match value.split_once('/') {
        Some((prefix, name)) if dns_subdomain(prefix) => name,
        Some(_) => return false,
        None => value,
    };
    !name.is_empty()
        && name.len() <= 63
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name.as_bytes()[name.len() - 1].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_storage_class(object: &Value, expected: &str) -> Result<String> {
    validate_storage_class_name(expected)?;
    ensure!(
        object
            .get("kind")
            .and_then(Value::as_str)
            .is_none_or(|kind| kind == "StorageClass")
            && object
                .get("apiVersion")
                .and_then(Value::as_str)
                .is_none_or(|version| version == "storage.k8s.io/v1")
            && object.pointer("/metadata/name").and_then(Value::as_str) == Some(expected)
            && object
                .pointer("/metadata/uid")
                .and_then(Value::as_str)
                .is_some_and(|uid| !uid.is_empty())
            && object
                .pointer("/metadata/deletionTimestamp")
                .is_none_or(Value::is_null)
            && object
                .get("provisioner")
                .and_then(Value::as_str)
                .is_some_and(qualified_provisioner),
        "StorageClass is invalid or deleting"
    );
    Ok(expected.to_owned())
}

/// Stable DNS-compatible Sandbox name derived from a UUID machine identity.
/// The manager persists the resulting name in its operation intent and never
/// generates a random replacement after a lost response.
pub fn deterministic_name(machine_id: &str) -> Result<String> {
    let uuid = uuid::Uuid::parse_str(machine_id).context("machine id must be a UUID")?;
    Ok(format!("machine-{}", uuid.simple()))
}

/// A bounded, manager-controlled desired state. Callers provide a complete
/// pod template from the approved profile; users never pass through a raw
/// Kubernetes object.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxIntent {
    pub namespace: String,
    pub name: String,
    pub owner_id: String,
    pub machine_id: String,
    pub operation_id: String,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: String,
    pub profile: String,
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_storage_class: Option<String>,
    pub workspace_size: String,
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
    pub running: bool,
    pub shutdown_time: Option<String>,
    pub bootstrap_base_url: String,
}

impl SandboxIntent {
    pub fn validate(&self) -> Result<()> {
        let endpoint =
            Url::parse(&self.bootstrap_base_url).context("managed bootstrap base URL required")?;
        ensure!(
            endpoint.scheme() == "https"
                && endpoint.host_str().is_some()
                && endpoint.username().is_empty()
                && endpoint.password().is_none()
                && endpoint.query().is_none()
                && endpoint.fragment().is_none(),
            "managed bootstrap requires HTTPS"
        );
        ensure!(
            self.runtime_generation > 0 && self.credential_epoch > 0,
            "sandbox runtime generation and credential epoch must be positive"
        );
        ensure!(
            uuid::Uuid::parse_str(&self.writer_fence).is_ok(),
            "sandbox writer fence must be a UUID"
        );
        for (label, value) in [
            ("namespace", self.namespace.as_str()),
            ("name", self.name.as_str()),
            ("owner_id", self.owner_id.as_str()),
            ("machine_id", self.machine_id.as_str()),
            ("operation_id", self.operation_id.as_str()),
            ("profile", self.profile.as_str()),
            ("image", self.image.as_str()),
        ] {
            ensure!(
                !value.trim().is_empty(),
                "sandbox {label} must not be empty"
            );
            ensure!(value.len() <= 253, "sandbox {label} exceeds 253 bytes");
            ensure!(
                !value.chars().any(|c| c.is_ascii_control() || c == '\\'),
                "sandbox {label} contains an unsafe character"
            );
        }
        if let Some(storage_class) = &self.workspace_storage_class {
            validate_storage_class_name(storage_class)?;
        }
        if let Some(runtime_class) = &self.runtime_class {
            ensure!(
                !runtime_class.is_empty()
                    && runtime_class.len() <= 253
                    && runtime_class.split('.').all(|label| {
                        !label.is_empty()
                            && label.len() <= 63
                            && label.as_bytes()[0].is_ascii_alphanumeric()
                            && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                            && label.bytes().all(|byte| {
                                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                            })
                    }),
                "sandbox runtime_class must be an existing DNS name when configured"
            );
        }
        ensure!(
            self.image.contains("@sha256:"),
            "sandbox image must be immutable and digest pinned"
        );
        ensure!(
            self.image
                .rsplit_once("@sha256:")
                .is_some_and(|(_, digest)| digest.len() == 64
                    && digest.bytes().all(|byte| byte.is_ascii_hexdigit())),
            "sandbox image digest is invalid"
        );
        ensure!(
            self.profile == nyxid_machine::managed::PROFILE
                && matches!(self.cpu_request.as_str(), "2" | "2000m")
                && matches!(self.cpu_limit.as_str(), "2" | "2000m")
                && matches!(self.memory_request.as_str(), "2Gi" | "2048Mi")
                && matches!(self.memory_limit.as_str(), "2Gi" | "2048Mi")
                && self.workspace_size == "20Gi",
            "sandbox resources must match the fixed Machine profile"
        );
        Ok(())
    }

    fn labels(&self) -> serde_json::Map<String, Value> {
        let mut labels = serde_json::Map::new();
        labels.insert("nyxid.dev/managed".into(), json!("true"));
        labels.insert("nyxid.dev/machine-id".into(), json!(self.machine_id));
        labels.insert("nyxid.dev/owner-id".into(), json!(self.owner_id));
        labels.insert("nyxid.dev/operation-id".into(), json!(self.operation_id));
        labels.insert(
            "nyxid.dev/runtime-generation".into(),
            json!(self.runtime_generation.to_string()),
        );
        labels.insert(
            "nyxid.dev/credential-epoch".into(),
            json!(self.credential_epoch.to_string()),
        );
        labels.insert("nyxid.dev/writer-fence".into(), json!(self.writer_fence));
        labels.insert("nyxid.dev/profile".into(), json!(self.profile));
        labels
    }

    fn workspace_labels(&self) -> serde_json::Map<String, Value> {
        self.labels()
            .into_iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "nyxid.dev/managed"
                        | "nyxid.dev/machine-id"
                        | "nyxid.dev/owner-id"
                        | "nyxid.dev/profile"
                )
            })
            .collect()
    }

    /// Build the exact CR body sent with server-side apply. The pod template
    /// is fixed by the profile; only the machine identity and operation labels
    /// vary per request.
    pub fn apply_object(&self) -> Value {
        let mode = if self.running { "Running" } else { "Suspended" };
        let mut spec = json!({
            "service": false,
            "operatingMode": mode,
            "shutdownPolicy": "Retain",
            "podTemplate": {
                "metadata": {"labels": self.labels()},
                "spec": {
                    "automountServiceAccountToken": false,
                    "serviceAccountName": "nyxid-machine-workload",
                    "containers": [{
                        "name": "node",
                        "image": self.image,
                        "env": [
                            {"name":"NYXID_MANAGED_KUBERNETES","value":"1"},
                            {"name":"NYXID_MANAGED_NODE_ID","value":self.machine_id},
                            {"name":"NYXID_MANAGED_POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}},
                            {"name":"NYXID_MANAGED_WORKLOAD_TOKEN_FILE","value":"/var/run/nyxid-workload/token"},
                            {"name":"NYXID_MANAGED_PREPARE_URL","value":format!("{}/api/v1/machines/managed/prepare",self.bootstrap_base_url.trim_end_matches('/'))},
                            {"name":"NYXID_MANAGED_BOOTSTRAP_URL","value":format!("{}/api/v1/machines/managed/bootstrap",self.bootstrap_base_url.trim_end_matches('/'))}
                        ],
                        "resources": {
                            "requests": {
                                "cpu": (nyxid_machine::managed::PROFILE_CPU_MILLIS / 1000).to_string(),
                                "memory": format!("{}Gi", nyxid_machine::managed::PROFILE_MEMORY_MIB / 1024)
                            },
                            "limits": {
                                "cpu": (nyxid_machine::managed::PROFILE_CPU_MILLIS / 1000).to_string(),
                                "memory": format!("{}Gi", nyxid_machine::managed::PROFILE_MEMORY_MIB / 1024)
                            }
                        },
                        "securityContext": {
                            "allowPrivilegeEscalation": false,
                            "readOnlyRootFilesystem": true,
                            "runAsNonRoot": false,
                            "runAsUser": 0,
                            "runAsGroup": 0,
                            "seccompProfile": {"type": "RuntimeDefault"},
                            // Required only by the supervisor to initialize
                            // the workspace and drop every command to agent.
                            // setuid clears these from command descendants.
                            "capabilities": {"drop": ["ALL"], "add": ["CHOWN", "SETUID", "SETGID", "KILL"]}
                        },
                        "volumeMounts": [
                            {"name": "workspace", "mountPath": "/workspace"},
                            {"name": "identity", "mountPath": "/var/lib/nyxid-machine"},
                            {"name": "workload-identity", "mountPath": "/var/run/nyxid-workload", "readOnly": true},
                            {"name": "tmp", "mountPath": "/tmp"},
                            {"name": "home", "mountPath": "/home"}
                        ],
                        "startupProbe": {
                            "exec": {"command": ["/bin/sh", "-ec", "test -s /var/lib/nyxid-machine/node/config.toml && test -s /var/lib/nyxid-machine/node/node.pid && kill -0 \"$(cat /var/lib/nyxid-machine/node/node.pid)\""]},
                            "periodSeconds": 5, "failureThreshold": 72, "timeoutSeconds": 2
                        },
                        "livenessProbe": {
                            "exec": {"command": ["/bin/sh", "-ec", "test -s /var/lib/nyxid-machine/node/config.toml && test -s /var/lib/nyxid-machine/node/node.pid && kill -0 \"$(cat /var/lib/nyxid-machine/node/node.pid)\""]},
                            "periodSeconds": 10, "failureThreshold": 3, "timeoutSeconds": 2
                        }
                    }],
                    "volumes": [
                        {"name": "identity", "emptyDir": {"medium": "Memory"}},
                        {"name":"workload-identity","projected":{"defaultMode":256,"sources":[
                            {"serviceAccountToken":{"audience":"nyxid-remote-machine-v1","expirationSeconds":600,"path":"token"}}
                        ]}},
                        {"name": "tmp", "emptyDir": {}},
                        {"name": "home", "emptyDir": {}}
                    ]
                }
            },
            "volumeClaimTemplates": [{
                "metadata": {"name": "workspace", "labels": self.workspace_labels()},
                "spec": {
                    "accessModes": ["ReadWriteOnce"],
                    "resources": {"requests": {"storage": self.workspace_size}}
                }
            }]
        });
        if let Some(runtime_class) = &self.runtime_class {
            spec["podTemplate"]["spec"]["runtimeClassName"] = json!(runtime_class);
        }
        if let Some(storage_class) = &self.workspace_storage_class {
            spec["volumeClaimTemplates"][0]["spec"]["storageClassName"] = json!(storage_class);
        }
        // Expiry takes a separate upstream controller branch and does not
        // refresh Suspended. A suspended intent removes our SSA-owned deadline
        // so normal suspension reconciliation can observe Pod termination.
        if self.running
            && let Some(shutdown_time) = &self.shutdown_time
        {
            spec["shutdownTime"] = json!(shutdown_time);
        }
        json!({
            "apiVersion": format!("{API_GROUP}/{API_VERSION}"),
            "kind": KIND,
            "metadata": {
                "name": self.name,
                "namespace": self.namespace,
                "labels": self.labels()
            },
            "spec": spec
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct SandboxObservation {
    pub shutdown_time: Option<String>,
    pub name: String,
    pub namespace: String,
    pub sandbox_uid: Option<String>,
    pub runtime_generation: Option<String>,
    pub credential_epoch: Option<String>,
    pub writer_fence: Option<String>,
    pub machine_id: Option<String>,
    pub owner_id: Option<String>,
    pub operation_id: Option<String>,
    pub profile: Option<String>,
    pub ready: bool,
    /// Requested state is separate from an applied controller condition.
    pub desired_suspended: bool,
    pub suspended: bool,
    pub generation: Option<i64>,
    pub deletion_timestamp: Option<String>,
    pub node_name: Option<String>,
    pub tracked_pod_name: Option<String>,
    pub expired: bool,
    pub observed_generation: Option<i64>,
    pub resource_version: Option<String>,
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Condition {
    #[serde(rename = "type")]
    pub condition_type: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    #[serde(rename = "observedGeneration")]
    pub observed_generation: Option<i64>,
    #[serde(rename = "lastTransitionTime")]
    pub last_transition_time: Option<String>,
}

/// Read-only control-plane observations. These are never physical stop or
/// storage isolation proofs. Unknown fields stay unknown, never successful.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct OwnedResourceObservation {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub creation_timestamp: Option<String>,
    pub deletion_timestamp: Option<String>,
    pub deletion_grace_period_seconds: Option<i64>,
    pub finalizers: Vec<String>,
    pub owner_uids: Vec<String>,
    pub controller_owner_uid: Option<String>,
    pub ownership_valid: bool,
    pub phase: Option<String>,
    pub node_name: Option<String>,
    pub machine_id: Option<String>,
    pub owner_id: Option<String>,
    pub operation_id: Option<String>,
    pub runtime_generation: Option<String>,
    pub credential_epoch: Option<String>,
    pub writer_fence: Option<String>,
    pub workspace_claim_name: Option<String>,
    pub volume_name: Option<String>,
    pub all_containers_terminated: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct NodeObservation {
    pub name: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub ready: bool,
    pub ready_last_heartbeat_time: Option<String>,
    pub container_runtime_version: Option<String>,
    pub deletion_timestamp: Option<String>,
}

/// A Node Lease is an identity/freshness observation, never runtime stop proof.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct NodeLeaseObservation {
    pub name: String,
    pub namespace: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub owner_node_uid: Option<String>,
    pub holder_identity: Option<String>,
    pub renew_time: Option<String>,
    pub lease_duration_seconds: Option<i64>,
    pub deletion_timestamp: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct VolumeAttachmentObservation {
    pub name: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub persistent_volume_name: Option<String>,
    pub node_name: Option<String>,
    pub attacher: Option<String>,
    /// Missing status is unknown, rather than detached.
    pub attached: Option<bool>,
    pub deletion_timestamp: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct PersistentVolumeObservation {
    pub name: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub claim_uid: Option<String>,
    pub claim_namespace: Option<String>,
    pub claim_name: Option<String>,
    pub reclaim_policy: Option<String>,
    pub phase: Option<String>,
    pub deletion_timestamp: Option<String>,
    pub finalizers: Vec<String>,
    #[serde(default)]
    pub csi_driver: Option<String>,
    #[serde(default)]
    pub csi_volume_handle: Option<String>,
}

/// Metadata-only inventory, independent of a surviving Sandbox. Labels are
/// correlation hints, never ownership or cleanup authority. Invalid values are
/// omitted from diagnostics; their identity hash still permits investigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlledResourceObservation {
    pub kind: &'static str,
    pub identity_hash: String,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub uid: Option<String>,
    pub sandbox_owner_uid: Option<String>,
    pub machine_id: Option<String>,
    pub owner_id: Option<String>,
    pub operation_id: Option<String>,
    pub runtime_generation: Option<i64>,
    pub credential_epoch: Option<i64>,
    pub profile: Option<String>,
    pub metadata_valid: bool,
}

impl ControlledResourceObservation {
    fn from_object(object: &Value, kind: &'static str, namespace: &str) -> Self {
        let text = |path| object.pointer(path).and_then(Value::as_str);
        let uuid = |path| {
            text(path)
                .filter(|v| v.len() == 36 && uuid::Uuid::parse_str(v).is_ok())
                .map(str::to_owned)
        };
        let dns = |path| {
            text(path)
                .filter(|v| {
                    !v.is_empty()
                        && v.len() <= 253
                        && v.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.')
                        })
                })
                .map(str::to_owned)
        };
        let identity_hash = hex::encode(Sha256::digest(
            json!([
                kind,
                object.pointer("/metadata/name"),
                object.pointer("/metadata/uid")
            ])
            .to_string()
            .as_bytes(),
        ));
        let owners = object
            .pointer("/metadata/ownerReferences")
            .and_then(Value::as_array);
        let owner = owners
            .filter(|owners| owners.len() == 1)
            .and_then(|owners| owners.first());
        let sandbox_owner_uid = owner
            .filter(|o| {
                o.get("kind").and_then(Value::as_str) == Some(KIND)
                    && o.get("apiVersion").and_then(Value::as_str)
                        == Some("agents.x-k8s.io/v1beta1")
                    && o.get("controller").and_then(Value::as_bool) == Some(true)
            })
            .and_then(|o| o.get("uid").and_then(Value::as_str))
            .filter(|v| v.len() == 36 && uuid::Uuid::parse_str(v).is_ok())
            .map(str::to_owned);
        let positive = |path| {
            text(path)
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|n| *n > 0)
        };
        let mut observed = Self {
            kind,
            identity_hash,
            namespace: dns("/metadata/namespace"),
            name: dns("/metadata/name"),
            uid: uuid("/metadata/uid"),
            sandbox_owner_uid,
            machine_id: uuid("/metadata/labels/nyxid.dev~1machine-id"),
            owner_id: uuid("/metadata/labels/nyxid.dev~1owner-id"),
            operation_id: uuid("/metadata/labels/nyxid.dev~1operation-id"),
            runtime_generation: positive("/metadata/labels/nyxid.dev~1runtime-generation"),
            credential_epoch: positive("/metadata/labels/nyxid.dev~1credential-epoch"),
            profile: text("/metadata/labels/nyxid.dev~1profile")
                .filter(|v| nyxid_machine::managed::is_supported_profile(v))
                .map(str::to_owned),
            metadata_valid: false,
        };
        observed.metadata_valid = observed.namespace.as_deref() == Some(namespace)
            && observed.name.is_some()
            && observed.uid.is_some()
            && observed.sandbox_owner_uid.is_some()
            && observed.machine_id.is_some()
            && observed.owner_id.is_some()
            && (kind == "PersistentVolumeClaim"
                || (observed.operation_id.is_some()
                    && observed.runtime_generation.is_some()
                    && observed.credential_epoch.is_some()))
            && observed.profile.is_some()
            && text("/metadata/labels/nyxid.dev~1managed") == Some("true")
            && text("/kind").is_none_or(|value| value == kind);
        observed
    }
}

fn text_at(object: &Value, path: &str) -> Option<String> {
    object
        .pointer(path)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn string_array_at(object: &Value, path: &str) -> Vec<String> {
    object
        .pointer(path)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

impl OwnedResourceObservation {
    fn from_object(object: &Value) -> Option<Self> {
        let owners = object
            .pointer("/metadata/ownerReferences")
            .and_then(Value::as_array);
        let controller = owners
            .filter(|owners| owners.len() == 1)
            .and_then(|owners| owners.first())
            .filter(|owner| {
                owner.get("controller").and_then(Value::as_bool) == Some(true)
                    && owner.get("kind").and_then(Value::as_str) == Some(KIND)
                    && owner.get("apiVersion").and_then(Value::as_str)
                        == Some("agents.x-k8s.io/v1beta1")
            });
        let controller_owner_uid = controller
            .and_then(|owner| owner.get("uid"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let container_names = object.pointer("/spec/containers").and_then(Value::as_array);
        let statuses = object
            .pointer("/status/containerStatuses")
            .and_then(Value::as_array);
        let terminated = |status: &Value| {
            status
                .pointer("/state/terminated")
                .is_some_and(Value::is_object)
        };
        let all_containers_terminated = container_names.is_some_and(|containers| {
            !containers.is_empty()
                && statuses.is_some_and(|statuses| {
                    containers.len() == statuses.len()
                        && containers.iter().all(|container| {
                            let name = container.get("name").and_then(Value::as_str);
                            name.is_some()
                                && statuses
                                    .iter()
                                    .filter(|status| {
                                        status.get("name").and_then(Value::as_str) == name
                                    })
                                    .count()
                                    == 1
                                && statuses.iter().any(|status| {
                                    status.get("name").and_then(Value::as_str) == name
                                        && terminated(status)
                                })
                        })
                })
        }) && object
            .pointer("/status/initContainerStatuses")
            .and_then(Value::as_array)
            .is_none_or(|statuses| statuses.iter().all(terminated));
        Some(Self {
            kind: text_at(object, "/kind").unwrap_or_default(),
            name: text_at(object, "/metadata/name")?,
            namespace: text_at(object, "/metadata/namespace"),
            uid: text_at(object, "/metadata/uid"),
            resource_version: text_at(object, "/metadata/resourceVersion"),
            creation_timestamp: text_at(object, "/metadata/creationTimestamp"),
            deletion_timestamp: text_at(object, "/metadata/deletionTimestamp"),
            deletion_grace_period_seconds: object
                .pointer("/metadata/deletionGracePeriodSeconds")
                .and_then(Value::as_i64),
            finalizers: string_array_at(object, "/metadata/finalizers"),
            owner_uids: owners
                .into_iter()
                .flatten()
                .filter_map(|owner| owner.get("uid").and_then(Value::as_str))
                .map(str::to_owned)
                .collect(),
            ownership_valid: controller_owner_uid.is_some(),
            controller_owner_uid,
            phase: text_at(object, "/status/phase"),
            node_name: text_at(object, "/spec/nodeName"),
            machine_id: text_at(object, "/metadata/labels/nyxid.dev~1machine-id"),
            owner_id: text_at(object, "/metadata/labels/nyxid.dev~1owner-id"),
            operation_id: text_at(object, "/metadata/labels/nyxid.dev~1operation-id"),
            runtime_generation: text_at(object, "/metadata/labels/nyxid.dev~1runtime-generation"),
            credential_epoch: text_at(object, "/metadata/labels/nyxid.dev~1credential-epoch"),
            writer_fence: text_at(object, "/metadata/labels/nyxid.dev~1writer-fence"),
            workspace_claim_name: object
                .pointer("/spec/volumes")
                .and_then(Value::as_array)
                .and_then(|volumes| {
                    volumes.iter().find(|volume| {
                        volume.get("name").and_then(Value::as_str) == Some("workspace")
                    })
                })
                .and_then(|volume| text_at(volume, "/persistentVolumeClaim/claimName")),
            volume_name: text_at(object, "/spec/volumeName"),
            all_containers_terminated,
        })
    }
}

impl NodeObservation {
    fn from_object(object: &Value) -> Result<Self> {
        let conditions = object
            .pointer("/status/conditions")
            .and_then(Value::as_array);
        let ready_conditions: Vec<_> = conditions
            .into_iter()
            .flatten()
            .filter(|condition| condition.get("type").and_then(Value::as_str) == Some("Ready"))
            .collect();
        let ready_condition = (ready_conditions.len() == 1).then(|| ready_conditions[0]);
        let unreachable = object
            .pointer("/spec/taints")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|taint| {
                matches!(
                    taint.get("key").and_then(Value::as_str),
                    Some("node.kubernetes.io/unreachable" | "node.kubernetes.io/not-ready")
                )
            })
            || conditions.into_iter().flatten().any(|condition| {
                condition.get("type").and_then(Value::as_str) == Some("NetworkUnavailable")
                    && condition.get("status").and_then(Value::as_str) == Some("True")
            });
        let deletion_timestamp = text_at(object, "/metadata/deletionTimestamp");
        Ok(Self {
            name: text_at(object, "/metadata/name").context("Node response has no name")?,
            uid: text_at(object, "/metadata/uid"),
            resource_version: text_at(object, "/metadata/resourceVersion"),
            ready: deletion_timestamp.is_none()
                && !unreachable
                && ready_condition.is_some_and(|condition| {
                    condition.get("status").and_then(Value::as_str) == Some("True")
                }),
            ready_last_heartbeat_time: ready_condition
                .and_then(|condition| text_at(condition, "/lastHeartbeatTime")),
            container_runtime_version: text_at(object, "/status/nodeInfo/containerRuntimeVersion"),
            deletion_timestamp,
        })
    }
}

impl PersistentVolumeObservation {
    fn from_object(object: &Value) -> Result<Self> {
        Ok(Self {
            name: text_at(object, "/metadata/name")
                .context("PersistentVolume response has no name")?,
            uid: text_at(object, "/metadata/uid"),
            resource_version: text_at(object, "/metadata/resourceVersion"),
            claim_uid: text_at(object, "/spec/claimRef/uid"),
            claim_namespace: text_at(object, "/spec/claimRef/namespace"),
            claim_name: text_at(object, "/spec/claimRef/name"),
            reclaim_policy: text_at(object, "/spec/persistentVolumeReclaimPolicy"),
            phase: text_at(object, "/status/phase"),
            deletion_timestamp: text_at(object, "/metadata/deletionTimestamp"),
            finalizers: string_array_at(object, "/metadata/finalizers"),
            csi_driver: text_at(object, "/spec/csi/driver"),
            csi_volume_handle: text_at(object, "/spec/csi/volumeHandle"),
        })
    }
}

impl NodeLeaseObservation {
    fn from_object(object: &Value) -> Result<Self> {
        let node_owners: Vec<_> = object
            .pointer("/metadata/ownerReferences")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|owner| {
                owner.get("apiVersion").and_then(Value::as_str) == Some("v1")
                    && owner.get("kind").and_then(Value::as_str) == Some("Node")
            })
            .collect();
        Ok(Self {
            name: text_at(object, "/metadata/name").context("Lease name missing")?,
            namespace: text_at(object, "/metadata/namespace").context("Lease namespace missing")?,
            uid: text_at(object, "/metadata/uid"),
            resource_version: text_at(object, "/metadata/resourceVersion"),
            owner_node_uid: (node_owners.len() == 1)
                .then(|| text_at(node_owners[0], "/uid"))
                .flatten(),
            holder_identity: text_at(object, "/spec/holderIdentity"),
            renew_time: text_at(object, "/spec/renewTime"),
            lease_duration_seconds: object
                .pointer("/spec/leaseDurationSeconds")
                .and_then(Value::as_i64),
            deletion_timestamp: text_at(object, "/metadata/deletionTimestamp"),
        })
    }
}

impl VolumeAttachmentObservation {
    fn from_object(object: &Value) -> Result<Self> {
        let source = object
            .pointer("/spec/source")
            .and_then(Value::as_object)
            .context("VolumeAttachment source invalid")?;
        match (
            source.get("persistentVolumeName"),
            source.get("inlineVolumeSpec"),
        ) {
            (Some(Value::String(name)), None) => validate_resource_segment(name)?,
            (None, Some(Value::Object(_))) => (),
            _ => return Err(anyhow!("VolumeAttachment source invalid")),
        }
        for field in ["nodeName", "attacher"] {
            validate_resource_segment(
                object
                    .pointer(&format!("/spec/{field}"))
                    .and_then(Value::as_str)
                    .context("VolumeAttachment identity invalid")?,
            )?;
        }
        if let Some(attached) = object.pointer("/status/attached") {
            ensure!(
                attached.is_boolean(),
                "VolumeAttachment attached status invalid"
            );
        }
        Ok(Self {
            name: text_at(object, "/metadata/name").context("VolumeAttachment name missing")?,
            uid: text_at(object, "/metadata/uid"),
            resource_version: text_at(object, "/metadata/resourceVersion"),
            persistent_volume_name: text_at(object, "/spec/source/persistentVolumeName"),
            node_name: text_at(object, "/spec/nodeName"),
            attacher: text_at(object, "/spec/attacher"),
            attached: object.pointer("/status/attached").and_then(Value::as_bool),
            deletion_timestamp: text_at(object, "/metadata/deletionTimestamp"),
        })
    }
}

impl SandboxObservation {
    pub fn from_object(object: &Value) -> Result<Self> {
        let metadata = object
            .get("metadata")
            .and_then(Value::as_object)
            .context("sandbox response has no metadata")?;
        let spec = object.get("spec").and_then(Value::as_object);
        let status = object.get("status").and_then(Value::as_object);
        let labels = metadata.get("labels").and_then(Value::as_object);
        let conditions = status
            .and_then(|status| status.get("conditions"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|condition| serde_json::from_value(condition).ok())
            .collect::<Vec<Condition>>();
        let generation = metadata.get("generation").and_then(Value::as_i64);
        // The pinned upstream CRD uses metav1.Condition.observedGeneration;
        // status.observedGeneration does not exist. Duplicate conditions are
        // malformed observations, not an opportunity to select a favorable one.
        let condition = |kind: &str| {
            let mut matching = conditions
                .iter()
                .filter(|condition| condition.condition_type == kind);
            let first = matching.next();
            if matching.next().is_some() {
                None
            } else {
                first
            }
        };
        let current_condition = |kind: &str| {
            condition(kind).filter(|condition| {
                generation
                    .is_some_and(|value| value > 0 && condition.observed_generation == Some(value))
            })
        };
        let desired_suspended = spec
            .and_then(|spec| spec.get("operatingMode"))
            .and_then(Value::as_str)
            == Some("Suspended");
        Ok(Self {
            name: metadata
                .get("name")
                .and_then(Value::as_str)
                .context("sandbox response has no name")?
                .to_owned(),
            namespace: metadata
                .get("namespace")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            sandbox_uid: metadata
                .get("uid")
                .and_then(Value::as_str)
                .map(str::to_owned),
            runtime_generation: labels
                .and_then(|labels| labels.get("nyxid.dev/runtime-generation"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            credential_epoch: labels
                .and_then(|labels| labels.get("nyxid.dev/credential-epoch"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            writer_fence: labels
                .and_then(|labels| labels.get("nyxid.dev/writer-fence"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            machine_id: labels
                .and_then(|labels| labels.get("nyxid.dev/machine-id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            owner_id: labels
                .and_then(|labels| labels.get("nyxid.dev/owner-id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            operation_id: labels
                .and_then(|labels| labels.get("nyxid.dev/operation-id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            profile: labels
                .and_then(|labels| labels.get("nyxid.dev/profile"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            ready: !desired_suspended
                && current_condition("Ready").is_some_and(|condition| condition.status == "True"),
            desired_suspended,
            suspended: desired_suspended
                && current_condition("Suspended").is_some_and(|condition| {
                    condition.status == "True"
                        && condition.reason.as_deref() == Some("PodTerminated")
                }),
            generation,
            deletion_timestamp: metadata
                .get("deletionTimestamp")
                .and_then(Value::as_str)
                .map(str::to_owned),
            node_name: status
                .and_then(|status| status.get("nodeName"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            tracked_pod_name: metadata
                .get("annotations")
                .and_then(|value| value.get("agents.x-k8s.io/pod-name"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            expired: current_condition("Ready")
                .is_some_and(|condition| condition.reason.as_deref() == Some("SandboxExpired")),
            shutdown_time: spec
                .and_then(|spec| spec.get("shutdownTime"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            observed_generation: condition(if desired_suspended {
                "Suspended"
            } else {
                "Ready"
            })
            .and_then(|condition| condition.observed_generation),
            resource_version: metadata
                .get("resourceVersion")
                .and_then(Value::as_str)
                .map(str::to_owned),
            conditions,
        })
    }
}

fn validate_resource_segment(value: &str) -> Result<()> {
    ensure!(dns_subdomain(value), "invalid Kubernetes resource name");
    Ok(())
}

fn validate_identity_token(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control),
        "Kubernetes identity precondition is invalid"
    );
    Ok(())
}

/// Recovery inventories are deliberately stricter than diagnostic listings.
/// Dropping an unparseable object could hide a live writer or late volume.
fn validate_recovery_object(
    object: &Value,
    kind: &str,
    version: &str,
    namespaced: bool,
) -> Result<()> {
    ensure!(
        object.get("kind").and_then(Value::as_str) == Some(kind)
            && object.get("apiVersion").and_then(Value::as_str) == Some(version),
        "Kubernetes recovery resource type invalid"
    );
    validate_resource_segment(
        object
            .pointer("/metadata/name")
            .and_then(Value::as_str)
            .context("Kubernetes recovery resource name missing")?,
    )?;
    for path in ["/metadata/uid", "/metadata/resourceVersion"] {
        validate_identity_token(
            object
                .pointer(path)
                .and_then(Value::as_str)
                .context("Kubernetes recovery resource identity missing")?,
        )?;
    }
    if namespaced {
        validate_resource_segment(
            object
                .pointer("/metadata/namespace")
                .and_then(Value::as_str)
                .context("Kubernetes recovery namespace missing")?,
        )?;
    }
    if let Some(owners) = object.pointer("/metadata/ownerReferences") {
        for owner in owners
            .as_array()
            .context("Kubernetes recovery owner references invalid")?
        {
            for field in ["apiVersion", "kind", "name", "uid"] {
                validate_identity_token(
                    owner
                        .get(field)
                        .and_then(Value::as_str)
                        .context("Kubernetes recovery owner reference invalid")?,
                )?;
            }
            for field in ["controller", "blockOwnerDeletion"] {
                if let Some(value) = owner.get(field) {
                    ensure!(value.is_boolean(), "Kubernetes recovery owner flag invalid");
                }
            }
        }
    }
    if kind == "PersistentVolume"
        && let Some(claim) = object.pointer("/spec/claimRef")
    {
        for field in ["name", "namespace"] {
            validate_resource_segment(
                claim
                    .get(field)
                    .and_then(Value::as_str)
                    .context("PersistentVolume claim reference invalid")?,
            )?;
        }
        if let Some(uid) = claim.get("uid") {
            validate_identity_token(uid.as_str().context("PersistentVolume claim UID invalid")?)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ObservationContext {
    resource_kind: ObservationResourceKind,
    verb: ObservationVerb,
    http_status: Option<u16>,
}

impl ObservationContext {
    fn new(resource_kind: ObservationResourceKind, verb: ObservationVerb) -> Self {
        Self {
            resource_kind,
            verb,
            http_status: None,
        }
    }

    fn received(self, response: &reqwest::Response) -> Self {
        Self {
            http_status: Some(response.status().as_u16()),
            ..self
        }
    }

    fn error(self, stage: ObservationStage, error_kind: ObservationErrorKind) -> anyhow::Error {
        SandboxObservationError {
            stage,
            resource_kind: self.resource_kind,
            verb: self.verb,
            error_kind,
            http_status: self.http_status,
        }
        .into()
    }

    fn transport(self, stage: ObservationStage, error: reqwest::Error) -> anyhow::Error {
        self.error(
            stage,
            if error.is_timeout() {
                ObservationErrorKind::Timeout
            } else {
                ObservationErrorKind::Transport
            },
        )
    }

    fn check_status(self, response: &reqwest::Response) -> Result<Self> {
        let context = self.received(response);
        if response.status().is_success() {
            Ok(context)
        } else {
            Err(context.error(
                ObservationStage::Request,
                match response.status() {
                    StatusCode::UNAUTHORIZED => ObservationErrorKind::Authentication,
                    StatusCode::FORBIDDEN => ObservationErrorKind::Forbidden,
                    _ => ObservationErrorKind::HttpStatus,
                },
            ))
        }
    }
}

/// Called only after the enclosing typed List has passed validation. Explicit
/// null or malformed TypeMeta never receives the omitted-field normalization.
fn normalize_recovery_list_item(
    object: &Value,
    kind: &str,
    version: &str,
    namespaced: bool,
    context: ObservationContext,
) -> Result<Value> {
    let mut normalized = object.clone();
    let fields = normalized.as_object_mut().ok_or_else(|| {
        context.error(
            ObservationStage::ValidateItem,
            ObservationErrorKind::InvalidResponse,
        )
    })?;
    for (field, expected) in [("kind", kind), ("apiVersion", version)] {
        match fields.get(field) {
            None => {
                fields.insert(field.into(), Value::String(expected.into()));
            }
            Some(Value::String(actual)) if actual == expected => (),
            Some(_) => {
                return Err(context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                ));
            }
        }
    }
    validate_recovery_object(&normalized, kind, version, namespaced).map_err(|_| {
        context.error(
            ObservationStage::ValidateIdentity,
            ObservationErrorKind::InvalidIdentity,
        )
    })?;
    Ok(normalized)
}

#[derive(Clone)]
pub struct SandboxApi {
    client: Client,
    base_url: Url,
    credentials: Option<KubernetesCredentials>,
    field_manager: String,
}

impl fmt::Debug for SandboxApi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SandboxApi")
            .field("base_url", &self.base_url)
            .field("credentials", &self.credentials)
            .field("field_manager", &self.field_manager)
            .finish()
    }
}

impl SandboxApi {
    fn validate_endpoint(base_url: &Url) -> Result<()> {
        ensure!(
            base_url.scheme() == "https"
                || (base_url.scheme() == "http"
                    && base_url
                        .host_str()
                        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"))),
            "Kubernetes API requires HTTPS outside localhost"
        );
        ensure!(
            base_url.username().is_empty()
                && base_url.password().is_none()
                && base_url.query().is_none()
                && base_url.fragment().is_none(),
            "Kubernetes API credentials must not appear in its URL"
        );
        Ok(())
    }

    pub fn new(base_url: Url, bearer_token: Option<String>) -> Result<Self> {
        Self::validate_endpoint(&base_url)?;
        Ok(Self {
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
            base_url,
            credentials: bearer_token.map(KubernetesCredentials::static_token),
            field_manager: "nyxid-machine-manager".to_string(),
        })
    }

    pub fn with_client(
        base_url: Url,
        bearer_token: Option<String>,
        client: Client,
    ) -> Result<Self> {
        Self::validate_endpoint(&base_url)?;
        Ok(Self {
            client,
            base_url,
            credentials: bearer_token.map(KubernetesCredentials::static_token),
            field_manager: "nyxid-machine-manager".to_string(),
        })
    }

    /// Production projected credentials rotate independently of this client.
    /// Reload the protected file for every request; no cached-token fallback.
    pub fn with_token_file_client(
        base_url: Url,
        token_file: PathBuf,
        client: Client,
    ) -> Result<Self> {
        Self::with_credentials_client(
            base_url,
            KubernetesCredentials::token_file(token_file)?,
            client,
        )
    }

    pub fn with_credentials_client(
        base_url: Url,
        credentials: KubernetesCredentials,
        client: Client,
    ) -> Result<Self> {
        Self::validate_endpoint(&base_url)?;
        Ok(Self {
            client,
            base_url,
            credentials: Some(credentials),
            field_manager: "nyxid-machine-manager".to_string(),
        })
    }

    async fn authorize(&self, request: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        match &self.credentials {
            Some(credentials) => credentials.authorize(request).await,
            None => Ok(request),
        }
    }

    fn resource_url(&self, namespace: &str, name: Option<&str>) -> Result<Url> {
        ensure!(
            !namespace.is_empty() && !namespace.contains('/'),
            "invalid namespace"
        );
        if let Some(name) = name {
            ensure!(
                !name.is_empty() && !name.contains('/'),
                "invalid sandbox name"
            );
        }
        let mut url = self.base_url.clone();
        let path = format!(
            "/apis/{API_GROUP}/{API_VERSION}/namespaces/{namespace}/sandboxes{}",
            name.map(|name| format!("/{name}")).unwrap_or_default()
        );
        url.set_path(&path);
        Ok(url)
    }

    async fn request(&self, method: Method, url: Url) -> Result<reqwest::Response> {
        let request = self.authorize(self.client.request(method, url)).await?;
        Ok(request.send().await?)
    }

    async fn observation_send(
        &self,
        request: reqwest::RequestBuilder,
        context: ObservationContext,
    ) -> Result<reqwest::Response> {
        let request = self.authorize(request).await.map_err(|_| {
            context.error(
                ObservationStage::Request,
                ObservationErrorKind::CredentialUnavailable,
            )
        })?;
        request
            .send()
            .await
            .map_err(|error| context.transport(ObservationStage::Request, error))
    }

    async fn observation_json(
        mut response: reqwest::Response,
        context: ObservationContext,
    ) -> Result<Value> {
        const MAX_BYTES: usize = 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err(context.error(
                ObservationStage::DecodeResponse,
                ObservationErrorKind::InventoryLimit,
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| context.transport(ObservationStage::DecodeResponse, error))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                return Err(context.error(
                    ObservationStage::DecodeResponse,
                    ObservationErrorKind::InventoryLimit,
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            context.error(
                ObservationStage::DecodeResponse,
                ObservationErrorKind::InvalidResponse,
            )
        })
    }

    async fn observation_optional_object(
        &self,
        url: Url,
        context: ObservationContext,
    ) -> Result<Option<(Value, ObservationContext)>> {
        let mut current_context = context;
        let mut stage = ObservationStage::Request;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let response = self.observation_send(self.client.get(url), context).await?;
            current_context = context.received(&response);
            if response.status() == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            let context = context.check_status(&response)?;
            stage = ObservationStage::DecodeResponse;
            Ok(Some((
                Self::observation_json(response, context).await?,
                context,
            )))
        })
        .await
        .map_err(|_| current_context.error(stage, ObservationErrorKind::Timeout))?
    }

    /// Resolve only an API-valid class. This never certifies CSI availability,
    /// capacity, binding or physical storage. The renderer still omits the field
    /// for a default selection so Kubernetes owns the actual defaulting decision.
    pub async fn resolve_storage_class(&self, configured: Option<&str>) -> Result<String> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut base = self.base_url.clone();
            base.set_path("/apis/storage.k8s.io/v1/storageclasses");
            if let Some(name) = configured {
                validate_storage_class_name(name)?;
                base.set_path(&format!("/apis/storage.k8s.io/v1/storageclasses/{name}"));
                return validate_storage_class(&self.read_storage_object(base).await?, name);
            }
            let mut defaults = Vec::new();
            let mut continuation: Option<String> = None;
            let mut seen = std::collections::HashSet::new();
            let mut names = std::collections::HashSet::new();
            let mut resource_version = None;
            for _ in 0..10 {
                let mut url = base.clone();
                url.query_pairs_mut().append_pair("limit", "100");
                if let Some(token) = &continuation {
                    url.query_pairs_mut().append_pair("continue", token);
                }
                let page = self.read_storage_object(url).await?;
                ensure!(
                    page.get("kind").and_then(Value::as_str) == Some("StorageClassList")
                        && page.get("apiVersion").and_then(Value::as_str)
                            == Some("storage.k8s.io/v1"),
                    "StorageClass inventory type invalid"
                );
                let version = page
                    .pointer("/metadata/resourceVersion")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .context("StorageClass inventory version missing")?;
                ensure!(
                    resource_version
                        .as_deref()
                        .is_none_or(|previous| previous == version),
                    "StorageClass inventory changed during pagination"
                );
                resource_version = Some(version.to_owned());
                let items = page
                    .get("items")
                    .and_then(Value::as_array)
                    .context("StorageClass inventory missing")?;
                ensure!(items.len() <= 100, "StorageClass inventory page too large");
                for object in items {
                    let name = object
                        .pointer("/metadata/name")
                        .and_then(Value::as_str)
                        .context("StorageClass name missing")?;
                    ensure!(
                        dns_subdomain(name) && names.insert(name.to_owned()),
                        "StorageClass inventory identity invalid"
                    );
                    if [
                        "storageclass.kubernetes.io/is-default-class",
                        "storageclass.beta.kubernetes.io/is-default-class",
                    ]
                    .iter()
                    .any(|key| object["metadata"]["annotations"][key] == "true")
                    {
                        let created = object
                            .pointer("/metadata/creationTimestamp")
                            .and_then(Value::as_str)
                            .context("Default StorageClass creation time missing")?;
                        let created = chrono::DateTime::parse_from_rfc3339(created)
                            .context("Default StorageClass creation time invalid")?;
                        defaults.push((created, name.to_owned(), object.clone()));
                    }
                }
                continuation = page
                    .pointer("/metadata/continue")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                let Some(next) = &continuation else {
                    // Kubernetes uses the newest default, then lexical name for
                    // equal creation timestamps. Never silently use an older
                    // class when that selected class is deleting or invalid.
                    defaults.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                    let (_, name, object) = defaults
                        .first()
                        .context("Cluster default StorageClass unavailable")?;
                    return validate_storage_class(object, name);
                };
                ensure!(
                    next.len() <= 16 * 1024 && seen.insert(next.clone()),
                    "StorageClass continuation invalid"
                );
            }
            Err(anyhow!("StorageClass inventory exceeds bounded limit"))
        })
        .await
        .context("StorageClass verification timed out")?
    }

    async fn read_storage_object(&self, url: Url) -> Result<Value> {
        const MAX_BYTES: usize = 256 * 1024;
        let mut response = self.request(Method::GET, url).await?;
        ensure!(
            response.status().is_success(),
            "StorageClass verification failed ({})",
            response.status()
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|len| len <= MAX_BYTES as u64),
            "StorageClass response too large"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_BYTES,
                "StorageClass response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("StorageClass response invalid")
    }

    pub async fn health(&self) -> Result<()> {
        let mut url = self.base_url.clone();
        url.set_path("/apis/agents.x-k8s.io/v1beta1");
        let response = self.request(Method::GET, url).await?;
        ensure!(
            response.status().is_success(),
            "Sandbox API unavailable ({})",
            response.status()
        );
        Ok(())
    }

    pub async fn get(&self, namespace: &str, name: &str) -> Result<Option<SandboxObservation>> {
        let context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Get);
        let url = self.resource_url(namespace, Some(name)).map_err(|_| {
            context.error(
                ObservationStage::ValidateIdentity,
                ObservationErrorKind::InvalidIdentity,
            )
        })?;
        let Some((object, context)) = self.observation_optional_object(url, context).await? else {
            return Ok(None);
        };
        Ok(Some(SandboxObservation::from_object(&object).map_err(
            |_| {
                context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                )
            },
        )?))
    }

    /// Read by deterministic name and reject a same-name object belonging to a
    /// different machine, owner, operation or profile. A name collision is an
    /// investigation condition; it is never silently adopted.
    pub async fn get_for_intent(
        &self,
        intent: &SandboxIntent,
    ) -> Result<Option<SandboxObservation>> {
        intent.validate()?;
        let Some(observation) = self.get(&intent.namespace, &intent.name).await? else {
            return Ok(None);
        };
        ensure!(
            observation.machine_id.as_deref() == Some(intent.machine_id.as_str())
                && observation.owner_id.as_deref() == Some(intent.owner_id.as_str())
                && observation.operation_id.as_deref() == Some(intent.operation_id.as_str())
                && observation.profile.as_deref() == Some(intent.profile.as_str())
                && observation.runtime_generation.as_deref()
                    == Some(intent.runtime_generation.to_string().as_str())
                && observation.credential_epoch.as_deref()
                    == Some(intent.credential_epoch.to_string().as_str())
                && observation.writer_fence.as_deref() == Some(intent.writer_fence.as_str()),
            "Sandbox identity mismatch for deterministic name"
        );
        Ok(Some(observation))
    }

    pub async fn apply(&self, intent: &SandboxIntent) -> Result<SandboxObservation> {
        intent.validate()?;
        // API-level admission only; PVC binding/provisioning remains observed
        // work. StorageClass has no Ready condition and proves no CSI capacity.
        self.resolve_storage_class(intent.workspace_storage_class.as_deref())
            .await?;
        let request = self
            .client
            .patch(self.resource_url(&intent.namespace, Some(&intent.name))?)
            .query(&[
                ("fieldManager", self.field_manager.as_str()),
                ("force", "false"),
            ])
            // Kubernetes SSA accepts YAML (including its JSON subset), not
            // application/apply-patch+json.
            .header(header::CONTENT_TYPE, "application/apply-patch+yaml")
            .json(&intent.apply_object());
        let context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Patch);
        let response = self.observation_send(request, context).await?;
        let context = context.check_status(&response)?;
        SandboxObservation::from_object(&Self::observation_json(response, context).await?).map_err(
            |_| {
                context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                )
            },
        )
    }

    /// Apply only when the live object still has the expected UID. This is the
    /// lost-response/reconciliation guard: a deterministic name is never
    /// allowed to adopt a replacement object silently.
    pub async fn apply_if_uid(
        &self,
        intent: &SandboxIntent,
        expected_uid: &str,
    ) -> Result<SandboxObservation> {
        intent.validate()?;
        let get_context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Get);
        let (current_object, get_context) = self
            .observation_optional_object(
                self.resource_url(&intent.namespace, Some(&intent.name))?,
                get_context,
            )
            .await?
            .context("Sandbox disappeared before UID-preconditioned apply")?;
        let current = SandboxObservation::from_object(&current_object).map_err(|_| {
            get_context.error(
                ObservationStage::ValidateItem,
                ObservationErrorKind::InvalidResponse,
            )
        })?;
        ensure!(
            current.sandbox_uid.as_deref() == Some(expected_uid),
            "Sandbox UID precondition failed"
        );
        let mut body = intent.apply_object();
        // A retained template is the storage authority, including an omitted
        // class. Today's deployment setting/default must not migrate a disk.
        // Validate the complete fixed template after substituting only its
        // original class; all ownership, size and shape checks remain exact.
        let mut retained = intent.clone();
        retained.workspace_storage_class =
            match current_object.pointer("/spec/volumeClaimTemplates/0/spec/storageClassName") {
                None => None,
                Some(Value::String(name)) => {
                    validate_storage_class_name(name)?;
                    Some(name.clone())
                }
                Some(_) => return Err(anyhow!("Sandbox retained storage class is invalid")),
            };
        let retained_body = retained.apply_object();
        ensure!(
            current_object.pointer("/spec/volumeClaimTemplates")
                == retained_body.pointer("/spec/volumeClaimTemplates"),
            "Sandbox workspace template ownership or storage conflict"
        );
        body["spec"]["volumeClaimTemplates"] =
            retained_body["spec"]["volumeClaimTemplates"].clone();
        body["metadata"]["uid"] = json!(expected_uid);
        body["metadata"]["resourceVersion"] = json!(
            current
                .resource_version
                .context("Sandbox has no resourceVersion")?
        );
        let request = self
            .client
            .patch(self.resource_url(&intent.namespace, Some(&intent.name))?)
            .query(&[
                ("fieldManager", self.field_manager.as_str()),
                ("force", "false"),
            ])
            .header(header::CONTENT_TYPE, "application/apply-patch+yaml")
            .json(&body);
        let context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Patch);
        let response = self.observation_send(request, context).await?;
        let context = context.check_status(&response)?;
        SandboxObservation::from_object(&Self::observation_json(response, context).await?).map_err(
            |_| {
                context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                )
            },
        )
    }

    pub async fn delete(&self, namespace: &str, name: &str) -> Result<bool> {
        let context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Delete);
        let url = self.resource_url(namespace, Some(name)).map_err(|_| {
            context.error(
                ObservationStage::ValidateIdentity,
                ObservationErrorKind::InvalidIdentity,
            )
        })?;
        let response = self
            .observation_send(self.client.delete(url), context)
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        context.check_status(&response)?;
        Ok(true)
    }

    /// Suspend the exact original object without rendering today's Pod/PVC
    /// templates. Kubernetes atomically tests UID/RV and the original tuple;
    /// an object replaced or changed after the read rejects the whole patch.
    /// This does not certify controller acknowledgement or physical stopping.
    pub async fn suspend_if_identity(
        &self,
        intent: &SandboxIntent,
        expected_uid: &str,
        expected_resource_version: &str,
    ) -> Result<SandboxObservation> {
        validate_resource_segment(&intent.namespace)?;
        validate_resource_segment(&intent.name)?;
        validate_identity_token(expected_uid)?;
        validate_identity_token(expected_resource_version)?;
        validate_identity_token(&intent.operation_id)?;
        let get_context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Get);
        let url = self.resource_url(&intent.namespace, Some(&intent.name))?;
        let (current, get_context) = self
            .observation_optional_object(url.clone(), get_context)
            .await?
            .context("Original Sandbox absent before suspension")?;
        validate_recovery_object(&current, KIND, "agents.x-k8s.io/v1beta1", true).map_err(
            |_| {
                get_context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                )
            },
        )?;
        ensure!(
            current.pointer("/metadata/name").and_then(Value::as_str) == Some(intent.name.as_str())
                && current
                    .pointer("/metadata/namespace")
                    .and_then(Value::as_str)
                    == Some(intent.namespace.as_str()),
            "Original Sandbox identity conflict"
        );
        let expected = [
            ("/metadata/uid", expected_uid.to_owned()),
            (
                "/metadata/resourceVersion",
                expected_resource_version.to_owned(),
            ),
            (
                "/metadata/labels/nyxid.dev~1owner-id",
                intent.owner_id.clone(),
            ),
            (
                "/metadata/labels/nyxid.dev~1machine-id",
                intent.machine_id.clone(),
            ),
            (
                "/metadata/labels/nyxid.dev~1runtime-generation",
                intent.runtime_generation.to_string(),
            ),
            (
                "/metadata/labels/nyxid.dev~1credential-epoch",
                intent.credential_epoch.to_string(),
            ),
            (
                "/metadata/labels/nyxid.dev~1writer-fence",
                intent.writer_fence.clone(),
            ),
        ];
        let mut patch = Vec::new();
        for (path, value) in expected {
            validate_identity_token(&value)?;
            ensure!(
                current.pointer(path).and_then(Value::as_str) == Some(value.as_str()),
                "Original Sandbox identity precondition failed"
            );
            patch.push(json!({"op":"test","path":path,"value":value}));
        }
        patch.push(json!({"op":"add","path":"/metadata/labels/nyxid.dev~1operation-id","value":intent.operation_id}));
        patch.push(json!({"op":"add","path":"/spec/operatingMode","value":"Suspended"}));
        if current.pointer("/spec/shutdownTime").is_some() {
            patch.push(json!({"op":"remove","path":"/spec/shutdownTime"}));
        }
        let request = self
            .client
            .patch(url)
            .header(header::CONTENT_TYPE, "application/json-patch+json")
            .json(&patch);
        let context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Patch);
        let response = self.observation_send(request, context).await?;
        let context = context.check_status(&response)?;
        let observation =
            SandboxObservation::from_object(&Self::observation_json(response, context).await?)
                .map_err(|_| {
                    context.error(
                        ObservationStage::ValidateItem,
                        ObservationErrorKind::InvalidResponse,
                    )
                })?;
        if !(observation.sandbox_uid.as_deref() == Some(expected_uid)
            && observation.name == intent.name
            && observation.namespace == intent.namespace
            && observation.operation_id.as_deref() == Some(intent.operation_id.as_str())
            && observation.desired_suspended)
        {
            return Err(context.error(
                ObservationStage::ValidateIdentity,
                ObservationErrorKind::InvalidIdentity,
            ));
        }
        Ok(observation)
    }

    /// Delete acceptance is not completion evidence. The server evaluates both
    /// preconditions atomically; 404 only skips this precise resource's DELETE.
    /// Foreground propagation leaves dependent-object cleanup to Kubernetes GC.
    pub async fn delete_if_identity(
        &self,
        namespace: &str,
        name: &str,
        expected_uid: &str,
        expected_resource_version: &str,
    ) -> Result<bool> {
        validate_resource_segment(namespace)?;
        validate_resource_segment(name)?;
        self.delete_url_if_identity(
            self.resource_url(namespace, Some(name))?,
            ObservationResourceKind::Sandbox,
            expected_uid,
            expected_resource_version,
        )
        .await
    }

    /// Restricted residual-child cleanup. Caller authorization and original
    /// ownership verification are required before this exact-identity request.
    /// No force/grace override, finalizer patch, PV or cloud delete is exposed.
    pub async fn delete_resource_if_identity(
        &self,
        namespace: &str,
        resource: &str,
        name: &str,
        expected_uid: &str,
        expected_resource_version: &str,
    ) -> Result<bool> {
        ensure!(
            matches!(resource, "pods" | "persistentvolumeclaims"),
            "unsupported cleanup resource"
        );
        validate_resource_segment(namespace)?;
        validate_resource_segment(name)?;
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/v1/namespaces/{namespace}/{resource}/{name}"));
        let kind = if resource == "pods" {
            ObservationResourceKind::Pod
        } else {
            ObservationResourceKind::PersistentVolumeClaim
        };
        self.delete_url_if_identity(url, kind, expected_uid, expected_resource_version)
            .await
    }

    async fn delete_url_if_identity(
        &self,
        url: Url,
        kind: ObservationResourceKind,
        expected_uid: &str,
        expected_resource_version: &str,
    ) -> Result<bool> {
        validate_identity_token(expected_uid)?;
        validate_identity_token(expected_resource_version)?;
        let mut options = json!({"preconditions": {
            "uid": expected_uid, "resourceVersion": expected_resource_version
        }});
        if kind == ObservationResourceKind::Sandbox {
            options["propagationPolicy"] = json!("Foreground");
        }
        let request = self
            .client
            .delete(url)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&options);
        let context = ObservationContext::new(kind, ObservationVerb::Delete);
        let response = self.observation_send(request, context).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        context.check_status(&response)?;
        Ok(true)
    }

    pub async fn delete_if_uid(
        &self,
        namespace: &str,
        name: &str,
        expected_uid: &str,
    ) -> Result<bool> {
        let current = self.get(namespace, name).await?;
        let Some(current) = current else {
            return Ok(false);
        };
        ensure!(
            current.sandbox_uid.as_deref() == Some(expected_uid),
            "Sandbox UID precondition failed"
        );
        let request = self
            .client
            .delete(self.resource_url(namespace, Some(name))?)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&json!({"preconditions": {"uid": expected_uid}}));
        let context =
            ObservationContext::new(ObservationResourceKind::Sandbox, ObservationVerb::Delete);
        let response = self.observation_send(request, context).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        context.check_status(&response)?;
        Ok(true)
    }

    /// List managed Sandboxes for orphan inventory. The manager filters by its
    /// immutable managed label and reconciles each object's UID/operation
    /// before making any further change.
    pub async fn list_managed(&self, namespace: &str) -> Result<Vec<SandboxObservation>> {
        ensure!(
            !namespace.is_empty() && !namespace.contains('/'),
            "invalid namespace"
        );
        let mut url = self.base_url.clone();
        url.set_path(&format!(
            "/apis/{API_GROUP}/{API_VERSION}/namespaces/{namespace}/sandboxes"
        ));
        url.query_pairs_mut()
            .append_pair("labelSelector", "nyxid.dev/managed=true");
        self.list_objects(url)
            .await?
            .iter()
            .map(SandboxObservation::from_object)
            .collect::<Result<Vec<_>>>()
    }

    /// List controlled Pod/PVC metadata without needing a live Sandbox UID.
    /// The selector bounds scope; each result is independently validated since
    /// labels alone cannot establish the durable owner or resource binding.
    pub async fn list_controlled_resources(
        &self,
        namespace: &str,
        resource: &str,
    ) -> Result<Vec<ControlledResourceObservation>> {
        let kind = match resource {
            "pods" => "Pod",
            "persistentvolumeclaims" => "PersistentVolumeClaim",
            _ => return Err(anyhow!("unsupported observed resource")),
        };
        ensure!(
            !namespace.is_empty() && !namespace.contains('/'),
            "invalid namespace"
        );
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/v1/namespaces/{namespace}/{resource}"));
        url.query_pairs_mut()
            .append_pair("labelSelector", "nyxid.dev/managed=true");
        Ok(self
            .list_objects(url)
            .await?
            .iter()
            .map(|object| ControlledResourceObservation::from_object(object, kind, namespace))
            .collect())
    }

    /// Bounded Kubernetes inventory pagination. A truncated/invalid response or
    /// repeated continuation is an error; callers cannot mistake it for an
    /// empty inventory and finalize cleanup.
    async fn list_objects(&self, base: Url) -> Result<Vec<Value>> {
        let mut collected = Vec::new();
        let mut continuation: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let mut url = base.clone();
            url.query_pairs_mut().append_pair("limit", "100");
            if let Some(token) = &continuation {
                url.query_pairs_mut().append_pair("continue", token);
            }
            let response = self.request(Method::GET, url).await?;
            ensure!(
                response.status().is_success(),
                "Kubernetes inventory failed ({})",
                response.status()
            );
            let body: Value = response.json().await?;
            let items = body
                .get("items")
                .and_then(Value::as_array)
                .context("Kubernetes inventory has no items array")?;
            ensure!(
                items.len() <= 100,
                "Kubernetes inventory exceeded its page bound"
            );
            collected.extend(items.iter().cloned());
            continuation = body
                .pointer("/metadata/continue")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            let Some(next) = &continuation else {
                return Ok(collected);
            };
            ensure!(
                next.len() <= 16 * 1024 && seen.insert(next.clone()),
                "Kubernetes inventory continuation invalid"
            );
        }
        Err(anyhow!(
            "Kubernetes inventory exceeds the bounded sweep capacity"
        ))
    }

    /// Read one resource by its persisted or controller-declared name. A foreign
    /// replacement is returned to the caller for explicit conflict handling;
    /// it is not hidden by an ownerReference filter.
    pub async fn resource(
        &self,
        namespace: &str,
        resource: &str,
        name: &str,
    ) -> Result<Option<OwnedResourceObservation>> {
        ensure!(
            matches!(resource, "pods" | "persistentvolumeclaims"),
            "unsupported observed resource"
        );
        validate_resource_segment(namespace)?;
        validate_resource_segment(name)?;
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/v1/namespaces/{namespace}/{resource}/{name}"));
        let kind = if resource == "pods" {
            ObservationResourceKind::Pod
        } else {
            ObservationResourceKind::PersistentVolumeClaim
        };
        let context = ObservationContext::new(kind, ObservationVerb::Get);
        let Some((object, context)) = self.observation_optional_object(url, context).await? else {
            return Ok(None);
        };
        Ok(Some(
            OwnedResourceObservation::from_object(&object).ok_or_else(|| {
                context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                )
            })?,
        ))
    }

    pub async fn node(&self, name: &str) -> Result<Option<NodeObservation>> {
        validate_resource_segment(name)?;
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/v1/nodes/{name}"));
        self.read_optional_object(url)
            .await?
            .as_ref()
            .map(NodeObservation::from_object)
            .transpose()
    }

    pub async fn persistent_volume(
        &self,
        name: &str,
    ) -> Result<Option<PersistentVolumeObservation>> {
        validate_resource_segment(name)?;
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/v1/persistentvolumes/{name}"));
        let context = ObservationContext::new(
            ObservationResourceKind::PersistentVolume,
            ObservationVerb::Get,
        );
        self.observation_optional_object(url, context)
            .await?
            .as_ref()
            .map(|(object, context)| {
                PersistentVolumeObservation::from_object(object).map_err(|_| {
                    context.error(
                        ObservationStage::ValidateItem,
                        ObservationErrorKind::InvalidResponse,
                    )
                })
            })
            .transpose()
    }

    async fn read_optional_object(&self, url: Url) -> Result<Option<Value>> {
        let response = self.request(Method::GET, url).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            response.status().is_success(),
            "Kubernetes resource observation failed ({})",
            response.status()
        );
        Ok(Some(response.json().await?))
    }

    async fn read_recovery_json(mut response: reqwest::Response) -> Result<Value> {
        const MAX_BYTES: usize = 1024 * 1024;
        ensure!(
            response
                .content_length()
                .is_none_or(|length| length <= MAX_BYTES as u64),
            "Kubernetes recovery response too large"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_BYTES,
                "Kubernetes recovery response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("Kubernetes recovery response invalid")
    }

    async fn read_recovery_optional_object(&self, url: Url) -> Result<Option<Value>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let response = self.request(Method::GET, url).await?;
            if response.status() == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            ensure!(
                response.status().is_success(),
                "Kubernetes recovery observation failed ({})",
                response.status()
            );
            Ok(Some(Self::read_recovery_json(response).await?))
        })
        .await
        .context("Kubernetes recovery observation timed out")?
    }

    /// A complete bounded list of one resource kind. Pagination shares one
    /// Kubernetes list RV; failures never return a partial inventory. Separate
    /// calls for Pods, PVs and attachments are not an atomic cluster snapshot.
    async fn recovery_objects(
        &self,
        base: Url,
        resource_kind: ObservationResourceKind,
        version: &str,
        namespaced: bool,
    ) -> Result<Vec<(Value, ObservationContext)>> {
        let context = ObservationContext::new(resource_kind, ObservationVerb::List);
        let kind = resource_kind.as_str();
        let mut current_context = context;
        let mut current_stage = ObservationStage::Request;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut collected = Vec::new();
            let mut continuation: Option<String> = None;
            let mut tokens = std::collections::HashSet::new();
            let mut identities = std::collections::HashSet::new();
            let mut resource_version: Option<String> = None;
            let list_kind = format!("{kind}List");
            for _ in 0..64 {
                let mut url = base.clone();
                url.query_pairs_mut().append_pair("limit", "100");
                if let Some(token) = &continuation {
                    url.query_pairs_mut().append_pair("continue", token);
                }
                current_context = context;
                current_stage = ObservationStage::Request;
                let response = self.observation_send(self.client.get(url), context).await?;
                current_context = context.check_status(&response)?;
                let context = current_context;
                current_stage = ObservationStage::DecodeResponse;
                let body = Self::observation_json(response, context).await?;
                if body.get("kind").and_then(Value::as_str) != Some(list_kind.as_str())
                    || body.get("apiVersion").and_then(Value::as_str) != Some(version)
                {
                    return Err(context.error(
                        ObservationStage::ValidateList,
                        ObservationErrorKind::InvalidResponse,
                    ));
                }
                let page_version = body
                    .pointer("/metadata/resourceVersion")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        context.error(
                            ObservationStage::ValidateList,
                            ObservationErrorKind::InvalidResponse,
                        )
                    })?;
                validate_identity_token(page_version).map_err(|_| {
                    context.error(
                        ObservationStage::ValidateList,
                        ObservationErrorKind::InvalidResponse,
                    )
                })?;
                if resource_version
                    .as_deref()
                    .is_some_and(|value| value != page_version)
                {
                    return Err(context.error(
                        ObservationStage::Pagination,
                        ObservationErrorKind::PaginationIncomplete,
                    ));
                }
                resource_version = Some(page_version.to_owned());
                let items = body.get("items").and_then(Value::as_array).ok_or_else(|| {
                    context.error(
                        ObservationStage::ValidateList,
                        ObservationErrorKind::InvalidResponse,
                    )
                })?;
                if items.len() > 100 {
                    return Err(context.error(
                        ObservationStage::ValidateList,
                        ObservationErrorKind::InventoryLimit,
                    ));
                }
                // Validate all page metadata before interpreting any list item.
                continuation = match body.pointer("/metadata/continue") {
                    None => None,
                    Some(Value::String(value)) if value.is_empty() => None,
                    Some(Value::String(value)) => Some(value.clone()),
                    Some(_) => {
                        return Err(context.error(
                            ObservationStage::Pagination,
                            ObservationErrorKind::PaginationIncomplete,
                        ));
                    }
                };
                let remaining = match body.pointer("/metadata/remainingItemCount") {
                    None => None,
                    Some(count) => Some(count.as_u64().ok_or_else(|| {
                        context.error(
                            ObservationStage::Pagination,
                            ObservationErrorKind::PaginationIncomplete,
                        )
                    })?),
                };
                if let Some(next) = &continuation {
                    if next.len() > 16 * 1024 {
                        return Err(context.error(
                            ObservationStage::Pagination,
                            ObservationErrorKind::InventoryLimit,
                        ));
                    }
                    if !tokens.insert(next.clone()) {
                        return Err(context.error(
                            ObservationStage::Pagination,
                            ObservationErrorKind::PaginationIncomplete,
                        ));
                    }
                } else if remaining.is_some_and(|count| count != 0) {
                    return Err(context.error(
                        ObservationStage::Pagination,
                        ObservationErrorKind::PaginationIncomplete,
                    ));
                }
                for object in items {
                    let normalized =
                        normalize_recovery_list_item(object, kind, version, namespaced, context)?;
                    let uid = text_at(&normalized, "/metadata/uid").ok_or_else(|| {
                        context.error(
                            ObservationStage::ValidateIdentity,
                            ObservationErrorKind::InvalidIdentity,
                        )
                    })?;
                    if !identities.insert(uid) {
                        return Err(context.error(
                            ObservationStage::Pagination,
                            ObservationErrorKind::PaginationIncomplete,
                        ));
                    }
                    collected.push((normalized, context));
                }
                if continuation.is_none() {
                    return Ok(collected);
                }
            }
            Err(current_context.error(
                ObservationStage::Pagination,
                ObservationErrorKind::InventoryLimit,
            ))
        })
        .await
        .map_err(|_| current_context.error(current_stage, ObservationErrorKind::Timeout))?
    }

    /// Discover original UID/owner candidates in the original namespace, and any
    /// Pod using the original namespaced claim, independently of labels. This
    /// returns candidates; ownership conflicts remain visible to the caller.
    pub async fn recovery_pods(
        &self,
        namespace: &str,
        original_pod_uid: Option<&str>,
        sandbox_uid: Option<&str>,
        claim_name: Option<&str>,
    ) -> Result<Vec<OwnedResourceObservation>> {
        validate_resource_segment(namespace)?;
        for uid in [original_pod_uid, sandbox_uid].into_iter().flatten() {
            validate_identity_token(uid)?;
        }
        if let Some(claim) = claim_name {
            validate_resource_segment(claim)?;
        }
        ensure!(
            original_pod_uid.is_some() || sandbox_uid.is_some() || claim_name.is_some(),
            "Kubernetes Pod recovery anchor missing"
        );
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/v1/namespaces/{namespace}/pods"));
        let mut matches = Vec::new();
        for (object, context) in self
            .recovery_objects(url, ObservationResourceKind::Pod, "v1", true)
            .await?
        {
            if !object.get("spec").is_some_and(Value::is_object) {
                return Err(context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                ));
            }
            let observed = OwnedResourceObservation::from_object(&object).ok_or_else(|| {
                context.error(
                    ObservationStage::ValidateIdentity,
                    ObservationErrorKind::InvalidIdentity,
                )
            })?;
            if observed.namespace.as_deref() != Some(namespace) {
                return Err(context.error(
                    ObservationStage::ValidateIdentity,
                    ObservationErrorKind::InvalidIdentity,
                ));
            }
            let volumes = match object.pointer("/spec/volumes") {
                None => None,
                Some(volumes) => Some(volumes.as_array().ok_or_else(|| {
                    context.error(
                        ObservationStage::ValidateItem,
                        ObservationErrorKind::InvalidResponse,
                    )
                })?),
            };
            let mut uses_claim = false;
            for volume in volumes.into_iter().flatten() {
                if !volume.is_object() {
                    return Err(context.error(
                        ObservationStage::ValidateItem,
                        ObservationErrorKind::InvalidResponse,
                    ));
                }
                if let Some(claim) = volume.get("persistentVolumeClaim") {
                    let name = claim
                        .get("claimName")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            context.error(
                                ObservationStage::ValidateIdentity,
                                ObservationErrorKind::InvalidIdentity,
                            )
                        })?;
                    validate_resource_segment(name).map_err(|_| {
                        context.error(
                            ObservationStage::ValidateIdentity,
                            ObservationErrorKind::InvalidIdentity,
                        )
                    })?;
                    uses_claim |= observed.namespace.as_deref() == Some(namespace)
                        && claim_name == Some(name);
                }
            }
            if original_pod_uid.is_some_and(|uid| observed.uid.as_deref() == Some(uid))
                || sandbox_uid
                    .is_some_and(|uid| observed.owner_uids.iter().any(|owner| owner == uid))
                || uses_claim
            {
                matches.push(observed);
            }
        }
        Ok(matches)
    }

    pub async fn claim_users(
        &self,
        namespace: &str,
        claim_name: &str,
    ) -> Result<Vec<OwnedResourceObservation>> {
        self.recovery_pods(namespace, None, None, Some(claim_name))
            .await
    }

    pub async fn node_lease(&self, name: &str) -> Result<Option<NodeLeaseObservation>> {
        validate_resource_segment(name)?;
        let mut url = self.base_url.clone();
        url.set_path(&format!(
            "/apis/coordination.k8s.io/v1/namespaces/kube-node-lease/leases/{name}"
        ));
        let Some(object) = self.read_recovery_optional_object(url).await? else {
            return Ok(None);
        };
        validate_recovery_object(&object, "Lease", "coordination.k8s.io/v1", true)?;
        let observation = NodeLeaseObservation::from_object(&object)?;
        ensure!(
            observation.name == name && observation.namespace == "kube-node-lease",
            "Kubernetes Node Lease identity conflict"
        );
        Ok(Some(observation))
    }

    /// Includes a same-name claim as a conflict candidate even when its original
    /// UID is unknown. Discovery alone never authorizes adoption or deletion.
    /// Empty candidates never prove CSI terminality or absence of cloud disks.
    pub async fn persistent_volumes_for_claim(
        &self,
        namespace: &str,
        name: &str,
        uid: Option<&str>,
    ) -> Result<Vec<PersistentVolumeObservation>> {
        validate_resource_segment(namespace)?;
        validate_resource_segment(name)?;
        if let Some(uid) = uid {
            validate_identity_token(uid)?;
        }
        let mut url = self.base_url.clone();
        url.set_path("/api/v1/persistentvolumes");
        self.recovery_objects(url, ObservationResourceKind::PersistentVolume, "v1", false)
            .await?
            .iter()
            .map(|(object, context)| {
                PersistentVolumeObservation::from_object(object).map_err(|_| {
                    context.error(
                        ObservationStage::ValidateItem,
                        ObservationErrorKind::InvalidResponse,
                    )
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(|rows| {
                rows.into_iter()
                    .filter(|row| {
                        uid.is_some_and(|uid| row.claim_uid.as_deref() == Some(uid))
                            || (row.claim_namespace.as_deref() == Some(namespace)
                                && row.claim_name.as_deref() == Some(name))
                    })
                    .collect()
            })
    }

    pub async fn volume_attachments(
        &self,
        pv_name: &str,
    ) -> Result<Vec<VolumeAttachmentObservation>> {
        validate_resource_segment(pv_name)?;
        let mut url = self.base_url.clone();
        url.set_path("/apis/storage.k8s.io/v1/volumeattachments");
        self.recovery_objects(
            url,
            ObservationResourceKind::VolumeAttachment,
            "storage.k8s.io/v1",
            false,
        )
        .await?
        .iter()
        .map(|(object, context)| {
            VolumeAttachmentObservation::from_object(object).map_err(|_| {
                context.error(
                    ObservationStage::ValidateItem,
                    ObservationErrorKind::InvalidResponse,
                )
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(|rows| {
            rows.into_iter()
                .filter(|row| row.persistent_volume_name.as_deref() == Some(pv_name))
                .collect()
        })
    }

    /// Observe controller-created Pods/PVCs by ownerReference. Cleanup callers
    /// consume this inventory, so every page and item must be valid before any
    /// ownership filtering. Invalid or incomplete data is never an empty list.
    /// This read does not establish runtime or storage termination.
    pub async fn list_owned_resources(
        &self,
        namespace: &str,
        resource: &str,
        sandbox_uid: &str,
    ) -> Result<Vec<OwnedResourceObservation>> {
        let kind = match resource {
            "pods" => ObservationResourceKind::Pod,
            "persistentvolumeclaims" => ObservationResourceKind::PersistentVolumeClaim,
            _ => return Err(anyhow!("unsupported observed resource")),
        };
        validate_resource_segment(namespace)?;
        validate_identity_token(sandbox_uid)?;
        let mut url = self.base_url.clone();
        url.set_path(&format!("/api/v1/namespaces/{namespace}/{resource}"));
        let rows = self
            .recovery_objects(url, kind, "v1", true)
            .await?
            .iter()
            .map(|(object, context)| {
                if object
                    .pointer("/metadata/namespace")
                    .and_then(Value::as_str)
                    != Some(namespace)
                {
                    return Err(context.error(
                        ObservationStage::ValidateIdentity,
                        ObservationErrorKind::InvalidIdentity,
                    ));
                }
                OwnedResourceObservation::from_object(object).ok_or_else(|| {
                    context.error(
                        ObservationStage::ValidateItem,
                        ObservationErrorKind::InvalidResponse,
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter(|item| item.owner_uids.iter().any(|uid| uid == sandbox_uid))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path, query_param},
    };

    fn observation_error(error: &anyhow::Error) -> &SandboxObservationError {
        let safe = error
            .downcast_ref::<SandboxObservationError>()
            .expect("observation failures retain a fixed diagnostic");
        let rendered = format!("{error:#} {error:?} {safe} {safe:?}");
        for secret in [
            "fixture-secret-token",
            "fixture-writer-fence",
            "?token=",
            "http://",
            "https://",
        ] {
            assert!(
                !rendered.contains(secret),
                "diagnostic leaked a protected fixture value"
            );
        }
        safe
    }

    #[tokio::test]
    async fn typed_lists_reject_outer_type_and_malformed_identity_after_normalization() {
        for kind in [
            ObservationResourceKind::Pod,
            ObservationResourceKind::PersistentVolumeClaim,
            ObservationResourceKind::PersistentVolume,
            ObservationResourceKind::VolumeAttachment,
        ] {
            let version = if kind == ObservationResourceKind::VolumeAttachment {
                "storage.k8s.io/v1"
            } else {
                "v1"
            };
            let namespaced = matches!(
                kind,
                ObservationResourceKind::Pod | ObservationResourceKind::PersistentVolumeClaim
            );
            for defect in [
                "outer-kind",
                "outer-version",
                "outer-version-missing",
                "missing-uid",
                "invalid-owner",
                "invalid-claim-ref",
            ] {
                if defect == "invalid-claim-ref"
                    && kind != ObservationResourceKind::PersistentVolume
                {
                    continue;
                }
                let server = MockServer::start().await;
                let mut item = recovery_owned_resource(kind.as_str(), "original", "uid", "owner");
                item.as_object_mut().unwrap().remove("kind");
                item.as_object_mut().unwrap().remove("apiVersion");
                let mut list = json!({"kind":format!("{}List", kind.as_str()),"apiVersion":version,
                    "metadata":{"resourceVersion":"40"},"items":[item]});
                match defect {
                    "outer-kind" => list["kind"] = json!("WrongList"),
                    "outer-version" => list["apiVersion"] = json!("wrong/v1"),
                    "outer-version-missing" => {
                        list.as_object_mut().unwrap().remove("apiVersion");
                    }
                    "missing-uid" => {
                        list["items"][0]["metadata"]
                            .as_object_mut()
                            .unwrap()
                            .remove("uid");
                    }
                    "invalid-owner" => {
                        list["items"][0]["metadata"]["ownerReferences"][0]["uid"] = json!(null)
                    }
                    "invalid-claim-ref" => {
                        list["items"][0]["spec"] =
                            json!({"claimRef":{"name":"claim", "namespace":42}})
                    }
                    _ => unreachable!(),
                }
                Mock::given(method("GET"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(list))
                    .mount(&server)
                    .await;
                let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
                let error = api
                    .recovery_objects(server.uri().parse().unwrap(), kind, version, namespaced)
                    .await
                    .err()
                    .unwrap();
                let safe = observation_error(&error);
                assert_eq!(safe.resource_kind, kind);
                if defect.starts_with("outer-") {
                    assert_eq!(safe.stage, ObservationStage::ValidateList);
                    assert_eq!(safe.error_kind, ObservationErrorKind::InvalidResponse);
                } else {
                    assert_eq!(safe.stage, ObservationStage::ValidateIdentity);
                    assert_eq!(safe.error_kind, ObservationErrorKind::InvalidIdentity);
                }
            }
        }
    }

    #[tokio::test]
    async fn observation_limits_and_pagination_have_distinct_fixed_diagnostics() {
        let pod = recovery_pod("original", "uid", "nyxid-machine");
        for (response, expected) in [
            (ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","continue":"loop"},"items":[]})), ObservationErrorKind::PaginationIncomplete),
            (ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},"items":[pod.clone(),pod.clone()]})), ObservationErrorKind::PaginationIncomplete),
            (ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","remainingItemCount":1},"items":[]})), ObservationErrorKind::PaginationIncomplete),
            (ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","continue":"x".repeat(16 * 1024 + 1)},"items":[]})), ObservationErrorKind::InventoryLimit),
            (ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},"items":vec![pod;101]})), ObservationErrorKind::InventoryLimit),
            (ResponseTemplate::new(200).set_body_raw("x".repeat(1024*1024+1),"application/json"), ObservationErrorKind::InventoryLimit),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET")).respond_with(response).mount(&server).await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let error = api.claim_users("nyxid-machine", "claim").await.unwrap_err();
            assert_eq!(observation_error(&error).error_kind, expected);
            assert_eq!(observation_error(&error).http_status, Some(200));
        }
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(|request: &wiremock::Request| {
                let page = request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "continue")
                    .map(|(_, value)| value.parse::<u32>().unwrap())
                    .unwrap_or(0);
                ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1",
                "metadata":{"resourceVersion":"40","continue":(page + 1).to_string()},"items":[]}))
            })
            .expect(64)
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let error = api.claim_users("nyxid-machine", "claim").await.unwrap_err();
        assert_eq!(
            observation_error(&error).error_kind,
            ObservationErrorKind::InventoryLimit
        );
        assert_eq!(
            observation_error(&error).stage,
            ObservationStage::Pagination
        );
    }

    #[tokio::test]
    async fn typed_list_type_metadata_matrix_preserves_kind_and_identity() {
        for (kind, version, namespaced) in [
            (ObservationResourceKind::Pod, "v1", true),
            (ObservationResourceKind::PersistentVolumeClaim, "v1", true),
            (ObservationResourceKind::PersistentVolume, "v1", false),
            (
                ObservationResourceKind::VolumeAttachment,
                "storage.k8s.io/v1",
                false,
            ),
        ] {
            for omitted in [
                vec![],
                vec!["kind"],
                vec!["apiVersion"],
                vec!["kind", "apiVersion"],
            ] {
                let server = MockServer::start().await;
                let mut item = recovery_owned_resource(
                    kind.as_str(),
                    "original",
                    "original-uid",
                    "sandbox-uid",
                );
                item["apiVersion"] = json!(version);
                if !namespaced {
                    item["metadata"]
                        .as_object_mut()
                        .unwrap()
                        .remove("namespace");
                }
                for field in &omitted {
                    item.as_object_mut().unwrap().remove(*field);
                }
                Mock::given(method("GET"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                        "kind":format!("{}List", kind.as_str()),"apiVersion":version,
                        "metadata":{"resourceVersion":"40"},"items":[item]
                    })))
                    .mount(&server)
                    .await;
                let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
                let rows = api
                    .recovery_objects(server.uri().parse().unwrap(), kind, version, namespaced)
                    .await
                    .unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].0["kind"], kind.as_str());
                assert_eq!(rows[0].0["apiVersion"], version);
                assert_eq!(rows[0].0["metadata"]["uid"], "original-uid");
            }
            for field in ["kind", "apiVersion"] {
                for malformed in [
                    Value::Null,
                    json!(42),
                    json!(true),
                    json!(""),
                    json!("WrongType"),
                ] {
                    let server = MockServer::start().await;
                    let mut item = recovery_owned_resource(
                        kind.as_str(),
                        "original",
                        "original-uid",
                        "sandbox-uid",
                    );
                    item["apiVersion"] = json!(version);
                    item[field] = malformed;
                    Mock::given(method("GET"))
                        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                            "kind":format!("{}List", kind.as_str()),"apiVersion":version,
                            "metadata":{"resourceVersion":"40"},"items":[item]
                        })))
                        .mount(&server)
                        .await;
                    let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
                    let error = api
                        .recovery_objects(server.uri().parse().unwrap(), kind, version, namespaced)
                        .await
                        .err()
                        .unwrap();
                    let safe = observation_error(&error);
                    assert_eq!(safe.error_kind, ObservationErrorKind::InvalidResponse);
                    assert_eq!(safe.stage, ObservationStage::ValidateItem);
                    assert_eq!(safe.resource_kind, kind);
                    assert_eq!(safe.http_status, Some(200));
                }
            }
        }
    }

    #[tokio::test]
    async fn recovery_pods_rejects_unrelated_namespace_before_filtering() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/nyxid-machine/pods"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},
                "items":[recovery_pod("unrelated", "foreign-uid", "other-namespace")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let error = api.claim_users("nyxid-machine", "claim").await.unwrap_err();
        let safe = observation_error(&error);
        assert_eq!(safe.stage, ObservationStage::ValidateIdentity);
        assert_eq!(safe.error_kind, ObservationErrorKind::InvalidIdentity);
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.url.path() != "/api/v1/pods")
        );
    }

    #[tokio::test]
    async fn observation_status_matrix_covers_delete_reads_lists_and_effects() {
        for (status, expected) in [
            (401, ObservationErrorKind::Authentication),
            (403, ObservationErrorKind::Forbidden),
            (500, ObservationErrorKind::HttpStatus),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET")).respond_with(ResponseTemplate::new(status)
                .set_body_string("fixture-secret-token fixture-writer-fence https://sensitive.invalid/?token=fixture-secret-token"))
                .mount(&server).await;
            Mock::given(method("DELETE"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let errors = [
                (
                    api.get("nyxid-machine", "original").await.unwrap_err(),
                    ObservationResourceKind::Sandbox,
                    ObservationVerb::Get,
                ),
                (
                    api.resource("nyxid-machine", "pods", "original")
                        .await
                        .unwrap_err(),
                    ObservationResourceKind::Pod,
                    ObservationVerb::Get,
                ),
                (
                    api.resource("nyxid-machine", "persistentvolumeclaims", "claim")
                        .await
                        .unwrap_err(),
                    ObservationResourceKind::PersistentVolumeClaim,
                    ObservationVerb::Get,
                ),
                (
                    api.persistent_volume("pv").await.unwrap_err(),
                    ObservationResourceKind::PersistentVolume,
                    ObservationVerb::Get,
                ),
                (
                    api.claim_users("nyxid-machine", "claim").await.unwrap_err(),
                    ObservationResourceKind::Pod,
                    ObservationVerb::List,
                ),
                (
                    api.list_owned_resources(
                        "nyxid-machine",
                        "persistentvolumeclaims",
                        "sandbox-uid",
                    )
                    .await
                    .unwrap_err(),
                    ObservationResourceKind::PersistentVolumeClaim,
                    ObservationVerb::List,
                ),
                (
                    api.persistent_volumes_for_claim("nyxid-machine", "claim", None)
                        .await
                        .unwrap_err(),
                    ObservationResourceKind::PersistentVolume,
                    ObservationVerb::List,
                ),
                (
                    api.volume_attachments("pv").await.unwrap_err(),
                    ObservationResourceKind::VolumeAttachment,
                    ObservationVerb::List,
                ),
                (
                    api.delete_if_identity("nyxid-machine", "original", "sandbox-uid", "1")
                        .await
                        .unwrap_err(),
                    ObservationResourceKind::Sandbox,
                    ObservationVerb::Delete,
                ),
                (
                    api.delete_resource_if_identity(
                        "nyxid-machine",
                        "pods",
                        "original",
                        "pod-uid",
                        "1",
                    )
                    .await
                    .unwrap_err(),
                    ObservationResourceKind::Pod,
                    ObservationVerb::Delete,
                ),
                (
                    api.delete_resource_if_identity(
                        "nyxid-machine",
                        "persistentvolumeclaims",
                        "claim",
                        "pvc-uid",
                        "1",
                    )
                    .await
                    .unwrap_err(),
                    ObservationResourceKind::PersistentVolumeClaim,
                    ObservationVerb::Delete,
                ),
            ];
            for (error, resource_kind, verb) in errors {
                let safe = observation_error(&error);
                assert_eq!(safe.error_kind, expected);
                assert_eq!(safe.stage, ObservationStage::Request);
                assert_eq!(safe.resource_kind, resource_kind);
                assert_eq!(safe.verb, verb);
                assert_eq!(safe.http_status, Some(status));
            }
        }
    }

    #[tokio::test]
    async fn observation_patch_status_is_safe_and_keeps_request_context() {
        for (status, expected) in [
            (401, ObservationErrorKind::Authentication),
            (403, ObservationErrorKind::Forbidden),
            (503, ObservationErrorKind::HttpStatus),
        ] {
            let server = MockServer::start().await;
            let desired = intent(&server);
            let live = object(&desired, "original-uid");
            let resource_version = live["metadata"]["resourceVersion"]
                .as_str()
                .unwrap()
                .to_owned();
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(live))
                .mount(&server)
                .await;
            Mock::given(method("PATCH"))
                .respond_with(ResponseTemplate::new(status).set_body_string(
                    "fixture-secret-token fixture-writer-fence https://private/?token=secret",
                ))
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let error = api
                .suspend_if_identity(&desired, "original-uid", &resource_version)
                .await
                .unwrap_err();
            let safe = observation_error(&error);
            assert_eq!(safe.error_kind, expected);
            assert_eq!(safe.resource_kind, ObservationResourceKind::Sandbox);
            assert_eq!(safe.verb, ObservationVerb::Patch);
            assert_eq!(safe.http_status, Some(status));
        }
    }

    #[tokio::test]
    async fn observation_404_is_absence_only_for_exact_reads_and_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(
            api.get("nyxid-machine", "original")
                .await
                .unwrap()
                .is_none()
        );
        assert!(api.persistent_volume("original").await.unwrap().is_none());
        assert!(
            !api.delete_if_identity("nyxid-machine", "original", "uid", "1")
                .await
                .unwrap()
        );
        for resource in ["pods", "persistentvolumeclaims"] {
            assert!(
                api.resource("nyxid-machine", resource, "original")
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                !api.delete_resource_if_identity("nyxid-machine", resource, "original", "uid", "1")
                    .await
                    .unwrap()
            );
            let error = api
                .list_owned_resources("nyxid-machine", resource, "uid")
                .await
                .unwrap_err();
            assert_eq!(
                observation_error(&error).error_kind,
                ObservationErrorKind::HttpStatus
            );
            assert_eq!(observation_error(&error).http_status, Some(404));
        }
        for error in [
            api.persistent_volumes_for_claim("nyxid-machine", "claim", None)
                .await
                .unwrap_err(),
            api.volume_attachments("pv").await.unwrap_err(),
        ] {
            assert_eq!(
                observation_error(&error).error_kind,
                ObservationErrorKind::HttpStatus
            );
            assert_eq!(observation_error(&error).http_status, Some(404));
        }
    }

    #[tokio::test]
    async fn observation_decode_credentials_transport_and_request_timeout_are_safe() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{fixture-secret-token fixture-writer-fence https://private/?token=secret",
            ))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        for error in [
            api.get("nyxid-machine", "original").await.unwrap_err(),
            api.claim_users("nyxid-machine", "claim").await.unwrap_err(),
        ] {
            assert_eq!(
                observation_error(&error).stage,
                ObservationStage::DecodeResponse
            );
            assert_eq!(
                observation_error(&error).error_kind,
                ObservationErrorKind::InvalidResponse
            );
            assert_eq!(observation_error(&error).http_status, Some(200));
        }
        let temporary = tempfile::tempdir().unwrap();
        let api = SandboxApi::with_token_file_client(
            server.uri().parse().unwrap(),
            temporary.path().join("missing-token"),
            Client::new(),
        )
        .unwrap();
        let error = api.claim_users("nyxid-machine", "claim").await.unwrap_err();
        assert_eq!(
            observation_error(&error).error_kind,
            ObservationErrorKind::CredentialUnavailable
        );
        assert_eq!(observation_error(&error).http_status, None);
        let delayed = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(200)),
            )
            .mount(&delayed)
            .await;
        let client = Client::builder()
            .timeout(std::time::Duration::from_millis(20))
            .build()
            .unwrap();
        let api = SandboxApi::with_client(delayed.uri().parse().unwrap(), None, client).unwrap();
        let error = api.claim_users("nyxid-machine", "claim").await.unwrap_err();
        assert_eq!(
            observation_error(&error).error_kind,
            ObservationErrorKind::Timeout
        );
        assert_eq!(observation_error(&error).http_status, None);
        let unreachable = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = unreachable.local_addr().unwrap();
        drop(unreachable);
        let api = SandboxApi::new(format!("http://{address}").parse().unwrap(), None).unwrap();
        let error = api.get("nyxid-machine", "original").await.unwrap_err();
        assert_eq!(
            observation_error(&error).error_kind,
            ObservationErrorKind::Transport
        );
        assert_eq!(observation_error(&error).http_status, None);
    }

    #[tokio::test]
    async fn observation_later_page_failure_returns_no_partial_inventory() {
        for later in [
            ResponseTemplate::new(403),
            ResponseTemplate::new(200).set_body_string("invalid-json"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","continue":"next"},
                "items":[recovery_pod("original", "uid", "nyxid-machine")]
            }))).mount(&server).await;
            Mock::given(method("GET"))
                .and(query_param("continue", "next"))
                .respond_with(later)
                .with_priority(1)
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let error = api
                .recovery_pods("nyxid-machine", Some("uid"), None, None)
                .await
                .unwrap_err();
            assert!(matches!(
                observation_error(&error).error_kind,
                ObservationErrorKind::Forbidden | ObservationErrorKind::InvalidResponse
            ));
        }
    }

    #[test]
    fn single_object_recovery_type_metadata_remains_strict() {
        for (kind, version) in [
            ("Sandbox", "agents.x-k8s.io/v1beta1"),
            ("Lease", "coordination.k8s.io/v1"),
        ] {
            let mut item = recovery_owned_resource(kind, "original", "uid", "owner");
            item["apiVersion"] = json!(version);
            validate_recovery_object(&item, kind, version, true).unwrap();
            for field in ["kind", "apiVersion"] {
                let mut omitted = item.clone();
                omitted.as_object_mut().unwrap().remove(field);
                assert!(validate_recovery_object(&omitted, kind, version, true).is_err());
                for malformed in [Value::Null, json!(42), json!(""), json!("WrongType")] {
                    let mut wrong = item.clone();
                    wrong[field] = malformed;
                    assert!(validate_recovery_object(&wrong, kind, version, true).is_err());
                }
            }
        }
    }

    #[tokio::test]
    async fn delete_recovery_suspend_preserves_templates_and_uses_uid_rv_tests() {
        let server = MockServer::start().await;
        let mut desired = intent(&server);
        desired.running = false;
        // Deletion must not depend on today's image/profile admission settings.
        desired.image = "deployment-image-is-currently-unavailable".into();
        let mut live = object(&desired, "original-uid");
        live["metadata"]["labels"]["nyxid.dev/operation-id"] = json!("old-operation");
        live["spec"]["shutdownTime"] = json!("2026-10-17T01:00:00Z");
        live["spec"]["podTemplate"]["metadata"]["annotations"] = json!({"retained":"original"});
        let response = live.clone();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(live))
            .mount(&server)
            .await;
        let tuple = desired.clone();
        Mock::given(method("PATCH"))
            .and(header("authorization", "Bearer test-token"))
            .and(header("content-type", "application/json-patch+json"))
            .respond_with(move |request: &wiremock::Request| {
                let patch: Value = serde_json::from_slice(&request.body).unwrap();
                let operations = patch.as_array().unwrap();
                assert!(operations.contains(&json!({"op":"test","path":"/metadata/uid","value":"original-uid"})));
                assert!(operations.contains(&json!({"op":"test","path":"/metadata/resourceVersion","value":"7"})));
                assert!(operations.contains(&json!({"op":"add","path":"/metadata/labels/nyxid.dev~1operation-id","value":tuple.operation_id})));
                assert!(operations.contains(&json!({"op":"add","path":"/spec/operatingMode","value":"Suspended"})));
                assert!(operations.contains(&json!({"op":"remove","path":"/spec/shutdownTime"})));
                assert!(operations.iter().all(|operation| !operation["path"].as_str().unwrap().contains("Template")));
                let mut response = response.clone();
                response["spec"]["operatingMode"] = json!("Suspended");
                response["metadata"]["labels"]["nyxid.dev/operation-id"] = json!(tuple.operation_id);
                ResponseTemplate::new(200).set_body_json(response)
            })
            .expect(1).mount(&server).await;
        let api =
            SandboxApi::new(server.uri().parse().unwrap(), Some("test-token".into())).unwrap();
        assert!(
            api.suspend_if_identity(&desired, "original-uid", "7")
                .await
                .unwrap()
                .desired_suspended
        );
    }

    #[tokio::test]
    async fn delete_recovery_suspend_rejects_replacement_stale_version_and_tuple() {
        for drift in [
            "uid",
            "resourceVersion",
            "writer-fence",
            "machine-id",
            "owner-id",
            "runtime-generation",
            "credential-epoch",
        ] {
            let server = MockServer::start().await;
            let desired = intent(&server);
            let mut live = object(&desired, "original-uid");
            if matches!(drift, "uid" | "resourceVersion") {
                live["metadata"][drift] = json!("replacement");
            } else {
                live["metadata"]["labels"][format!("nyxid.dev/{drift}")] = json!("replacement");
            }
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(live))
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            assert!(
                api.suspend_if_identity(&desired, "original-uid", "7")
                    .await
                    .is_err(),
                "accepted {drift}"
            );
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method.as_str() == "GET")
            );
        }
    }

    #[tokio::test]
    async fn delete_recovery_exact_deletes_send_both_preconditions_and_keep_404_local() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(header("authorization", "Bearer test-token"))
            .and(|request: &wiremock::Request| {
                let mut expected = json!({"preconditions": {
                    "uid": "original-uid", "resourceVersion": "7"
                }});
                if request.url.path().contains("/sandboxes/") {
                    expected["propagationPolicy"] = json!("Foreground");
                }
                request.body_json::<Value>().ok().as_ref() == Some(&expected)
            })
            .respond_with(|request: &wiremock::Request| {
                if request.url.path().ends_with("/absent") {
                    ResponseTemplate::new(404)
                } else {
                    ResponseTemplate::new(202)
                }
            })
            .expect(3)
            .mount(&server)
            .await;
        let api =
            SandboxApi::new(server.uri().parse().unwrap(), Some("test-token".into())).unwrap();
        assert!(
            api.delete_if_identity("nyxid-machine", "original", "original-uid", "7")
                .await
                .unwrap()
        );
        assert!(
            api.delete_resource_if_identity(
                "nyxid-machine",
                "pods",
                "original",
                "original-uid",
                "7"
            )
            .await
            .unwrap()
        );
        assert!(
            !api.delete_resource_if_identity(
                "nyxid-machine",
                "persistentvolumeclaims",
                "absent",
                "original-uid",
                "7"
            )
            .await
            .unwrap()
        );
        assert!(
            api.delete_resource_if_identity(
                "nyxid-machine",
                "secrets",
                "original",
                "original-uid",
                "7"
            )
            .await
            .is_err()
        );
        assert!(
            api.delete_resource_if_identity("nyxid-machine", "pods", "original", "", "7")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn sandbox_foreground_delete_preserves_status_and_never_cleans_children() {
        for status in [202, 404, 409] {
            let server = MockServer::start().await;
            Mock::given(method("DELETE"))
                .and(path(
                    "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/original",
                ))
                .and(body_json(json!({
                    "preconditions": {"uid": "original-uid", "resourceVersion": "7"},
                    "propagationPolicy": "Foreground"
                })))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let result = api
                .delete_if_identity("nyxid-machine", "original", "original-uid", "7")
                .await;
            match status {
                202 => assert!(result.unwrap()),
                404 => assert!(!result.unwrap()),
                409 => {
                    let error = result.unwrap_err();
                    let diagnostic = observation_error(&error);
                    assert_eq!(diagnostic.error_kind, ObservationErrorKind::HttpStatus);
                }
                _ => unreachable!(),
            }
            // DELETE acceptance/absence/conflict never triggers child deletes,
            // storage inventories, or an unconditional retry against a new UID.
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }

        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(
                ResponseTemplate::new(202).set_delay(std::time::Duration::from_millis(100)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = Client::builder()
            .timeout(std::time::Duration::from_millis(20))
            .build()
            .unwrap();
        let api = SandboxApi::with_client(server.uri().parse().unwrap(), None, client).unwrap();
        let error = api
            .delete_if_identity("nyxid-machine", "original", "original-uid", "7")
            .await
            .unwrap_err();
        assert_eq!(
            observation_error(&error).error_kind,
            ObservationErrorKind::Timeout
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_recovery_rejects_dot_path_segments_before_request() {
        let server = MockServer::start().await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(
            api.delete_resource_if_identity("nyxid-machine", "pods", "..", "uid", "7")
                .await
                .is_err()
        );
        assert!(
            api.delete_if_identity("..", "original", "uid", "7")
                .await
                .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    fn recovery_pod(name: &str, uid: &str, namespace: &str) -> Value {
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"uid":uid,"namespace":namespace,"resourceVersion":"3"},"spec":{"containers":[{"name":"node"}],"volumes":[]}})
    }

    #[tokio::test]
    async fn delete_recovery_pod_search_is_complete_without_labels_and_claims_are_namespaced() {
        let server = MockServer::start().await;
        let original = recovery_pod("original", "old-pod", "nyxid-machine");
        let mut owned = recovery_pod("owned", "owned-pod", "nyxid-machine");
        owned["metadata"]["ownerReferences"] = json!([{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","name":"original","uid":"sandbox-uid","controller":true}]);
        let mut user = recovery_pod("claim-user", "user-pod", "nyxid-machine");
        // A non-workspace volume still counts as a writer using the claim.
        user["spec"]["volumes"] =
            json!([{"name":"other","persistentVolumeClaim":{"claimName":"original-claim"}}]);
        Mock::given(method("GET"))
            .and(path("/api/v1/pods"))
            .respond_with(ResponseTemplate::new(403))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/api/v1/namespaces/nyxid-machine/pods"))
            .and(query_param("limit","100"))
            .respond_with(move |request:&wiremock::Request| {
                assert!(!request.url.query_pairs().any(|(key,_)| key=="labelSelector"));
                if request.url.query_pairs().any(|(key,value)|key=="continue"&&value=="next") {
                    ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},"items":[user.clone()]}))
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","continue":"next"},"items":[original.clone(),owned.clone()]}))
                }
            }).expect(4).mount(&server).await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let rows = api
            .recovery_pods(
                "nyxid-machine",
                Some("old-pod"),
                Some("sandbox-uid"),
                Some("original-claim"),
            )
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["original", "owned", "claim-user"]
        );
        let users = api
            .claim_users("nyxid-machine", "original-claim")
            .await
            .unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "claim-user");
    }

    #[tokio::test]
    async fn delete_recovery_pods_and_claim_users_reject_malformed_foreign_owners_before_filtering()
    {
        let mut malformed_owners = Vec::new();
        let valid = json!({"apiVersion":"apps/v1","kind":"ReplicaSet","name":"foreign","uid":"foreign-uid","controller":true,"blockOwnerDeletion":false});
        for field in ["apiVersion", "kind", "name", "uid"] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(field);
            malformed_owners.push((format!("missing-{field}"), missing));
            let mut wrong_type = valid.clone();
            wrong_type[field] = json!(23);
            malformed_owners.push((format!("nonstring-{field}"), wrong_type));
            let mut empty = valid.clone();
            empty[field] = json!("");
            malformed_owners.push((format!("empty-{field}"), empty));
        }
        for field in ["controller", "blockOwnerDeletion"] {
            let mut owner = valid.clone();
            owner[field] = json!("true");
            malformed_owners.push((format!("nonboolean-{field}"), owner));
        }
        for (defect, owner) in malformed_owners {
            let server = MockServer::start().await;
            let mut unrelated = recovery_pod("foreign-pod", "foreign-pod-uid", "nyxid-machine");
            unrelated["metadata"]["ownerReferences"] = json!([owner]);
            Mock::given(method("GET"))
                .and(path("/api/v1/namespaces/nyxid-machine/pods"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "apiVersion":"v1","kind":"PodList",
                    "metadata":{"resourceVersion":"complete"},"items":[unrelated]
                })))
                .expect(2)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let recovery = api
                .recovery_pods(
                    "nyxid-machine",
                    Some("original-pod"),
                    Some("sandbox-uid"),
                    Some("claim"),
                )
                .await;
            let users = api.claim_users("nyxid-machine", "claim").await;
            assert!(
                recovery.is_err(),
                "recovery_pods silently filtered {defect}"
            );
            assert!(users.is_err(), "claim_users silently filtered {defect}");
        }
    }

    #[tokio::test]
    async fn delete_recovery_inventory_rejects_incomplete_malformed_and_changed_pages() {
        for body in [
            json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","continue":123},"items":[]}),
            json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},"items":[{"kind":"Pod","metadata":{"uid":"unknown"}}]}),
            json!({"kind":"PodList","apiVersion":"v1","metadata":{},"items":[]}),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            assert!(api.claim_users("nyxid-machine", "claim").await.is_err());
        }
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(|request:&wiremock::Request| {
            if request.url.query_pairs().any(|(key,_)|key=="continue") {
                ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"41"},"items":[]}))
            } else {ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","continue":"next"},"items":[]}))}
        }).mount(&server).await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.claim_users("nyxid-machine", "claim").await.is_err());
    }

    #[tokio::test]
    async fn delete_recovery_inventory_refuses_forbidden_looping_oversized_and_duplicate_pages() {
        let pod = recovery_pod("original", "uid", "nyxid-machine");
        for response in [
            ResponseTemplate::new(403),
            ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40","continue":"loop"},"items":[]})),
            ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},"items":vec![pod.clone();101]})),
            ResponseTemplate::new(200).set_body_json(json!({"kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},"items":[pod.clone(),pod.clone()]})),
            ResponseTemplate::new(200).set_body_raw("x".repeat(1024*1024+1),"application/json"),
        ] {
            let server=MockServer::start().await;
            Mock::given(method("GET")).respond_with(response).mount(&server).await;
            let api=SandboxApi::new(server.uri().parse().unwrap(),None).unwrap();
            assert!(api.claim_users("nyxid-machine","claim").await.is_err());
        }
    }

    #[tokio::test]
    async fn delete_recovery_inventory_rejects_remaining_items_without_continuation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "apiVersion":"v1","kind":"PodList",
                "metadata":{"resourceVersion":"40","remainingItemCount":1},"items":[]
            })))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.claim_users("nyxid-machine", "claim").await.is_err());
    }

    #[tokio::test]
    async fn delete_recovery_inventory_has_an_aggregate_deadline() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(10)))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let start = std::time::Instant::now();
        let error = api.claim_users("nyxid-machine", "claim").await.unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<SandboxObservationError>()
                .unwrap()
                .error_kind,
            ObservationErrorKind::Timeout
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(7));
    }

    #[tokio::test]
    async fn delete_recovery_exact_reads_distinguish_absence_from_forbidden_and_malformed() {
        for status in [404, 403] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let result = api.node_lease("original-node").await;
            if status == 404 {
                assert!(result.unwrap().is_none());
            } else {
                assert!(result.is_err());
            }
        }
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"metadata":{"name":"replacement"}})),
            )
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.node_lease("original-node").await.is_err());
    }

    #[tokio::test]
    async fn delete_recovery_reads_original_lease_pv_candidates_csi_and_attachments() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/apis/coordination.k8s.io/v1/namespaces/kube-node-lease/leases/original-node"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"apiVersion":"coordination.k8s.io/v1","kind":"Lease","metadata":{"name":"original-node","namespace":"kube-node-lease","uid":"lease-uid","resourceVersion":"8","ownerReferences":[{"apiVersion":"v1","kind":"Node","name":"original-node","uid":"node-uid"}]},"spec":{"holderIdentity":"original-node","renewTime":"2026-10-17T01:00:00Z","leaseDurationSeconds":40}})))
            .mount(&server).await;
        let original = json!({"apiVersion":"v1","kind":"PersistentVolume","metadata":{"name":"original-pv","uid":"pv-uid","resourceVersion":"9"},"spec":{"claimRef":{"namespace":"nyxid-machine","name":"claim","uid":"claim-uid"},"csi":{"driver":"pd.csi.storage.gke.io","volumeHandle":"projects/project/zones/zone/disks/disk"},"persistentVolumeReclaimPolicy":"Delete"}});
        let mut conflict = original.clone();
        conflict["metadata"]["name"] = json!("conflicting-pv");
        conflict["metadata"]["uid"] = json!("other-pv-uid");
        conflict["spec"]["claimRef"]["uid"] = json!("replacement-claim");
        let mut uid_only = original.clone();
        uid_only["metadata"]["name"] = json!("uid-only-pv");
        uid_only["metadata"]["uid"] = json!("uid-only-pv-uid");
        uid_only["spec"]["claimRef"]["name"] = json!("different-claim");
        let mut unbound = original.clone();
        unbound["metadata"]["name"] = json!("unbound-pv");
        unbound["metadata"]["uid"] = json!("unbound-pv-uid");
        unbound["spec"].as_object_mut().unwrap().remove("claimRef");
        Mock::given(method("GET")).and(path("/api/v1/persistentvolumes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"apiVersion":"v1","kind":"PersistentVolumeList","metadata":{"resourceVersion":"10"},"items":[original,conflict,uid_only,unbound]})))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/apis/storage.k8s.io/v1/volumeattachments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"apiVersion":"storage.k8s.io/v1","kind":"VolumeAttachmentList","metadata":{"resourceVersion":"10"},"items":[{"apiVersion":"storage.k8s.io/v1","kind":"VolumeAttachment","metadata":{"name":"attachment","uid":"va-uid","resourceVersion":"10"},"spec":{"source":{"persistentVolumeName":"original-pv"},"nodeName":"original-node","attacher":"pd.csi.storage.gke.io"},"status":{"attached":false}}]})))
            .mount(&server).await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let lease = api.node_lease("original-node").await.unwrap().unwrap();
        assert_eq!(lease.owner_node_uid.as_deref(), Some("node-uid"));
        assert_eq!(lease.renew_time.as_deref(), Some("2026-10-17T01:00:00Z"));
        assert_eq!(lease.lease_duration_seconds, Some(40));
        let pvs = api
            .persistent_volumes_for_claim("nyxid-machine", "claim", Some("claim-uid"))
            .await
            .unwrap();
        assert_eq!(pvs.len(), 3);
        assert!(pvs.iter().any(|pv| pv.name == "uid-only-pv"));
        let named = api
            .persistent_volumes_for_claim("nyxid-machine", "claim", None)
            .await
            .unwrap();
        assert_eq!(named.len(), 2);
        assert!(
            named
                .iter()
                .all(|pv| pv.claim_name.as_deref() == Some("claim"))
        );
        assert!(
            api.persistent_volumes_for_claim("nyxid-machine", "claim", Some(""))
                .await
                .is_err()
        );
        assert_eq!(pvs[0].csi_driver.as_deref(), Some("pd.csi.storage.gke.io"));
        assert_eq!(
            pvs[0].csi_volume_handle.as_deref(),
            Some("projects/project/zones/zone/disks/disk")
        );
        let attachments = api.volume_attachments("original-pv").await.unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].attached, Some(false));
        assert_eq!(attachments[0].node_name.as_deref(), Some("original-node"));
    }

    #[tokio::test]
    async fn optional_runtime_class_is_omitted_and_explicit_class_is_preserved() {
        let server = MockServer::start().await;
        let mut desired = intent(&server);
        desired.runtime_class = None;
        desired.validate().unwrap();
        assert!(
            desired
                .apply_object()
                .pointer("/spec/podTemplate/spec/runtimeClassName")
                .is_none()
        );
        desired.runtime_class = Some("kata".into());
        desired.validate().unwrap();
        assert_eq!(
            desired.apply_object()["spec"]["podTemplate"]["spec"]["runtimeClassName"],
            "kata"
        );
        desired.runtime_class = Some(String::new());
        assert!(desired.validate().is_err());
    }

    #[test]
    fn suspension_requires_current_applied_condition_not_desired_mode() {
        let mut value = json!({
            "metadata": {"name":"machine", "namespace":"nyxid-machine", "uid":"sandbox-uid", "generation":9, "resourceVersion":"12"},
            "spec": {"operatingMode":"Suspended"},
            "status": {"observedGeneration":9, "conditions":[
                {"type":"Suspended", "status":"False", "observedGeneration":9, "reason":"PodTerminating"}
            ]}
        });
        assert!(!SandboxObservation::from_object(&value).unwrap().suspended);
        value["status"]["conditions"][0]["status"] = json!("True");
        value["status"]["conditions"][0]["observedGeneration"] = json!(8);
        assert!(!SandboxObservation::from_object(&value).unwrap().suspended);
        value["status"]["conditions"][0]["observedGeneration"] = json!(9);
        value["status"]["conditions"][0]["reason"] = json!("PodTerminated");
        let observation = SandboxObservation::from_object(&value).unwrap();
        assert!(observation.suspended);
        assert!(observation.desired_suspended);
        assert_eq!(observation.observed_generation, Some(9));
        assert_eq!(observation.generation, Some(9));
        value["status"]["conditions"][0]
            .as_object_mut()
            .unwrap()
            .remove("observedGeneration");
        assert!(!SandboxObservation::from_object(&value).unwrap().suspended);
        value["status"]["conditions"][0]["observedGeneration"] = json!(9);
        value["spec"]["operatingMode"] = json!("Running");
        assert!(!SandboxObservation::from_object(&value).unwrap().suspended);
    }

    #[test]
    fn ready_condition_must_observe_current_generation_and_not_duplicate() {
        let mut value = json!({"metadata":{"name":"machine", "generation":4},
            "spec":{"operatingMode":"Running"},
            "status":{"conditions":[{"type":"Ready","status":"True","observedGeneration":3}]}});
        assert!(!SandboxObservation::from_object(&value).unwrap().ready);
        value["status"]["conditions"][0]["observedGeneration"] = json!(4);
        assert!(SandboxObservation::from_object(&value).unwrap().ready);
        let duplicate = value["status"]["conditions"][0].clone();
        value["status"]["conditions"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(!SandboxObservation::from_object(&value).unwrap().ready);
    }

    #[tokio::test]
    async fn suspension_removes_expiry_so_upstream_can_refresh_suspended_condition() {
        let server = MockServer::start().await;
        let mut desired = intent(&server);
        assert!(desired.apply_object()["spec"].get("shutdownTime").is_some());
        desired.running = false;
        let suspended = desired.apply_object();
        assert_eq!(suspended["spec"]["operatingMode"], "Suspended");
        assert!(suspended["spec"].get("shutdownTime").is_none());
    }

    #[tokio::test]
    async fn workspace_template_is_stable_across_runtime_and_operation_changes() {
        let server = MockServer::start().await;
        {
            let original = intent(&server);
            let mut successor = original.clone();
            successor.runtime_generation += 1;
            successor.credential_epoch += 1;
            successor.operation_id = uuid::Uuid::new_v4().to_string();
            successor.writer_fence = uuid::Uuid::new_v4().to_string();
            for running in [false, true] {
                successor.running = running;
                successor.validate().unwrap();
                assert_eq!(
                    original.apply_object()["spec"]["volumeClaimTemplates"],
                    successor.apply_object()["spec"]["volumeClaimTemplates"]
                );
                assert_eq!(
                    original.apply_object()["spec"]["podTemplate"]["spec"]["volumes"],
                    successor.apply_object()["spec"]["podTemplate"]["spec"]["volumes"]
                );
                assert_ne!(
                    original.apply_object()["spec"]["podTemplate"],
                    successor.apply_object()["spec"]["podTemplate"]
                );
            }
        }
    }

    #[test]
    fn pod_observation_keeps_force_delete_ownership_and_termination_facts() {
        let mut pod = controlled_resource("Pod");
        pod["metadata"]["deletionGracePeriodSeconds"] = json!(0);
        pod["metadata"]["resourceVersion"] = json!("17");
        pod["metadata"]["finalizers"] = json!(["example.test/hold"]);
        pod["spec"] = json!({"nodeName":"worker-a", "containers":[{"name":"node"}],
            "volumes":[{"name":"workspace", "persistentVolumeClaim":{"claimName":"workspace-machine"}}]});
        pod["status"] = json!({"phase":"Failed", "containerStatuses":[{"name":"node","state":{"terminated":{"exitCode":137}}}]});
        let observed = OwnedResourceObservation::from_object(&pod).unwrap();
        assert!(observed.ownership_valid);
        assert!(observed.all_containers_terminated);
        assert_eq!(observed.deletion_grace_period_seconds, Some(0));
        assert_eq!(
            observed.workspace_claim_name.as_deref(),
            Some("workspace-machine")
        );
        assert_eq!(observed.node_name.as_deref(), Some("worker-a"));
        assert_eq!(observed.finalizers, ["example.test/hold"]);
        pod["metadata"]["ownerReferences"][0]["controller"] = json!(false);
        pod["status"]["containerStatuses"][0]["state"] = json!({"running":{}});
        let unknown = OwnedResourceObservation::from_object(&pod).unwrap();
        assert!(!unknown.ownership_valid);
        assert!(!unknown.all_containers_terminated);
    }

    #[test]
    fn node_observation_exposes_heartbeat_and_refuses_unreachable_or_ambiguous_ready() {
        let mut node = json!({"metadata":{"name":"worker-a","uid":"node-uid","resourceVersion":"11"},
            "status":{"nodeInfo":{"containerRuntimeVersion":"containerd://2.0.5"},
            "conditions":[{"type":"Ready","status":"True","lastHeartbeatTime":"2026-10-14T08:00:00Z"}]}});
        let healthy = NodeObservation::from_object(&node).unwrap();
        assert!(healthy.ready);
        assert_eq!(
            healthy.container_runtime_version.as_deref(),
            Some("containerd://2.0.5")
        );
        assert_eq!(
            healthy.ready_last_heartbeat_time.as_deref(),
            Some("2026-10-14T08:00:00Z")
        );
        node["spec"] =
            json!({"taints":[{"key":"node.kubernetes.io/unreachable","effect":"NoExecute"}]});
        assert!(!NodeObservation::from_object(&node).unwrap().ready);
        node.as_object_mut().unwrap().remove("spec");
        node["status"]["conditions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"Ready","status":"True"}));
        assert!(!NodeObservation::from_object(&node).unwrap().ready);
    }

    #[test]
    fn persistent_volume_observation_retains_claim_identity_policy_and_finalizers() {
        let volume = json!({"metadata":{"name":"pv-a","uid":"pv-uid","resourceVersion":"31",
            "deletionTimestamp":"2026-10-14T08:00:00Z", "finalizers":["external-provisioner.volume.kubernetes.io/finalizer"]},
            "spec":{"claimRef":{"uid":"claim-uid","name":"workspace-machine","namespace":"nyxid-machine"},
            "persistentVolumeReclaimPolicy":"Retain"},"status":{"phase":"Released"}});
        let observed = PersistentVolumeObservation::from_object(&volume).unwrap();
        assert_eq!(observed.claim_uid.as_deref(), Some("claim-uid"));
        assert_eq!(observed.reclaim_policy.as_deref(), Some("Retain"));
        assert_eq!(observed.phase.as_deref(), Some("Released"));
        assert_eq!(observed.finalizers.len(), 1);
        assert!(observed.deletion_timestamp.is_some());
    }

    #[tokio::test]
    async fn direct_native_resource_reads_distinguish_absence_denial_and_foreign_owner() {
        let server = MockServer::start().await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let mut foreign = controlled_resource("Pod");
        foreign["metadata"]["ownerReferences"][0]["kind"] = json!("AnotherController");
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/nyxid-machine/pods/foreign"))
            .respond_with(ResponseTemplate::new(200).set_body_json(foreign))
            .expect(1)
            .mount(&server)
            .await;
        let pod = api
            .resource("nyxid-machine", "pods", "foreign")
            .await
            .unwrap()
            .unwrap();
        assert!(!pod.ownership_valid);
        Mock::given(method("GET"))
            .and(path("/api/v1/nodes/denied"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        assert!(api.node("denied").await.is_err());
        Mock::given(method("GET"))
            .and(path("/api/v1/persistentvolumes/absent"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        assert!(api.persistent_volume("absent").await.unwrap().is_none());
        assert!(
            api.resource("nyxid-machine", "pods", "../escape")
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[test]
    fn workspace_inventory_accepts_stable_labels_without_a_runtime_or_operation() {
        let mut pvc = controlled_resource("PersistentVolumeClaim");
        let labels = pvc["metadata"]["labels"].as_object_mut().unwrap();
        for key in [
            "nyxid.dev/operation-id",
            "nyxid.dev/runtime-generation",
            "nyxid.dev/credential-epoch",
        ] {
            labels.remove(key);
        }
        let observed = ControlledResourceObservation::from_object(
            &pvc,
            "PersistentVolumeClaim",
            "nyxid-machine",
        );
        assert!(observed.metadata_valid);
        assert!(observed.operation_id.is_none());
        assert!(observed.runtime_generation.is_none());
        pvc["metadata"]["ownerReferences"][0]["controller"] = json!(false);
        assert!(
            !ControlledResourceObservation::from_object(
                &pvc,
                "PersistentVolumeClaim",
                "nyxid-machine"
            )
            .metadata_valid
        );
    }

    #[tokio::test]
    async fn projected_manager_token_rotates_and_invalid_replacements_fail_closed() {
        let server = MockServer::start().await;
        let directory = tempfile::tempdir().unwrap();
        let token_path = directory.path().join("manager-token");
        std::fs::write(&token_path, "first-projected-token").unwrap();
        let api = SandboxApi::with_token_file_client(
            server.uri().parse().unwrap(),
            token_path.clone(),
            Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        )
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/apis/agents.x-k8s.io/v1beta1"))
            .and(header("authorization", "Bearer first-projected-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        api.health().await.unwrap();
        let replacement = directory.path().join("replacement-token");
        std::fs::write(&replacement, "second-projected-token\n").unwrap();
        std::fs::rename(&replacement, &token_path).unwrap();
        Mock::given(method("GET"))
            .and(path("/apis/agents.x-k8s.io/v1beta1"))
            .and(header("authorization", "Bearer second-projected-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        api.health().await.unwrap();
        let rendered = format!("{api:?}");
        assert!(!rendered.contains("projected-token"));
        assert!(!rendered.contains(token_path.to_str().unwrap()));
        for invalid in [
            Vec::new(),
            vec![b'x'; 64 * 1024 + 1],
            b"secret\nheader".to_vec(),
        ] {
            std::fs::write(&token_path, invalid).unwrap();
            let error = api.health().await.unwrap_err().to_string();
            assert_eq!(error, "Kubernetes credential file unavailable or invalid");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&token_path, "unsafe-file-token").unwrap();
            std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o666)).unwrap();
            assert!(api.health().await.is_err());
        }
        std::fs::remove_file(&token_path).unwrap();
        assert!(api.health().await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[test]
    fn remote_cleartext_and_secret_bearing_api_urls_are_rejected() {
        assert!(
            SandboxApi::new(
                "http://cluster.example.test".parse().unwrap(),
                Some("test".into())
            )
            .is_err()
        );
        assert!(
            SandboxApi::new(
                "https://user:password@cluster.example.test"
                    .parse()
                    .unwrap(),
                None
            )
            .is_err()
        );
        assert!(
            SandboxApi::new(
                "https://cluster.example.test?token=test".parse().unwrap(),
                None
            )
            .is_err()
        );
        assert!(SandboxApi::new("http://[::1]:8000".parse().unwrap(), None).is_ok());
    }

    fn intent(_server: &MockServer) -> SandboxIntent {
        SandboxIntent {
            bootstrap_base_url: "https://nyxid.test".into(),
            namespace: "nyxid-machine".into(),
            name: "machine-123".into(),
            owner_id: "owner-123".into(),
            machine_id: "machine-123".into(),
            operation_id: "operation-123".into(),
            runtime_generation: 4,
            credential_epoch: 9,
            writer_fence: uuid::Uuid::new_v4().to_string(),
            profile: nyxid_machine::managed::PROFILE.into(),
            image: "ghcr.io/chronoaiproject/nyxid/nyxid-node-remote-machine@sha256:".to_string()
                + &"ab".repeat(32),
            runtime_class: Some("kata".into()),
            workspace_storage_class: Some("managed-csi".into()),
            workspace_size: "20Gi".into(),
            cpu_request: "2".into(),
            cpu_limit: "2".into(),
            memory_request: "2Gi".into(),
            memory_limit: "2Gi".into(),
            running: true,
            shutdown_time: Some("2026-10-14T00:00:00Z".into()),
        }
    }

    fn object(intent: &SandboxIntent, uid: &str) -> Value {
        json!({
            "apiVersion": format!("{API_GROUP}/{API_VERSION}"),
            "kind": KIND,
            "metadata": {
                "name": intent.name,
                "namespace": intent.namespace,
                "uid": uid,
                "resourceVersion": "7",
                "generation": 3,
                "labels": {
                    "nyxid.dev/machine-id": intent.machine_id,
                    "nyxid.dev/owner-id": intent.owner_id,
                    "nyxid.dev/operation-id": intent.operation_id,
                    "nyxid.dev/profile": intent.profile,
                    "nyxid.dev/runtime-generation": "4",
                    "nyxid.dev/credential-epoch": "9"
                    ,"nyxid.dev/writer-fence": intent.writer_fence
                }
            },
            "spec": {"operatingMode": "Running", "volumeClaimTemplates": intent.apply_object()["spec"]["volumeClaimTemplates"]},
            "status": {
                "phase": "Ready",
                "conditions": [{"type":"Ready","status":"True","observedGeneration":3}]
            }
        })
    }

    #[tokio::test]
    async fn current_temporary_storage_omits_all_limits_and_obsolete_profiles_reject() {
        let server = MockServer::start().await;
        let current = intent(&server);
        current.validate().unwrap();
        let body = current.apply_object();
        for path in [
            "/spec/podTemplate/spec/containers/0/resources/requests/ephemeral-storage",
            "/spec/podTemplate/spec/containers/0/resources/limits/ephemeral-storage",
            "/spec/podTemplate/spec/volumes/2/emptyDir/sizeLimit",
            "/spec/podTemplate/spec/volumes/3/emptyDir/sizeLimit",
        ] {
            assert!(
                body.pointer(path).is_none(),
                "current template must omit {path}"
            );
        }
        for name in ["requests", "limits"] {
            assert_eq!(
                body["spec"]["podTemplate"]["spec"]["containers"][0]["resources"][name],
                json!({"cpu":"2","memory":"2Gi"})
            );
        }
        assert_eq!(
            body["spec"]["podTemplate"]["spec"]["volumes"][0],
            json!({"name":"identity","emptyDir":{"medium":"Memory"}})
        );
        for (index, name) in [(2, "tmp"), (3, "home")] {
            assert_eq!(
                body["spec"]["podTemplate"]["spec"]["volumes"][index],
                json!({"name":name,"emptyDir":{}})
            );
        }
        assert_eq!(
            body["spec"]["volumeClaimTemplates"][0]["spec"]["resources"]["requests"]["storage"],
            "20Gi"
        );
        let serialized = serde_json::to_value(&current).unwrap();
        assert!(serialized.get("ephemeral_storage_limit").is_none());
        assert_eq!(
            serde_json::from_value::<SandboxIntent>(serialized).unwrap(),
            current
        );

        for profile in [
            "remote-shell-files-v1",
            "remote-shell-files-v2",
            "unknown-profile",
        ] {
            let mut invalid = current.clone();
            invalid.profile = profile.into();
            assert!(invalid.validate().is_err());
        }
    }

    #[tokio::test]
    async fn apply_is_digest_pinned_and_returns_bound_observation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/storage.k8s.io/v1/storageclasses/managed-csi"))
            .respond_with(ResponseTemplate::new(200).set_body_json(storage_class("managed-csi")))
            .expect(1)
            .mount(&server)
            .await;
        let desired = intent(&server);
        Mock::given(method("PATCH"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/machine-123",
            ))
            .and(query_param("fieldManager", "nyxid-machine-manager"))
            .and(header("content-type", "application/apply-patch+yaml"))
            .and(header("authorization", "Bearer test-token"))
            .and(body_json(desired.apply_object()))
            .respond_with(ResponseTemplate::new(200).set_body_json(object(&desired, "sandbox-uid")))
            .mount(&server)
            .await;
        let api =
            SandboxApi::new(server.uri().parse().unwrap(), Some("test-token".into())).unwrap();
        let observed = api.apply(&desired).await.unwrap();
        assert_eq!(observed.sandbox_uid.as_deref(), Some("sandbox-uid"));
        assert_eq!(observed.runtime_generation.as_deref(), Some("4"));
        assert!(observed.ready);
    }

    fn storage_class(name: &str) -> Value {
        json!({"apiVersion":"storage.k8s.io/v1","kind":"StorageClass",
            "metadata":{"name":name,"uid":"class-uid","creationTimestamp":"2026-10-15T00:00:00Z"},
            "provisioner":"fixture.csi.example"})
    }

    fn storage_class_list(items: Vec<Value>) -> Value {
        json!({"apiVersion":"storage.k8s.io/v1","kind":"StorageClassList",
            "metadata":{"resourceVersion":"7"},"items":items})
    }

    #[tokio::test]
    async fn storage_class_explicit_requires_existing_nondeleting_valid_provisioner() {
        let server = MockServer::start().await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let valid = storage_class("managed-csi");
        let mut deleting = valid.clone();
        deleting["metadata"]["deletionTimestamp"] = json!("2026-10-15T01:00:00Z");
        let mut missing_uid = valid.clone();
        missing_uid["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("uid");
        let mut foreign = valid.clone();
        foreign["metadata"]["name"] = json!("other");
        for object in [deleting, missing_uid, foreign] {
            Mock::given(method("GET"))
                .and(path("/apis/storage.k8s.io/v1/storageclasses/managed-csi"))
                .respond_with(ResponseTemplate::new(200).set_body_json(object))
                .expect(1)
                .mount(&server)
                .await;
            assert!(
                api.resolve_storage_class(Some("managed-csi"))
                    .await
                    .is_err()
            );
            server.reset().await;
        }
        for provisioner in [
            json!(null),
            json!(""),
            json!(" "),
            json!("bad/name/again"),
            json!("https://secret.invalid"),
            json!("-bad"),
            json!("bad!"),
        ] {
            let mut invalid = valid.clone();
            invalid["provisioner"] = provisioner;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(invalid))
                .expect(1)
                .mount(&server)
                .await;
            assert!(
                api.resolve_storage_class(Some("managed-csi"))
                    .await
                    .is_err()
            );
            server.reset().await;
        }
        for status in [403, 404, 500] {
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            assert!(
                api.resolve_storage_class(Some("managed-csi"))
                    .await
                    .is_err()
            );
            server.reset().await;
        }
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(valid))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            api.resolve_storage_class(Some("managed-csi"))
                .await
                .unwrap(),
            "managed-csi"
        );
        for name in ["", " ", "Bad", "bad/path", "bad?query", "-bad"] {
            assert!(api.resolve_storage_class(Some(name)).await.is_err());
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn default_storage_uses_newest_annotation_with_kubernetes_name_tie_break() {
        let server = MockServer::start().await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let mut old = storage_class("old");
        old["metadata"]["annotations"] =
            json!({"storageclass.kubernetes.io/is-default-class":"true"});
        let mut recent_z = storage_class("recent-z");
        recent_z["metadata"]["creationTimestamp"] = json!("2026-10-15T01:00:00.5Z");
        recent_z["metadata"]["annotations"] =
            json!({"storageclass.beta.kubernetes.io/is-default-class":"true"});
        let mut recent_a = recent_z.clone();
        recent_a["metadata"]["name"] = json!("recent-a");
        let mut page1 = storage_class_list(vec![old]);
        page1["metadata"]["continue"] = json!("page-2");
        Mock::given(method("GET"))
            .and(path("/apis/storage.k8s.io/v1/storageclasses"))
            .and(query_param("limit", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page1))
            .with_priority(10)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("continue", "page-2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(storage_class_list(vec![recent_z, recent_a])),
            )
            .with_priority(1)
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(api.resolve_storage_class(None).await.unwrap(), "recent-a");
    }

    #[tokio::test]
    async fn default_storage_absence_permission_invalid_or_incomplete_inventory_is_not_ready() {
        let server = MockServer::start().await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let mut selected = storage_class("default");
        selected["metadata"]["annotations"] =
            json!({"storageclass.kubernetes.io/is-default-class":"true"});
        let mut deleting = selected.clone();
        deleting["metadata"]["deletionTimestamp"] = json!("2026-10-15T01:00:00Z");
        let mut invalid = selected.clone();
        invalid["provisioner"] = json!("");
        let mut undated = selected.clone();
        undated["metadata"]["creationTimestamp"] = json!(null);
        for objects in [
            vec![],
            vec![storage_class("nondefault")],
            vec![deleting],
            vec![invalid],
            vec![undated],
            vec![selected.clone(); 101],
        ] {
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(storage_class_list(objects)))
                .expect(1)
                .mount(&server)
                .await;
            assert!(api.resolve_storage_class(None).await.is_err());
            server.reset().await;
        }
        for status in [403, 404, 500] {
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            assert!(api.resolve_storage_class(None).await.is_err());
            server.reset().await;
        }
        let mut looping = storage_class_list(vec![]);
        looping["metadata"]["continue"] = json!("loop");
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(looping))
            .expect(2)
            .mount(&server)
            .await;
        assert!(api.resolve_storage_class(None).await.is_err());
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(256 * 1024 + 1)))
            .expect(1)
            .mount(&server)
            .await;
        assert!(api.resolve_storage_class(None).await.is_err());
    }

    #[tokio::test]
    async fn fresh_default_apply_omits_field_on_actual_patch_after_validating_default() {
        let server = MockServer::start().await;
        let mut desired = intent(&server);
        desired.workspace_storage_class = None;
        let mut selected = storage_class("cluster-default");
        selected["metadata"]["annotations"] =
            json!({"storageclass.kubernetes.io/is-default-class":"true"});
        Mock::given(method("GET"))
            .and(path("/apis/storage.k8s.io/v1/storageclasses"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(storage_class_list(vec![selected])),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(object(&desired, "sandbox-uid")))
            .expect(1)
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        api.apply(&desired).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let patch: Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert!(
            patch
                .pointer("/spec/volumeClaimTemplates/0/spec/storageClassName")
                .is_none()
        );
    }

    #[tokio::test]
    async fn resume_retains_default_omission_across_explicit_selection_and_default_changes() {
        for (original_class, new_class) in [
            (None, Some("new-class")),
            (None, None),
            (Some("old-class"), None),
        ] {
            let server = MockServer::start().await;
            let mut original = intent(&server);
            original.workspace_storage_class = original_class.map(str::to_owned);
            let mut desired = original.clone();
            desired.workspace_storage_class = new_class.map(str::to_owned);
            let live = object(&original, "sandbox-uid");
            Mock::given(method("GET"))
                .and(path(
                    "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/machine-123",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(live.clone()))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("PATCH"))
                .respond_with(ResponseTemplate::new(200).set_body_json(live.clone()))
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            api.apply_if_uid(&desired, "sandbox-uid").await.unwrap();
            let requests = server.received_requests().await.unwrap();
            let patch: Value = serde_json::from_slice(&requests[1].body).unwrap();
            assert_eq!(
                patch["spec"]["volumeClaimTemplates"],
                live["spec"]["volumeClaimTemplates"]
            );
            assert_eq!(
                requests.len(),
                2,
                "resume does not consult storage default or provision a replacement"
            );
        }
    }

    #[tokio::test]
    async fn empty_null_or_invalid_storage_classes_cannot_disable_defaulting_on_resume() {
        for value in [json!(""), json!(null), json!("bad/path")] {
            let server = MockServer::start().await;
            let desired = intent(&server);
            let mut live = object(&desired, "sandbox-uid");
            live["spec"]["volumeClaimTemplates"][0]["spec"]["storageClassName"] = value;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(live))
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            assert!(api.apply_if_uid(&desired, "sandbox-uid").await.is_err());
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn fresh_apply_rejects_missing_storage_class_before_sandbox_mutation() {
        let server = MockServer::start().await;
        let desired = intent(&server);
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(object(&desired, "sandbox-uid")))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.apply(&desired).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test]
    async fn default_storage_intent_omits_storage_class_instead_of_disabling_defaulting() {
        let server = MockServer::start().await;
        let mut serialized = serde_json::to_value(intent(&server)).unwrap();
        serialized
            .as_object_mut()
            .unwrap()
            .remove("workspace_storage_class");
        let desired: SandboxIntent = serde_json::from_value(serialized).unwrap();
        desired.validate().unwrap();
        assert!(
            desired
                .apply_object()
                .pointer("/spec/volumeClaimTemplates/0/spec/storageClassName")
                .is_none()
        );
    }

    #[tokio::test]
    async fn resume_keeps_original_workspace_template_when_configured_class_changes() {
        let server = MockServer::start().await;
        let original = intent(&server);
        let mut desired = original.clone();
        desired.workspace_storage_class = Some("different-csi".into());
        let live = object(&original, "sandbox-uid");
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(live.clone()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(live.clone()))
            .expect(1)
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        api.apply_if_uid(&desired, "sandbox-uid").await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let patched: Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(
            patched["spec"]["volumeClaimTemplates"],
            live["spec"]["volumeClaimTemplates"]
        );
    }

    #[tokio::test]
    async fn apply_if_uid_requires_live_uid_and_uses_kubernetes_apply_media_type() {
        let server = MockServer::start().await;
        let desired = intent(&server);
        Mock::given(method("GET"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/machine-123",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(object(&desired, "sandbox-uid")))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/machine-123",
            ))
            .and(header("content-type", "application/apply-patch+yaml"))
            .and(body_json({
                let mut body = desired.apply_object();
                body["metadata"]["uid"] = json!("sandbox-uid");
                body["metadata"]["resourceVersion"] = json!("7");
                body
            }))
            .respond_with(ResponseTemplate::new(200).set_body_json(object(&desired, "sandbox-uid")))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let observed = api.apply_if_uid(&desired, "sandbox-uid").await.unwrap();
        assert_eq!(observed.sandbox_uid.as_deref(), Some("sandbox-uid"));
        assert!(api.apply_if_uid(&desired, "different-uid").await.is_err());
    }

    #[tokio::test]
    async fn uid_apply_rejects_extra_workspace_labels_without_patching() {
        let server = MockServer::start().await;
        let desired = intent(&server);
        let mut live = object(&desired, "sandbox-uid");
        live["spec"]["volumeClaimTemplates"][0]["metadata"]["labels"]["nyxid.dev/operation-id"] =
            json!("old-operation");
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(live))
            .expect(1)
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.apply_if_uid(&desired, "sandbox-uid").await.is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn uid_apply_rejects_foreign_or_mutated_workspace_without_patching() {
        for pointer in [
            "/spec/volumeClaimTemplates/0/spec/resources/requests/storage",
            "/spec/volumeClaimTemplates/0/metadata/labels/nyxid.dev~1owner-id",
        ] {
            let server = MockServer::start().await;
            let desired = intent(&server);
            let mut live = object(&desired, "sandbox-uid");
            *live.pointer_mut(pointer).unwrap() = json!("foreign");
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(live))
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            assert!(api.apply_if_uid(&desired, "sandbox-uid").await.is_err());
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn delete_if_uid_sends_uid_precondition() {
        let server = MockServer::start().await;
        let desired = intent(&server);
        Mock::given(method("GET"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/machine-123",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(object(&desired, "sandbox-uid")))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/machine-123",
            ))
            .and(body_json(json!({"preconditions": {"uid": "sandbox-uid"}})))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(
            api.delete_if_uid("nyxid-machine", "machine-123", "sandbox-uid")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn list_managed_uses_the_immutable_managed_label() {
        let server = MockServer::start().await;
        let desired = intent(&server);
        Mock::given(method("GET"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes",
            ))
            .and(query_param("labelSelector", "nyxid.dev/managed=true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [object(&desired, "sandbox-uid")]
            })))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let listed = api.list_managed("nyxid-machine").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].sandbox_uid.as_deref(), Some("sandbox-uid"));
    }

    #[tokio::test]
    async fn inventory_follows_bounded_pages_and_rejects_repeated_continuation() {
        let server = MockServer::start().await;
        let first = object(&intent(&server), "first-uid");
        let second = object(&intent(&server), "second-uid");
        Mock::given(method("GET"))
            .and(query_param("limit", "100"))
            .respond_with(move |request: &wiremock::Request| {
                if request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "continue" && value == "next-page")
                {
                    ResponseTemplate::new(200).set_body_json(json!({"items":[second.clone()]}))
                } else {
                    ResponseTemplate::new(200).set_body_json(
                        json!({"metadata":{"continue":"next-page"},"items":[first.clone()]}),
                    )
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let rows = api.list_managed("nyxid-machine").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].sandbox_uid.as_deref(), Some("second-uid"));
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"metadata":{"continue":"loop"},"items":[]})),
            )
            .expect(2)
            .mount(&server)
            .await;
        assert!(api.list_managed("nyxid-machine").await.is_err());
    }

    fn controlled_resource(kind: &str) -> Value {
        json!({"kind":kind,"metadata":{"namespace":"nyxid-machine","name":"retained-resource",
            "uid":uuid::Uuid::new_v4().to_string(),"labels":{
                "nyxid.dev/managed":"true","nyxid.dev/machine-id":uuid::Uuid::new_v4().to_string(),
                "nyxid.dev/owner-id":uuid::Uuid::new_v4().to_string(),"nyxid.dev/operation-id":uuid::Uuid::new_v4().to_string(),
                "nyxid.dev/runtime-generation":"1","nyxid.dev/credential-epoch":"1","nyxid.dev/profile":nyxid_machine::managed::PROFILE},
            "ownerReferences":[{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","controller":true,"uid":uuid::Uuid::new_v4().to_string()}]}})
    }

    #[test]
    fn controlled_inventory_recognizes_only_current_profile() {
        for kind in ["Pod", "PersistentVolumeClaim"] {
            let mut object = controlled_resource(kind);
            let observed =
                ControlledResourceObservation::from_object(&object, kind, "nyxid-machine");
            assert!(observed.metadata_valid);
            assert_eq!(
                observed.profile.as_deref(),
                Some(nyxid_machine::managed::PROFILE)
            );
            for profile in [
                "remote-shell-files-v1",
                "remote-shell-files-v2",
                "unknown-profile",
            ] {
                object["metadata"]["labels"]["nyxid.dev/profile"] = json!(profile);
                assert!(
                    !ControlledResourceObservation::from_object(&object, kind, "nyxid-machine")
                        .metadata_valid
                );
            }
        }
    }

    #[tokio::test]
    async fn controlled_resource_inventory_is_independent_paginated_and_read_only() {
        let server = MockServer::start().await;
        for (resource, kind) in [
            ("pods", "Pod"),
            ("persistentvolumeclaims", "PersistentVolumeClaim"),
        ] {
            let first = controlled_resource(kind);
            let second = controlled_resource(kind);
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/namespaces/nyxid-machine/{resource}")))
                .and(query_param("labelSelector", "nyxid.dev/managed=true"))
                .and(query_param("limit", "100"))
                .respond_with(move |request: &wiremock::Request| {
                    if request
                        .url
                        .query_pairs()
                        .any(|(key, value)| key == "continue" && value == "next")
                    {
                        ResponseTemplate::new(200).set_body_json(json!({"items":[second.clone()]}))
                    } else {
                        ResponseTemplate::new(200).set_body_json(
                            json!({"items":[first.clone()],"metadata":{"continue":"next"}}),
                        )
                    }
                })
                .expect(2)
                .mount(&server)
                .await;
        }
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        for resource in ["pods", "persistentvolumeclaims"] {
            let rows = api
                .list_controlled_resources("nyxid-machine", resource)
                .await
                .unwrap();
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|row| row.metadata_valid));
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(
            requests
                .iter()
                .all(|request| request.method.as_str() == "GET"
                    && !request.url.path().contains("sandboxes"))
        );
        assert!(
            api.list_controlled_resources("nyxid-machine", "secrets")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn controlled_resource_inventory_retains_invalid_identity_for_safe_investigation() {
        let server = MockServer::start().await;
        let mut malformed = controlled_resource("Pod");
        malformed["metadata"]["uid"] = json!("not-a-uid-secret-content");
        malformed["metadata"]["name"] = json!("../../private\nunsafe");
        malformed["metadata"]["labels"]["nyxid.dev/owner-id"] = json!("untrusted-owner");
        malformed["metadata"]["annotations"] = json!({"unrelated-secret":"must-not-be-projected"});
        let mut untrusted = controlled_resource("Pod");
        untrusted["metadata"]["ownerReferences"][0]["kind"] = json!("DifferentController");
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"items":[malformed.clone(),untrusted]})),
            )
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let rows = api
            .list_controlled_resources("nyxid-machine", "pods")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| !row.metadata_valid));
        assert!(rows[0].uid.is_none() && rows[0].name.is_none() && rows[0].owner_id.is_none());
        assert!(rows[1].sandbox_owner_uid.is_none());
        malformed["metadata"]["resourceVersion"] = json!("new-observation");
        let repeated =
            ControlledResourceObservation::from_object(&malformed, "Pod", "nyxid-machine");
        assert_eq!(rows[0].identity_hash, repeated.identity_hash);
        let debug = format!("{rows:?}");
        assert!(
            !debug.contains("secret-content")
                && !debug.contains("must-not-be-projected")
                && !debug.contains("../../private")
        );
    }

    #[tokio::test]
    async fn controlled_resource_inventory_rejects_overfull_and_looping_pages() {
        let server = MockServer::start().await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"items":vec![controlled_resource("Pod");101]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            api.list_controlled_resources("nyxid-machine", "pods")
                .await
                .is_err()
        );
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"items":[],"metadata":{"continue":"loop"}})),
            )
            .expect(2)
            .mount(&server)
            .await;
        assert!(
            api.list_controlled_resources("nyxid-machine", "persistentvolumeclaims")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn stale_generation_and_profile_widening_fail_closed() {
        let server = MockServer::start().await;
        let desired = intent(&server);
        let mut stale = object(&desired, "sandbox-uid");
        stale["metadata"]["labels"]["nyxid.dev/credential-epoch"] = json!("1");
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(stale))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.get_for_intent(&desired).await.is_err());
        let mut widened = desired.clone();
        widened.memory_limit = "4Gi".into();
        assert!(api.apply(&widened).await.is_err());
        let mut valid = desired;
        valid.memory_limit = "2048Mi".into();
        valid.cpu_request = "2000m".into();
        valid.validate().unwrap();
        let body = valid.apply_object();
        assert!(body.pointer("/spec/podTemplate/spec/volumes").is_some());
        assert!(body.pointer("/spec/podTemplate/volumes").is_none());
        assert_eq!(
            body.pointer("/spec/podTemplate/spec/containers/0/resources/limits/memory"),
            Some(&json!("2Gi"))
        );
        assert_eq!(
            body.pointer("/spec/podTemplate/metadata/labels/nyxid.dev~1writer-fence"),
            Some(&json!(valid.writer_fence))
        );
    }

    #[tokio::test]
    async fn list_owned_resources_filters_by_sandbox_owner_uid() {
        let server = MockServer::start().await;
        let mut owned = recovery_owned_resource("Pod", "owned", "pod-1", "sandbox-uid");
        owned["status"] = json!({"phase":"Running"});
        let foreign = recovery_owned_resource("Pod", "foreign", "pod-2", "other");
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/nyxid-machine/pods"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "kind":"PodList","apiVersion":"v1","metadata":{"resourceVersion":"40"},
                "items": [owned, foreign]
            })))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let listed = api
            .list_owned_resources("nyxid-machine", "pods", "sandbox-uid")
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "owned");
        assert_eq!(listed[0].phase.as_deref(), Some("Running"));
    }

    fn recovery_owned_resource(kind: &str, name: &str, uid: &str, sandbox_uid: &str) -> Value {
        json!({"kind":kind,"apiVersion":"v1","metadata":{
            "name":name,"namespace":"nyxid-machine","uid":uid,"resourceVersion":"3",
            "ownerReferences":[{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox",
                "name":"original-sandbox","uid":sandbox_uid,"controller":true}]}})
    }

    #[tokio::test]
    async fn owned_cleanup_inventory_accepts_omitted_item_type_metadata() {
        let server = MockServer::start().await;
        let mut claim = recovery_owned_resource(
            "PersistentVolumeClaim",
            "workspace-original",
            "pvc-uid",
            "sandbox-uid",
        );
        claim.as_object_mut().unwrap().remove("kind");
        claim.as_object_mut().unwrap().remove("apiVersion");
        Mock::given(method("GET"))
            .and(path(
                "/api/v1/namespaces/nyxid-machine/persistentvolumeclaims",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "kind": "PersistentVolumeClaimList", "apiVersion": "v1",
                "metadata": {"resourceVersion": "40"}, "items": [claim]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let rows = api
            .list_owned_resources("nyxid-machine", "persistentvolumeclaims", "sandbox-uid")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "PersistentVolumeClaim");
        assert_eq!(rows[0].uid.as_deref(), Some("pvc-uid"));
    }

    #[tokio::test]
    async fn owned_cleanup_inventory_rejects_nonstring_continuation() {
        for (resource, kind) in [
            ("pods", "Pod"),
            ("persistentvolumeclaims", "PersistentVolumeClaim"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/namespaces/nyxid-machine/{resource}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "kind":format!("{kind}List"),"apiVersion":"v1",
                    "metadata":{"resourceVersion":"40","continue":123},"items":[]
                })))
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let error = api
                .list_owned_resources("nyxid-machine", resource, "sandbox-uid")
                .await
                .unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<SandboxObservationError>()
                    .unwrap()
                    .error_kind,
                ObservationErrorKind::PaginationIncomplete
            );
        }
    }

    #[tokio::test]
    async fn owned_cleanup_inventory_rejects_remaining_items_without_continuation() {
        for (resource, kind) in [
            ("pods", "Pod"),
            ("persistentvolumeclaims", "PersistentVolumeClaim"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "kind":format!("{kind}List"),"apiVersion":"v1",
                    "metadata":{"resourceVersion":"40","remainingItemCount":1},"items":[]
                })))
                .expect(1)
                .mount(&server)
                .await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let error = api
                .list_owned_resources("nyxid-machine", resource, "sandbox-uid")
                .await
                .unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<SandboxObservationError>()
                    .unwrap()
                    .error_kind,
                ObservationErrorKind::PaginationIncomplete
            );
        }
    }

    #[tokio::test]
    async fn owned_cleanup_inventory_rejects_malformed_items_before_ownership_filtering() {
        for (resource, kind) in [
            ("pods", "Pod"),
            ("persistentvolumeclaims", "PersistentVolumeClaim"),
        ] {
            let valid = recovery_owned_resource(kind, "original", "original-uid", "sandbox-uid");
            for defect in [
                "missing-name",
                "missing-uid",
                "invalid-owners",
                "missing-owner-uid",
                "missing-owner-kind",
                "invalid-controller",
                "wrong-namespace",
            ] {
                let server = MockServer::start().await;
                // Even a malformed unrelated item must not be silently dropped.
                let mut malformed = recovery_owned_resource(
                    kind,
                    "malformed",
                    "malformed-uid",
                    "different-sandbox",
                );
                match defect {
                    "missing-name" => {
                        malformed["metadata"]
                            .as_object_mut()
                            .unwrap()
                            .remove("name");
                    }
                    "missing-uid" => {
                        malformed["metadata"].as_object_mut().unwrap().remove("uid");
                    }
                    "invalid-owners" => {
                        malformed["metadata"]["ownerReferences"] = json!({"uid":"sandbox-uid"})
                    }
                    "missing-owner-uid" => {
                        malformed["metadata"]["ownerReferences"][0]
                            .as_object_mut()
                            .unwrap()
                            .remove("uid");
                    }
                    "missing-owner-kind" => {
                        malformed["metadata"]["ownerReferences"][0]
                            .as_object_mut()
                            .unwrap()
                            .remove("kind");
                    }
                    "invalid-controller" => {
                        malformed["metadata"]["ownerReferences"][0]["controller"] = json!("true")
                    }
                    "wrong-namespace" => {
                        malformed["metadata"]["namespace"] = json!("other-namespace")
                    }
                    _ => unreachable!(),
                }
                Mock::given(method("GET"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                        "kind":format!("{kind}List"),"apiVersion":"v1",
                        "metadata":{"resourceVersion":"40"},"items":[valid.clone(),malformed]
                    })))
                    .expect(1)
                    .mount(&server)
                    .await;
                let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
                assert!(
                    api.list_owned_resources("nyxid-machine", resource, "sandbox-uid")
                        .await
                        .is_err(),
                    "accepted {resource} {defect}"
                );
            }
        }
    }

    #[tokio::test]
    async fn owned_cleanup_inventory_returns_only_matches_after_complete_valid_pagination() {
        for (resource, kind) in [
            ("pods", "Pod"),
            ("persistentvolumeclaims", "PersistentVolumeClaim"),
        ] {
            let server = MockServer::start().await;
            let original = recovery_owned_resource(kind, "original", "original-uid", "sandbox-uid");
            let late = recovery_owned_resource(kind, "late", "late-uid", "sandbox-uid");
            let foreign =
                recovery_owned_resource(kind, "foreign", "foreign-uid", "different-sandbox");
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/namespaces/nyxid-machine/{resource}")))
                .and(query_param("limit", "100"))
                .respond_with(move |request: &wiremock::Request| {
                    assert!(!request.url.query_pairs().any(|(key,_)| key == "labelSelector"));
                    if request.url.query_pairs().any(|(key,value)| key == "continue" && value == "next") {
                        ResponseTemplate::new(200).set_body_json(json!({
                            "kind":format!("{kind}List"),"apiVersion":"v1",
                            "metadata":{"resourceVersion":"40","remainingItemCount":0},"items":[late.clone()]
                        }))
                    } else {
                        ResponseTemplate::new(200).set_body_json(json!({
                            "kind":format!("{kind}List"),"apiVersion":"v1",
                            "metadata":{"resourceVersion":"40","continue":"next","remainingItemCount":1},"items":[original.clone(),foreign.clone()]
                        }))
                    }
                })
                .expect(2).mount(&server).await;
            let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
            let rows = api
                .list_owned_resources("nyxid-machine", resource, "sandbox-uid")
                .await
                .unwrap();
            assert_eq!(
                rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
                vec!["original", "late"]
            );
            assert!(rows.iter().all(|row| row.ownership_valid));
        }
    }

    #[tokio::test]
    async fn owned_cleanup_inventory_rejects_later_invalid_page_without_partial_results() {
        let server = MockServer::start().await;
        let original = recovery_owned_resource(
            "PersistentVolumeClaim",
            "original",
            "original-uid",
            "sandbox-uid",
        );
        Mock::given(method("GET"))
            .respond_with(move |request: &wiremock::Request| {
                if request.url.query_pairs().any(|(key,_)| key == "continue") {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "kind":"PersistentVolumeClaimList","apiVersion":"v1",
                        "metadata":{"resourceVersion":"41"},"items":[]
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "kind":"PersistentVolumeClaimList","apiVersion":"v1",
                        "metadata":{"resourceVersion":"40","continue":"next"},"items":[original.clone()]
                    }))
                }
            })
            .expect(2).mount(&server).await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        let error = api
            .list_owned_resources("nyxid-machine", "persistentvolumeclaims", "sandbox-uid")
            .await
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<SandboxObservationError>()
                .unwrap()
                .error_kind,
            ObservationErrorKind::PaginationIncomplete
        );
    }

    #[tokio::test]
    async fn get_preserves_not_found_as_reconcilable_absence() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/missing",
            ))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.get("nyxid-machine", "missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn deterministic_name_collision_is_rejected_instead_of_adopted() {
        let server = MockServer::start().await;
        let desired = intent(&server);
        let mut foreign = desired.clone();
        foreign.machine_id = "another-machine".into();
        Mock::given(method("GET"))
            .and(path(
                "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes/machine-123",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(object(&foreign, "foreign-uid")))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.get_for_intent(&desired).await.is_err());
    }

    #[tokio::test]
    async fn invalid_digest_and_conflict_fail_closed() {
        let server = MockServer::start().await;
        let mut desired = intent(&server);
        desired.image = "ghcr.io/example:mutable".into();
        let api =
            SandboxApi::new(server.uri().parse().unwrap(), Some("test-token".into())).unwrap();
        assert!(api.apply(&desired).await.is_err());

        let desired = intent(&server);
        Mock::given(method("GET"))
            .and(path("/apis/storage.k8s.io/v1/storageclasses/managed-csi"))
            .respond_with(ResponseTemplate::new(200).set_body_json(storage_class("managed-csi")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;
        assert!(api.apply(&desired).await.is_err());
    }

    #[tokio::test]
    async fn health_failure_is_not_reported_as_ready() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/apis/agents.x-k8s.io/v1beta1"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let api = SandboxApi::new(server.uri().parse().unwrap(), None).unwrap();
        assert!(api.health().await.is_err());
    }
}
