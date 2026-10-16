//! Optional read-only operator diagnostics for a selected Kubernetes Deployment.
//! Ordinary lifecycle admission uses the Sandbox/native observation contract;
//! missing selection, denied Deployment RBAC or an unavailable detailed view
//! does not block Machine create/run/pause/resume/delete.
//! The expected image digest is operator configuration, not inferred from a
//! repository commit. Matching it does not prove CRD or physical compatibility.
//!
//! Configuration: NYXID_REMOTE_MACHINE_CONTROLLER_NAMESPACE,
//! NYXID_REMOTE_MACHINE_CONTROLLER_DEPLOYMENT,
//! NYXID_REMOTE_MACHINE_CONTROLLER_IMAGE_DIGEST (sha256:<64 lowercase hex>),
//! and optional NYXID_REMOTE_MACHINE_CONTROLLER_CONTAINER. Multiple containers
//! require an explicit container to make an image-match assertion. Transport
//! reuses the configured Kubernetes API, protected token file, CA and limits.
//! RBAC: get apps/deployments, restricted to the selected namespace/name.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use super::remote_machine_kubernetes_identity::KubernetesWorkloadVerifier;

const MAX_CONTAINERS: usize = 32;
const MAX_IMAGE_BYTES: usize = 512;
const MAX_REPLICAS: u64 = 1_000_000;

#[derive(Serialize)]
pub struct ContainerImageView {
    name: String,
    image: String,
}

#[derive(Serialize)]
pub struct ControllerHealthView {
    status: &'static str,
    namespace: Option<String>,
    deployment: Option<String>,
    expected_image_digest: Option<String>,
    selected_container: Option<String>,
    generation: Option<i64>,
    observed_generation: Option<i64>,
    desired_replicas: Option<u32>,
    replicas: Option<u32>,
    ready_replicas: Option<u32>,
    updated_replicas: Option<u32>,
    available_replicas: Option<u32>,
    unavailable_replicas: Option<u32>,
    template_images: Vec<ContainerImageView>,
    rollout_match: Option<bool>,
    expected_digest_match: Option<bool>,
    reasons: Vec<&'static str>,
    image_evidence: &'static str,
    compatibility: &'static str,
}

impl ControllerHealthView {
    fn empty(status: &'static str, reason: &'static str) -> Self {
        Self {
            status,
            namespace: None,
            deployment: None,
            expected_image_digest: None,
            selected_container: None,
            generation: None,
            observed_generation: None,
            desired_replicas: None,
            replicas: None,
            ready_replicas: None,
            updated_replicas: None,
            available_replicas: None,
            unavailable_replicas: None,
            template_images: Vec::new(),
            rollout_match: None,
            expected_digest_match: None,
            reasons: vec![reason],
            image_evidence: "deployment_template_only_not_running_image_ids",
            compatibility: "not_established_by_rollout_or_image_digest_match",
        }
    }
}

pub(crate) struct Selection {
    pub(crate) namespace: String,
    pub(crate) deployment: String,
    digest: String,
    container: Option<String>,
}

impl Selection {
    pub(crate) fn from_environment() -> Result<Self, &'static str> {
        let value =
            |suffix: &str| std::env::var(format!("NYXID_REMOTE_MACHINE_CONTROLLER_{suffix}")).ok();
        Self::parse(
            value("NAMESPACE"),
            value("DEPLOYMENT"),
            value("IMAGE_DIGEST"),
            value("CONTAINER"),
        )
    }

    pub(crate) fn parse(
        namespace: Option<String>,
        deployment: Option<String>,
        digest: Option<String>,
        container: Option<String>,
    ) -> Result<Self, &'static str> {
        let namespace = namespace.ok_or("controller_namespace_not_configured")?;
        let deployment = deployment.ok_or("controller_deployment_not_configured")?;
        let digest = digest.ok_or("controller_image_digest_not_configured")?;
        if !dns_label(&namespace) {
            return Err("controller_namespace_invalid");
        }
        if deployment.len() > 253 || !deployment.split('.').all(dns_label) {
            return Err("controller_deployment_invalid");
        }
        if !sha256_digest(&digest) {
            return Err("controller_image_digest_invalid");
        }
        if container.as_deref().is_some_and(|name| !dns_label(name)) {
            return Err("controller_container_invalid");
        }
        Ok(Self {
            namespace,
            deployment,
            digest,
            container,
        })
    }

    fn view(&self) -> ControllerHealthView {
        let mut view = ControllerHealthView::empty("unknown", "deployment_not_observed");
        view.namespace = Some(self.namespace.clone());
        view.deployment = Some(self.deployment.clone());
        view.expected_image_digest = Some(self.digest.clone());
        view.selected_container = self.container.clone();
        view
    }
}

fn dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')
}

fn sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
    })
}

fn image_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IMAGE_BYTES
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"/._:@-".contains(&ch))
        && !value.contains("://")
        && match value.split_once('@') {
            Some((name, digest)) => !name.is_empty() && sha256_digest(digest),
            None => true,
        }
}

/// Called only by the existing human operator platform endpoint. Missing
/// configuration and denied/unavailable API access remain explicit unknowns.
/// This diagnostic is not a normal lifecycle prerequisite.
pub async fn observe(verifier: Option<&KubernetesWorkloadVerifier>) -> ControllerHealthView {
    let selection = match Selection::from_environment() {
        Ok(selection) => selection,
        Err(reason) => return ControllerHealthView::empty("unconfigured", reason),
    };
    let Some(verifier) = verifier else {
        let mut view = selection.view();
        view.status = "unavailable";
        view.reasons = vec!["kubernetes_transport_unconfigured"];
        return view;
    };
    observe_selected(verifier, &selection).await
}

async fn observe_selected(
    verifier: &KubernetesWorkloadVerifier,
    selection: &Selection,
) -> ControllerHealthView {
    let path = format!(
        "/apis/apps/v1/namespaces/{}/deployments/{}",
        selection.namespace, selection.deployment,
    );
    match tokio::time::timeout(Duration::from_secs(5), verifier.read_json(&path)).await {
        Ok(Ok(value)) => project(selection, &value),
        // Never return upstream error text, token, URL, response body, or headers.
        _ => {
            let mut view = selection.view();
            view.status = "unavailable";
            view.reasons = vec!["deployment_read_unavailable_or_not_authorized"];
            view
        }
    }
}

fn project(selection: &Selection, value: &Value) -> ControllerHealthView {
    let mut view = selection.view();
    if value["apiVersion"] != "apps/v1"
        || value["kind"] != "Deployment"
        || value["metadata"]["namespace"].as_str() != Some(selection.namespace.as_str())
        || value["metadata"]["name"].as_str() != Some(selection.deployment.as_str())
    {
        view.reasons = vec!["deployment_identity_unknown_or_mismatched"];
        return view;
    }
    let Some(containers) = value
        .pointer("/spec/template/spec/containers")
        .and_then(Value::as_array)
    else {
        view.reasons = vec!["deployment_container_spec_unknown"];
        return view;
    };
    if containers.is_empty() || containers.len() > MAX_CONTAINERS {
        view.reasons = vec!["deployment_container_projection_limit"];
        return view;
    }
    let mut names = std::collections::BTreeSet::new();
    for container in containers {
        let (Some(name), Some(image)) = (container["name"].as_str(), container["image"].as_str())
        else {
            view.reasons = vec!["deployment_container_spec_unknown"];
            return view;
        };
        if !dns_label(name) || !image_reference(image) || !names.insert(name) {
            view.template_images.clear();
            view.reasons = vec!["deployment_container_projection_invalid"];
            return view;
        }
        view.template_images.push(ContainerImageView {
            name: name.into(),
            image: image.into(),
        });
    }
    view.reasons.clear();
    let selected = if let Some(container) = &selection.container {
        view.template_images
            .iter()
            .find(|image| &image.name == container)
    } else if view.template_images.len() == 1 {
        view.template_images.first()
    } else {
        None
    };
    if let Some(selected) = selected {
        view.selected_container = Some(selected.name.clone());
        view.expected_digest_match = Some(
            selected
                .image
                .rsplit_once('@')
                .is_some_and(|(_, digest)| digest == selection.digest),
        );
    } else {
        view.reasons
            .push("controller_container_unknown_or_ambiguous");
    }
    let positive = |value: &Value| value.as_i64().filter(|value| *value > 0);
    let count = |value: &Value| {
        value
            .as_u64()
            .filter(|value| *value <= MAX_REPLICAS)
            .map(|value| value as u32)
    };
    view.generation = positive(&value["metadata"]["generation"]);
    view.observed_generation = positive(&value["status"]["observedGeneration"]);
    view.desired_replicas = count(&value["spec"]["replicas"]);
    view.replicas = count(&value["status"]["replicas"]);
    view.ready_replicas = count(&value["status"]["readyReplicas"]);
    view.updated_replicas = count(&value["status"]["updatedReplicas"]);
    view.available_replicas = count(&value["status"]["availableReplicas"]);
    view.unavailable_replicas = count(&value["status"]["unavailableReplicas"]);
    let invalid_count = [
        "/spec/replicas",
        "/status/replicas",
        "/status/readyReplicas",
        "/status/updatedReplicas",
        "/status/availableReplicas",
        "/status/unavailableReplicas",
    ]
    .iter()
    .any(|path| {
        value
            .pointer(path)
            .is_some_and(|value| count(value).is_none())
    });
    view.rollout_match = match (view.generation, view.observed_generation) {
        (Some(generation), Some(observed)) if observed < generation => Some(false),
        (Some(generation), Some(observed)) if observed == generation => {
            match (
                view.desired_replicas,
                view.replicas,
                view.ready_replicas,
                view.updated_replicas,
                view.available_replicas,
            ) {
                (Some(desired), Some(total), Some(ready), Some(updated), Some(available)) => Some(
                    desired > 0
                        && total == desired
                        && ready == desired
                        && updated == desired
                        && available == desired
                        && view.unavailable_replicas.is_none_or(|count| count == 0),
                ),
                _ => None,
            }
        }
        _ => None,
    };
    if invalid_count {
        view.rollout_match = None;
        view.reasons.push("deployment_replica_count_invalid");
    } else if value
        .pointer("/metadata/deletionTimestamp")
        .is_some_and(|value| !value.is_null())
    {
        view.rollout_match = Some(false);
        view.reasons.push("deployment_deletion_in_progress");
    }
    if view.expected_digest_match == Some(false) {
        view.reasons.push("controller_expected_digest_mismatch");
    }
    if view.rollout_match == Some(false) {
        view.reasons.push("deployment_rollout_not_current_or_ready");
    } else if view.rollout_match.is_none() {
        view.reasons.push("deployment_rollout_status_unknown");
    }
    view.status = match (view.expected_digest_match, view.rollout_match) {
        (Some(false), _) => "mismatch",
        (Some(true), Some(true)) => "ready",
        (_, Some(false)) => "not_ready",
        _ => "unknown",
    };
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn selection() -> Selection {
        Selection::parse(
            Some("sandbox-system".into()),
            Some("sandbox-controller".into()),
            Some(format!("sha256:{}", "a".repeat(64))),
            None,
        )
        .unwrap()
    }
    fn deployment() -> Value {
        json!({"apiVersion":"apps/v1","kind":"Deployment",
            "metadata":{"namespace":"sandbox-system","name":"sandbox-controller","generation":5},
            "spec":{"replicas":2,"template":{"spec":{"containers":[{"name":"controller",
                "image":format!("registry.test/controller@sha256:{}","a".repeat(64))}]}}},
            "status":{"observedGeneration":5,"replicas":2,"readyReplicas":2,
                "updatedReplicas":2,"availableReplicas":2}})
    }

    #[test]
    fn only_current_full_rollout_with_exact_configured_digest_is_ready() {
        let mut value = deployment();
        let healthy = project(&selection(), &value);
        assert_eq!(healthy.status, "ready");
        assert_eq!(healthy.rollout_match, Some(true));
        assert_eq!(healthy.expected_digest_match, Some(true));
        assert_eq!(
            healthy.compatibility,
            "not_established_by_rollout_or_image_digest_match"
        );
        value["status"]["observedGeneration"] = json!(4);
        assert_eq!(project(&selection(), &value).rollout_match, Some(false));
        value["status"]["observedGeneration"] = json!(5);
        value["status"]["updatedReplicas"] = json!(1);
        assert_eq!(project(&selection(), &value).rollout_match, Some(false));
        value["status"]["updatedReplicas"] = json!(2);
        value["metadata"]["deletionTimestamp"] = json!("2026-10-13T21:00:00Z");
        let deleting = project(&selection(), &value);
        assert_eq!(deleting.rollout_match, Some(false));
        assert_eq!(deleting.status, "not_ready");
    }

    #[test]
    fn mismatched_digest_and_multi_container_ambiguity_are_not_ready() {
        let mut value = deployment();
        value["spec"]["template"]["spec"]["containers"][0]["image"] =
            json!("registry.test/controller:latest");
        assert_eq!(project(&selection(), &value).status, "mismatch");
        value["spec"]["template"]["spec"]["containers"].as_array_mut().unwrap().push(
            json!({"name":"sidecar","image":format!("registry.test/controller@sha256:{}","a".repeat(64))}));
        let unknown = project(&selection(), &value);
        assert_eq!(unknown.expected_digest_match, None);
        assert_eq!(unknown.status, "unknown");
        let mut selected = selection();
        selected.container = Some("controller".into());
        assert_eq!(
            project(&selected, &value).expected_digest_match,
            Some(false)
        );
    }

    #[test]
    fn missing_invalid_or_over_limit_fields_stay_unknown_and_bounded() {
        assert!(Selection::parse(None, None, None, None).is_err());
        assert!(
            Selection::parse(
                Some("../secrets".into()),
                Some("controller".into()),
                Some(format!("sha256:{}", "a".repeat(64))),
                None
            )
            .is_err()
        );
        assert!(
            Selection::parse(
                Some("system".into()),
                Some("controller".into()),
                Some("main".into()),
                None
            )
            .is_err()
        );
        let mut value = deployment();
        value["status"]["readyReplicas"] = json!(MAX_REPLICAS + 1);
        assert_eq!(project(&selection(), &value).status, "unknown");
        value["status"]["readyReplicas"] = json!(2);
        value["status"]["unavailableReplicas"] = json!(-1);
        assert_eq!(project(&selection(), &value).rollout_match, None);
        value["metadata"]["namespace"] = json!("other");
        assert!(project(&selection(), &value).template_images.is_empty());
        let mut value = deployment();
        value["spec"]["template"]["spec"]["containers"][0]["image"] =
            json!("x".repeat(MAX_IMAGE_BYTES + 1));
        let view = project(&selection(), &value);
        assert_eq!(view.status, "unknown");
        assert!(view.template_images.is_empty());
        assert_eq!(project(&selection(), &json!({})).status, "unknown");
    }

    async fn loopback(status: axum::http::StatusCode, body: Value) -> ControllerHealthView {
        use axum::{Json, Router, http::HeaderMap, routing::get};
        let app = Router::new().route(
            "/apis/apps/v1/namespaces/sandbox-system/deployments/sandbox-controller",
            get(move |headers: HeaderMap| {
                let body = body.clone();
                async move {
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer fixture-controller-token"
                    );
                    (status, Json(body))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let path =
            std::env::temp_dir().join(format!("nyxid-controller-token-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, "fixture-controller-token")
            .await
            .unwrap();
        let verifier = KubernetesWorkloadVerifier::for_test_endpoint(
            format!("http://{address}").parse().unwrap(),
            path.clone(),
        );
        let view = observe_selected(&verifier, &selection()).await;
        server.abort();
        tokio::fs::remove_file(path).await.unwrap();
        assert!(
            !serde_json::to_string(&view)
                .unwrap()
                .contains("fixture-controller-token")
        );
        view
    }

    #[tokio::test]
    async fn deployment_get_reuses_real_kubernetes_bearer_transport_and_safe_projection() {
        let view = loopback(axum::http::StatusCode::OK, deployment()).await;
        assert_eq!(view.status, "ready");
        assert_eq!(view.generation, Some(5));
        assert_eq!(view.ready_replicas, Some(2));
    }

    #[tokio::test]
    async fn api_failure_and_unknown_payload_never_report_deployed_compatibility() {
        let unavailable = loopback(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            json!({"message":"upstream private diagnostics fixture-controller-token"}),
        )
        .await;
        assert_eq!(unavailable.status, "unavailable");
        assert_eq!(unavailable.expected_digest_match, None);
        let unknown = loopback(
            axum::http::StatusCode::OK,
            json!({"status":"not a deployment"}),
        )
        .await;
        assert_eq!(unknown.status, "unknown");
    }
}
