//! Kubernetes is the workload issuer: a projected, audience-bound Pod token is
//! verified with TokenReview, then its live Pod and Sandbox owner are checked.
//! This establishes bootstrap identity only, never physical writer exclusion.
use nyxid_machine_updater::kubernetes_credentials::KubernetesCredentials;
#[cfg(test)]
use std::path::PathBuf;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use nyxid_machine::managed::Binding;
use reqwest::{Client, Method, Url};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::remote_machine_bootstrap::{BootstrapTicketVerifier, VerifiedBootstrapTicket};
use super::remote_machine_bootstrap_verifier::AUDIENCE;
use crate::errors::{AppError, AppResult};

const WORKLOAD_ACCOUNT: &str = "nyxid-machine-workload";
const MAX_RESPONSE: usize = 256 * 1024;

pub struct KubernetesWorkloadVerifier {
    client: Client,
    base: Url,
    credentials: KubernetesCredentials,
    namespace: String,
}

fn rejected() -> AppError {
    AppError::Unauthorized("managed Kubernetes workload identity is invalid".into())
}

fn unavailable() -> AppError {
    AppError::NodeOffline("managed Kubernetes identity service is temporarily unavailable".into())
}

impl KubernetesWorkloadVerifier {
    /// Bounded authenticated read for explicitly requested operator diagnostics.
    /// Callers construct only validated Kubernetes resource paths.
    pub(crate) async fn read_json(&self, path: &str) -> AppResult<Value> {
        self.request(Method::GET, path, None).await
    }

    #[cfg(test)]
    pub(crate) fn for_test_endpoint(base: Url, manager_token_file: PathBuf) -> Self {
        assert!(matches!(base.host_str(), Some("127.0.0.1" | "localhost")));
        Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            base,
            credentials: KubernetesCredentials::token_file(manager_token_file).unwrap(),
            namespace: "nyxid-machine".into(),
        }
    }

    /// Explicit operator probe only; no probe is performed on ordinary proxy,
    /// MCP, or authentication paths. Unknown remains unknown.
    pub async fn operational_checks(&self) -> Vec<(&'static str, Option<bool>)> {
        let probes = [
            ("kubernetes_api", "/version"),
            ("sandbox_v1beta1_api", "/apis/agents.x-k8s.io/v1beta1"),
            (
                "namespace_quota",
                "/api/v1/namespaces/nyxid-machine/resourcequotas/remote-machine-quota",
            ),
        ];
        let mut results =
            futures::future::join_all(probes.into_iter().map(|(name, path)| async move {
                let response = tokio::time::timeout(
                    Duration::from_secs(5),
                    self.request(Method::GET, path, None),
                )
                .await;
                let present = match response {
                    Ok(Ok(value)) => Some(match name {
                        "sandbox_v1beta1_api" => {
                            value["resources"].as_array().is_some_and(|rows| {
                                rows.iter().any(|row| {
                                    row["name"] == "sandboxes" && row["kind"] == "Sandbox"
                                })
                            })
                        }
                        "namespace_quota" => value["status"]["hard"]
                            .as_object()
                            .is_some_and(|hard| !hard.is_empty()),
                        _ => value["gitVersion"].as_str().is_some(),
                    }),
                    _ => None,
                };
                (name, present)
            }))
            .await;
        let permissions = [
            (
                "storage_class_get",
                "storage.k8s.io",
                "storageclasses",
                "get",
            ),
            (
                "storage_class_list",
                "storage.k8s.io",
                "storageclasses",
                "list",
            ),
            ("sandbox_get", "agents.x-k8s.io", "sandboxes", "get"),
            ("sandbox_create", "agents.x-k8s.io", "sandboxes", "create"),
            ("sandbox_patch", "agents.x-k8s.io", "sandboxes", "patch"),
            ("sandbox_delete", "agents.x-k8s.io", "sandboxes", "delete"),
            ("sandbox_list", "agents.x-k8s.io", "sandboxes", "list"),
            ("pod_read", "", "pods", "get"),
            ("pod_list", "", "pods", "list"),
            ("pvc_list", "", "persistentvolumeclaims", "list"),
            ("quota_list", "", "resourcequotas", "list"),
            ("pvc_read", "", "persistentvolumeclaims", "get"),
            (
                "workload_token_review",
                "authentication.k8s.io",
                "tokenreviews",
                "create",
            ),
        ];
        results.extend(futures::future::join_all(permissions.into_iter().map(|(name, group, resource, verb)| async move {
            let attributes = if matches!(resource, "tokenreviews" | "storageclasses") {
                json!({"group":group,"resource":resource,"verb":verb})
            } else {
                json!({"namespace":self.namespace,"group":group,"resource":resource,"verb":verb})
            };
            (name, self.access_review(attributes).await)
        })).await);
        let quota = self.access_review(json!({"namespace": self.namespace, "resource":"resourcequotas", "verb":"get", "name":"remote-machine-quota"})).await;
        results.push(("quota_get", quota));
        let controller =
            match super::remote_machine_controller_health::Selection::from_environment() {
                Ok(s) => self.controller_access_review(&s).await,
                Err(_) => None,
            };
        results.push(("selected_controller_get", controller));
        results
    }

    async fn controller_access_review(
        &self,
        selection: &super::remote_machine_controller_health::Selection,
    ) -> Option<bool> {
        self.access_review(json!({"namespace":selection.namespace,"group":"apps","resource":"deployments","verb":"get","name":selection.deployment})).await
    }

    pub(crate) async fn access_review(&self, attributes: Value) -> Option<bool> {
        match tokio::time::timeout(Duration::from_secs(5), self.request(Method::POST,
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews", Some(json!({
                "apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":{"resourceAttributes":attributes}
            })))).await {
            Ok(Ok(value)) if value["status"]["evaluationError"].as_str().unwrap_or("").is_empty() => value["status"]["allowed"].as_bool(),
            _ => None,
        }
    }

    pub fn from_environment() -> AppResult<Self> {
        let connection =
            super::remote_machine_kubernetes_auth::KubernetesConnection::from_environment(
                Duration::from_secs(15),
            )
            .map_err(|_| {
                AppError::ValidationError(
                    "invalid managed Kubernetes identity configuration".into(),
                )
            })?;
        Ok(Self {
            client: connection.client,
            base: connection.base,
            credentials: connection.credentials,
            namespace: "nyxid-machine".into(),
        })
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> AppResult<Value> {
        let request = self
            .client
            .request(method, self.base.join(path).map_err(|_| rejected())?);
        let mut request = self
            .credentials
            .authorize(request)
            .await
            .map_err(|_| unavailable())?;
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| unavailable())?;
        if response.status().is_server_error()
            || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(unavailable());
        }
        if !response.status().is_success() {
            return Err(rejected());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err(rejected());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| rejected())
    }

    fn reviewed_pod(&self, review: &Value, binding: &Binding) -> AppResult<String> {
        let status = &review["status"];
        let expected_user = format!(
            "system:serviceaccount:{}:{WORKLOAD_ACCOUNT}",
            self.namespace
        );
        if status["authenticated"] != true
            || status["user"]["username"].as_str() != Some(&expected_user)
            || !status["audiences"]
                .as_array()
                .is_some_and(|values| values.iter().any(|v| v == AUDIENCE))
        {
            return Err(rejected());
        }
        let extra = &status["user"]["extra"];
        let one = |key: &str| -> AppResult<String> {
            let values = extra[key]
                .as_array()
                .filter(|v| v.len() == 1)
                .ok_or_else(rejected)?;
            values[0]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .ok_or_else(rejected)
        };
        if one("authentication.kubernetes.io/pod-uid")? != binding.pod_uid {
            return Err(rejected());
        }
        let name = one("authentication.kubernetes.io/pod-name")?;
        if name.len() > 253
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        {
            return Err(rejected());
        }
        Ok(name)
    }

    fn validate_pod(&self, pod: &Value, binding: &Binding) -> AppResult<()> {
        let metadata = &pod["metadata"];
        let labels = &metadata["labels"];
        let generation = binding.runtime_generation.to_string();
        let epoch = binding.credential_epoch.to_string();
        if metadata["uid"].as_str() != Some(&binding.pod_uid)
            || metadata["namespace"].as_str() != Some(&self.namespace)
            || !metadata["deletionTimestamp"].is_null()
            || pod["spec"]["serviceAccountName"].as_str() != Some(WORKLOAD_ACCOUNT)
            || pod["spec"]["automountServiceAccountToken"] != false
            || labels["nyxid.dev/managed"] != "true"
            || labels["nyxid.dev/machine-id"].as_str() != Some(&binding.node_id)
            || labels["nyxid.dev/runtime-generation"].as_str() != Some(&generation)
            || labels["nyxid.dev/credential-epoch"].as_str() != Some(&epoch)
            || labels["nyxid.dev/writer-fence"].as_str() != Some(&binding.writer_fence)
            || !metadata["ownerReferences"]
                .as_array()
                .is_some_and(|owners| {
                    let controllers: Vec<_> = owners
                        .iter()
                        .filter(|owner| owner["controller"] == true)
                        .collect();
                    controllers.len() == 1
                        && controllers[0]["kind"] == "Sandbox"
                        && controllers[0]["apiVersion"] == "agents.x-k8s.io/v1beta1"
                        && controllers[0]["uid"].as_str() == Some(&binding.sandbox_uid)
                })
        {
            return Err(rejected());
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl BootstrapTicketVerifier for KubernetesWorkloadVerifier {
    async fn verify(
        &self,
        ticket: &Value,
        binding: &Binding,
    ) -> AppResult<VerifiedBootstrapTicket> {
        binding.validate().map_err(|_| rejected())?;
        let token = ticket["token"]
            .as_str()
            .filter(|t| !t.is_empty() && t.len() <= 48 * 1024)
            .ok_or_else(rejected)?;
        let digest_text = ticket["recipient_key_digest"]
            .as_str()
            .ok_or_else(rejected)?;
        let recipient_digest: [u8; 32] = hex::decode(digest_text)
            .map_err(|_| rejected())?
            .try_into()
            .map_err(|_| rejected())?;
        if hex::encode(recipient_digest) != digest_text {
            return Err(rejected());
        }
        // The token issuer verifies the exact audience and Pod-bound token.
        // Never trust decoded JWT claims before successful TokenReview.
        let review = self
            .request(
                Method::POST,
                "/apis/authentication.k8s.io/v1/tokenreviews",
                Some(json!({
                    "apiVersion":"authentication.k8s.io/v1", "kind":"TokenReview",
                    "spec":{"token":token, "audiences":[AUDIENCE]}
                })),
            )
            .await?;
        let pod_name = self.reviewed_pod(&review, binding)?;
        let pod = self
            .request(
                Method::GET,
                &format!("/api/v1/namespaces/{}/pods/{pod_name}", self.namespace),
                None,
            )
            .await?;
        self.validate_pod(&pod, binding)?;
        let payload = token.split('.').nth(1).ok_or_else(rejected)?;
        let decoded = URL_SAFE_NO_PAD.decode(payload).map_err(|_| rejected())?;
        let claims: Value = serde_json::from_slice(&decoded).map_err(|_| rejected())?;
        let now = Utc::now();
        let expiry =
            chrono::DateTime::from_timestamp(claims["exp"].as_i64().ok_or_else(rejected)?, 0)
                .ok_or_else(rejected)?;
        if expiry <= now {
            return Err(rejected());
        }
        Ok(VerifiedBootstrapTicket::from_provider_verified_claims(
            binding.clone(),
            recipient_digest,
            expiry.min(now + chrono::Duration::minutes(2)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn observer_ssar_binds_exact_controller_and_rejects_evaluation_errors() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let server = MockServer::start().await;
        let token = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(token.path(), "local-manager-token").unwrap();
        let verifier = KubernetesWorkloadVerifier::for_test_endpoint(
            server.uri().parse().unwrap(),
            token.path().to_path_buf(),
        );
        let selected = super::super::remote_machine_controller_health::Selection::parse(
            Some("controller-system".into()),
            Some("sandbox-controller".into()),
            Some(format!("sha256:{}", "a".repeat(64))),
            None,
        )
        .unwrap();
        let attrs = json!({"namespace":"controller-system","group":"apps","resource":"deployments","verb":"get","name":"sandbox-controller"});
        for (status, expected) in [
            (json!({"allowed":true}), Some(true)),
            (json!({"allowed":false}), Some(false)),
            (
                json!({"allowed":true,"evaluationError":"RBAC authorizer unavailable"}),
                None,
            ),
        ] {
            server.reset().await;
            Mock::given(method("POST")).and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
                .and(body_json(json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":{"resourceAttributes":attrs}})))
                .respond_with(ResponseTemplate::new(201).set_body_json(json!({"status":status}))).expect(1)
                .mount(&server).await;
            assert_eq!(verifier.controller_access_review(&selected).await, expected);
            server.verify().await;
        }
        assert!(
            super::super::remote_machine_controller_health::Selection::parse(
                Some("../other".into()),
                Some("sandbox-controller".into()),
                Some(format!("sha256:{}", "a".repeat(64))),
                None
            )
            .is_err()
        );
    }

    fn fixture() -> (KubernetesWorkloadVerifier, Binding, Value, Value) {
        let id = || uuid::Uuid::new_v4().to_string();
        let binding = Binding {
            node_id: id(),
            sandbox_uid: id(),
            pod_uid: id(),
            runtime_generation: 2,
            credential_epoch: 3,
            writer_fence: id(),
            operation_id: id(),
            delivery_id: id(),
        };
        let verifier = KubernetesWorkloadVerifier {
            client: Client::new(),
            base: "https://kubernetes.test".parse().unwrap(),
            credentials: KubernetesCredentials::token_file(PathBuf::from("/unused")).unwrap(),
            namespace: "nyxid-machine".into(),
        };
        let review = json!({"status":{"authenticated":true,"audiences":[AUDIENCE],"user":{
            "username":"system:serviceaccount:nyxid-machine:nyxid-machine-workload","extra":{
                "authentication.kubernetes.io/pod-name":["machine-pod"],"authentication.kubernetes.io/pod-uid":[binding.pod_uid]}}}});
        let pod = json!({"metadata":{"uid":binding.pod_uid,"namespace":"nyxid-machine","labels":{
            "nyxid.dev/managed":"true","nyxid.dev/machine-id":binding.node_id,"nyxid.dev/runtime-generation":"2",
            "nyxid.dev/credential-epoch":"3","nyxid.dev/writer-fence":binding.writer_fence},
            "ownerReferences":[{"kind":"Sandbox","apiVersion":"agents.x-k8s.io/v1beta1","uid":binding.sandbox_uid,"controller":true}]},
            "spec":{"serviceAccountName":WORKLOAD_ACCOUNT,"automountServiceAccountToken":false}});
        (verifier, binding, review, pod)
    }
    #[test]
    fn kubernetes_identity_requires_pod_bound_audience_and_live_exact_owner() {
        let (verifier, binding, review, pod) = fixture();
        assert_eq!(
            verifier.reviewed_pod(&review, &binding).unwrap(),
            "machine-pod"
        );
        verifier.validate_pod(&pod, &binding).unwrap();
        for pointer in [
            "/status/authenticated",
            "/status/audiences",
            "/status/user/username",
            "/status/user/extra/authentication.kubernetes.io~1pod-uid",
        ] {
            let mut changed = review.clone();
            *changed.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(
                verifier.reviewed_pod(&changed, &binding).is_err(),
                "{pointer}"
            );
        }
        for pointer in [
            "/metadata/uid",
            "/metadata/ownerReferences/0/uid",
            "/metadata/labels/nyxid.dev~1writer-fence",
            "/metadata/labels/nyxid.dev~1machine-id",
            "/metadata/labels/nyxid.dev~1runtime-generation",
            "/metadata/labels/nyxid.dev~1credential-epoch",
            "/spec/serviceAccountName",
        ] {
            let mut changed = pod.clone();
            *changed.pointer_mut(pointer).unwrap() = json!("foreign");
            assert!(
                verifier.validate_pod(&changed, &binding).is_err(),
                "{pointer}"
            );
        }
        let mut duplicate = pod.clone();
        duplicate["metadata"]["ownerReferences"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"Sandbox","controller":true,"uid":"foreign"}));
        assert!(verifier.validate_pod(&duplicate, &binding).is_err());
        let mut deleting = pod;
        deleting["metadata"]["deletionTimestamp"] = json!(Utc::now().to_rfc3339());
        assert!(verifier.validate_pod(&deleting, &binding).is_err());
    }
    #[tokio::test]
    async fn tokenreview_transport_prepares_and_seals_only_the_live_pod_recipient() {
        use super::super::remote_machine_bootstrap::{self as bootstrap, BootstrapInput};
        use axum::{
            Json, Router,
            http::HeaderMap,
            routing::{get, post},
        };
        let (db, keys, binding, recipient) = bootstrap::tests::fixture().await;
        let (_, _, mut review, mut pod) = fixture();
        review["status"]["user"]["extra"]["authentication.kubernetes.io/pod-uid"] =
            json!([binding.pod_uid]);
        pod["metadata"]["uid"] = json!(binding.pod_uid);
        pod["metadata"]["labels"]["nyxid.dev/machine-id"] = json!(binding.node_id);
        pod["metadata"]["labels"]["nyxid.dev/runtime-generation"] =
            json!(binding.runtime_generation.to_string());
        pod["metadata"]["labels"]["nyxid.dev/credential-epoch"] =
            json!(binding.credential_epoch.to_string());
        pod["metadata"]["labels"]["nyxid.dev/writer-fence"] = json!(binding.writer_fence);
        pod["metadata"]["ownerReferences"][0]["uid"] = json!(binding.sandbox_uid);
        let token = format!(
            "test.{}.fixture",
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(
                    &json!({"exp":(Utc::now()+chrono::Duration::minutes(5)).timestamp()})
                )
                .unwrap()
            )
        );
        let expected_token = token.clone();
        let denied = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
        let denied_route = denied.clone();
        let app = Router::new()
            .route(
                "/apis/authentication.k8s.io/v1/tokenreviews",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let review = review.clone();
                    let expected_token = expected_token.clone();
                    let denied = denied_route.clone();
                    async move {
                        assert_eq!(
                            headers.get("authorization").unwrap(),
                            "Bearer fixture-manager-token"
                        );
                        assert_eq!(body["spec"]["audiences"], json!([AUDIENCE]));
                        assert_eq!(body["spec"]["token"], expected_token);
                        match denied.load(std::sync::atomic::Ordering::SeqCst) {
                            1 => (
                                axum::http::StatusCode::OK,
                                Json(json!({"status":{"authenticated":false}})),
                            ),
                            2 => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({}))),
                            _ => (axum::http::StatusCode::OK, Json(review)),
                        }
                    }
                }),
            )
            .route(
                "/api/v1/namespaces/nyxid-machine/pods/machine-pod",
                get(move |headers: HeaderMap| {
                    let pod = pod.clone();
                    async move {
                        assert_eq!(
                            headers.get("authorization").unwrap(),
                            "Bearer fixture-manager-token"
                        );
                        Json(pod)
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let token_path =
            std::env::temp_dir().join(format!("nyxid-kube-token-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&token_path, "fixture-manager-token")
            .await
            .unwrap();
        // HTTP construction is test-only; configured production clients require HTTPS.
        let verifier = KubernetesWorkloadVerifier {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            base: format!("http://{address}").parse().unwrap(),
            credentials: KubernetesCredentials::token_file(token_path.clone()).unwrap(),
            namespace: "nyxid-machine".into(),
        };
        let public = recipient.public_key_b64u().unwrap();
        let digest = hex::encode(nyxid_machine::bootstrap::recipient_key_digest(&public).unwrap());
        let ticket = json!({"token":token,"recipient_key_digest":digest});
        let prepared = bootstrap::prepare_workload(
            &db,
            &verifier,
            &binding.node_id,
            &binding.pod_uid,
            &ticket,
            &public,
        )
        .await
        .unwrap();
        let result = bootstrap::bootstrap(
            &db,
            &keys,
            &verifier,
            BootstrapInput {
                binding: prepared.clone(),
                ticket: ticket.clone(),
                recipient_public_key: public.clone(),
            },
        )
        .await
        .unwrap();
        assert!(!result.replayed);
        let replay = bootstrap::bootstrap(
            &db,
            &keys,
            &verifier,
            BootstrapInput {
                binding: prepared.clone(),
                ticket: ticket.clone(),
                recipient_public_key: public.clone(),
            },
        )
        .await
        .unwrap();
        assert!(replay.replayed);
        assert_eq!(result.sealed, replay.sealed);
        let other = nyxid_machine::bootstrap::generate_recipient().unwrap();
        assert!(
            bootstrap::prepare_workload(
                &db,
                &verifier,
                &binding.node_id,
                &binding.pod_uid,
                &ticket,
                &other.public_key_b64u().unwrap()
            )
            .await
            .is_err()
        );
        denied.store(2, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            bootstrap::prepare_workload(
                &db,
                &verifier,
                &binding.node_id,
                &binding.pod_uid,
                &ticket,
                &public
            )
            .await,
            Err(AppError::NodeOffline(_))
        ));
        denied.store(1, std::sync::atomic::Ordering::SeqCst);
        assert!(
            bootstrap::prepare_workload(
                &db,
                &verifier,
                &binding.node_id,
                &binding.pod_uid,
                &ticket,
                &public
            )
            .await
            .is_err()
        );
        server.abort();
        tokio::fs::remove_file(token_path).await.unwrap();
        db.drop().await.unwrap();
    }
}
