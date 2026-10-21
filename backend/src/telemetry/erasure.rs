//! Authenticated PostHog erasure API. Capture/ingest acceptance is never erasure.
use futures::StreamExt;
use serde::Deserialize;
use std::{collections::BTreeMap, time::Duration};
use zeroize::Zeroizing;

const MAX_RESPONSE: usize = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DestinationInput {
    id: String,
    api_host: String,
    project_id: String,
    personal_api_key: String,
}
impl Drop for DestinationInput {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.personal_api_key.zeroize();
    }
}

pub struct Destination {
    pub id: String,
    pub api_host: String,
    pub project_id: String,
    key: Zeroizing<String>,
    http: reqwest::Client,
}
crate::errors::redacted_debug!(Destination);

#[derive(Default)]
pub struct ErasureClients(pub BTreeMap<String, Destination>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorError {
    Transport,
    Rejected,
    ResponseInvalid,
    ResponseTooLarge,
    AmbiguousPerson,
}

impl ErasureClients {
    /// Explicitly list current AND historical destinations. Secret API keys
    /// stay in configuration, never in durable jobs or diagnostics.
    pub fn from_env() -> Result<Self, &'static str> {
        let Ok(value) = std::env::var("NYXID_TELEMETRY_ERASURE_DESTINATIONS") else {
            return Ok(Self::default());
        };
        Self::parse(Zeroizing::new(value))
    }
    fn parse(value: Zeroizing<String>) -> Result<Self, &'static str> {
        let entries: Vec<DestinationInput> = serde_json::from_str(&value)
            .map_err(|_| "Invalid telemetry erasure destinations configuration")?;
        if entries.len() > 16 {
            return Err("Too many telemetry erasure destinations");
        }
        let mut clients = BTreeMap::new();
        for entry in entries {
            let url = url::Url::parse(&entry.api_host).map_err(|_| "Invalid erasure API host")?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err("Erasure API host must be an HTTPS origin without credentials");
            }
            if entry.id == "unresolved"
                || entry.id.is_empty()
                || entry.id.len() > 64
                || !entry
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || entry.project_id.is_empty()
                || entry.project_id.len() > 20
                || !entry.project_id.bytes().all(|b| b.is_ascii_digit())
                || entry.personal_api_key.is_empty()
                || entry.personal_api_key.len() > 1024
            {
                return Err("Invalid erasure destination identifier or credential");
            }
            let destination = Destination {
                id: entry.id.clone(),
                api_host: url.as_str().trim_end_matches('/').into(),
                project_id: entry.project_id.clone(),
                key: Zeroizing::new(entry.personal_api_key.clone()),
                http: reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|_| "Erasure HTTP initialization failed")?,
            };
            if clients.insert(entry.id.clone(), destination).is_some() {
                return Err("Duplicate erasure destination identifier");
            }
        }
        Ok(Self(clients))
    }
}

pub struct DeletionStatus {
    pub markers: Vec<String>,
    pub completed: bool,
    pub action_required: bool,
}

impl Destination {
    fn persons_url(&self) -> String {
        format!("{}/api/projects/{}/persons", self.api_host, self.project_id)
    }
    async fn json(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<serde_json::Value, VendorError> {
        let response = request
            .bearer_auth(self.key.as_str())
            .send()
            .await
            .map_err(|_| VendorError::Transport)?;
        if !response.status().is_success() {
            return Err(VendorError::Rejected);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE as u64)
        {
            return Err(VendorError::ResponseTooLarge);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| VendorError::Transport)?;
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                return Err(VendorError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| VendorError::ResponseInvalid)
    }
    pub async fn lookup(&self, distinct_id: &str) -> Result<Option<String>, VendorError> {
        let value = self
            .json(
                self.http
                    .get(format!("{}/", self.persons_url()))
                    .query(&[("distinct_id", distinct_id)]),
            )
            .await?;
        let results = value
            .get("results")
            .and_then(|v| v.as_array())
            .ok_or(VendorError::ResponseInvalid)?;
        if results.len() > 1 || value.get("next").is_some_and(|v| !v.is_null()) {
            return Err(VendorError::AmbiguousPerson);
        }
        let Some(row) = results.first() else {
            return Ok(None);
        };
        let id = row
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or(VendorError::ResponseInvalid)?;
        uuid::Uuid::parse_str(id).map_err(|_| VendorError::ResponseInvalid)?;
        Ok(Some(id.into()))
    }
    /// The single-person API removes the person/recordings and returns empty
    /// 202; event deletion remains asynchronous and must be polled separately.
    pub async fn request_delete(&self, person: &str) -> Result<(), VendorError> {
        uuid::Uuid::parse_str(person).map_err(|_| VendorError::ResponseInvalid)?;
        let response = self
            .http
            .delete(format!("{}/{}", self.persons_url(), person))
            .query(&[("delete_events", "true"), ("delete_recordings", "true")])
            .bearer_auth(self.key.as_str())
            .send()
            .await
            .map_err(|_| VendorError::Transport)?;
        if response.status() == reqwest::StatusCode::ACCEPTED {
            Ok(())
        } else {
            Err(VendorError::Rejected)
        }
    }
    pub async fn deletion_status(
        &self,
        person: &str,
        baseline: Option<&[String]>,
    ) -> Result<DeletionStatus, VendorError> {
        let value = self
            .json(
                self.http
                    .get(format!("{}/deletion_status", self.persons_url()))
                    .query(&[("person_uuid", person), ("status", "all")]),
            )
            .await?;
        let rows = value
            .as_array()
            .or_else(|| value.get("results").and_then(|v| v.as_array()))
            .ok_or(VendorError::ResponseInvalid)?;
        if rows.len() > 100 || value.get("next").is_some_and(|v| !v.is_null()) {
            return Err(VendorError::ResponseInvalid);
        }
        let mut markers = Vec::new();
        let mut found = false;
        let mut completed = true;
        let mut action_required = false;
        for row in rows {
            if row.get("person_uuid").and_then(|v| v.as_str()) != Some(person) {
                return Err(VendorError::ResponseInvalid);
            }
            let created = row
                .get("created_at")
                .and_then(|v| v.as_str())
                .filter(|v| v.len() <= 64)
                .ok_or(VendorError::ResponseInvalid)?;
            chrono::DateTime::parse_from_rfc3339(created)
                .map_err(|_| VendorError::ResponseInvalid)?;
            markers.push(created.into());
            if baseline.is_none_or(|prior| prior.iter().any(|old| old == created)) {
                continue;
            }
            found = true;
            // The documented states are pending/completed. A provider failure
            // or an unknown state must be visible for reconciliation rather
            // than masquerade forever as a successfully accepted deletion.
            if !matches!(
                row.get("status").and_then(|v| v.as_str()),
                Some("pending" | "completed")
            ) {
                action_required = true;
            }
            if row.get("status").and_then(|v| v.as_str()) != Some("completed")
                || row
                    .get("delete_verified_at")
                    .and_then(|v| v.as_str())
                    .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                    .is_none()
            {
                completed = false;
            }
        }
        Ok(DeletionStatus {
            markers,
            completed: found && completed,
            action_required,
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, Method, StatusCode, Uri},
        response::IntoResponse,
        routing::any,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    pub const PERSON: &str = "550e8400-e29b-41d4-a716-446655440099";
    #[derive(Default)]
    pub struct Vendor {
        pub lookups: AtomicUsize,
        pub deletes: AtomicUsize,
        pub verified: AtomicBool,
        pub missing_person: AtomicBool,
        pub fail_status: AtomicBool,
        pub deletion_failed: AtomicBool,
        pub hold_capture: AtomicBool,
        pub capture_started: tokio::sync::Notify,
        pub capture_release: tokio::sync::Notify,
    }
    pub struct Server {
        pub state: Arc<Vendor>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn handle(
        State(state): State<Arc<Vendor>>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
    ) -> axum::response::Response {
        if uri.path() == "/capture/" {
            state.capture_started.notify_one();
            if state.hold_capture.load(Ordering::SeqCst) {
                state.capture_release.notified().await;
            }
            // A late provider receipt recreates the person unless deletion
            // waits for this transport to settle.
            state.missing_person.store(false, Ordering::SeqCst);
            return StatusCode::OK.into_response();
        }
        if headers.get("authorization").and_then(|v| v.to_str().ok())
            != Some("Bearer distinctive-personal-api-key")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        if uri.path().ends_with("/deletion_status") {
            assert_eq!(method, Method::GET);
            assert!(uri.query().unwrap().contains("status=all"));
            if state.fail_status.load(Ordering::SeqCst) {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            let mut rows = vec![
                serde_json::json!({"person_uuid":PERSON,"created_at":"2026-01-01T00:00:00Z","status":"completed","delete_verified_at":"2026-01-01T01:00:00Z"}),
            ];
            if state.deletes.load(Ordering::SeqCst) > 0 {
                let verified = state.verified.load(Ordering::SeqCst);
                rows.push(serde_json::json!({"person_uuid":PERSON,"created_at":format!("2026-01-{:02}T00:00:00Z", state.deletes.load(Ordering::SeqCst)+1),"status":if state.deletion_failed.load(Ordering::SeqCst){"failed"}else if verified{"completed"}else{"pending"},"delete_verified_at":if verified{Some("2026-01-02T01:00:00Z")}else{None}}));
            }
            return Json(serde_json::json!(rows)).into_response();
        }
        if method == Method::DELETE {
            assert!(uri.path().ends_with(PERSON));
            let params = uri.query().unwrap();
            assert!(
                params.contains("delete_events=true") && params.contains("delete_recordings=true")
            );
            state.deletes.fetch_add(1, Ordering::SeqCst);
            state.missing_person.store(true, Ordering::SeqCst);
            return StatusCode::ACCEPTED.into_response();
        }
        assert_eq!(method, Method::GET);
        assert!(uri.query().unwrap().starts_with("distinct_id="));
        state.lookups.fetch_add(1, Ordering::SeqCst);
        Json(serde_json::json!({"results":if state.missing_person.load(Ordering::SeqCst){vec![]}else{vec![serde_json::json!({"id":PERSON})]},"next":null})).into_response()
    }
    pub async fn clients() -> (ErasureClients, Server) {
        let state = Arc::new(Vendor::default());
        let app = Router::new()
            .route("/{*path}", any(handle))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let destination = Destination {
            id: "historical-project".into(),
            api_host: host,
            project_id: "123".into(),
            key: Zeroizing::new("distinctive-personal-api-key".into()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        };
        (
            ErasureClients(BTreeMap::from([(destination.id.clone(), destination)])),
            Server { state, task },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_reserves_unresolved_parent_identifier() {
        let input = r#"[{"id":"unresolved","api_host":"https://example.test","project_id":"1","personal_api_key":"distinctive-test-credential"}]"#;
        let error = ErasureClients::parse(Zeroizing::new(input.into()))
            .err()
            .unwrap();
        assert!(!error.contains("distinctive-test-credential"));
    }
    #[test]
    fn configuration_rejects_unsafe_destinations_without_echoing_input() {
        let input = r#"[{"id":"test","api_host":"https://user:distinctive-secret@example.test/","project_id":"1","personal_api_key":"distinctive-secret"}]"#;
        let error = ErasureClients::parse(Zeroizing::new(input.into()))
            .err()
            .unwrap();
        assert!(!error.contains("distinctive-secret"));
        for host in [
            "http://example.test",
            "https://example.test/path",
            "https://example.test/?q=secret",
            "https://example.test/#fragment",
        ] {
            let input = serde_json::json!([{"id":"test","api_host":host,"project_id":"1","personal_api_key":"key"}]);
            assert!(ErasureClients::parse(Zeroizing::new(input.to_string())).is_err());
        }
        assert!(
            ErasureClients::parse(Zeroizing::new("[]".into()))
                .unwrap()
                .0
                .is_empty()
        );
    }
    #[tokio::test]
    async fn authenticated_delete_is_accepted_until_new_async_status_is_verified() {
        use std::sync::atomic::Ordering;
        let (clients, server) = test_support::clients().await;
        let destination = clients.0.values().next().unwrap();
        assert!(!format!("{destination:?}").contains("distinctive-personal-api-key"));
        let person = destination.lookup("account-id").await.unwrap().unwrap();
        let baseline = destination.deletion_status(&person, None).await.unwrap();
        assert!(
            !baseline.completed,
            "old completed requests cannot prove a fresh deletion"
        );
        destination.request_delete(&person).await.unwrap();
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 1);
        assert!(
            !destination
                .deletion_status(&person, Some(&baseline.markers))
                .await
                .unwrap()
                .completed
        );
        server.state.verified.store(true, Ordering::SeqCst);
        assert!(
            destination
                .deletion_status(&person, Some(&baseline.markers))
                .await
                .unwrap()
                .completed
        );
        server.state.fail_status.store(true, Ordering::SeqCst);
        assert_eq!(
            destination
                .deletion_status(&person, Some(&baseline.markers))
                .await
                .err(),
            Some(VendorError::Rejected)
        );
    }
}
