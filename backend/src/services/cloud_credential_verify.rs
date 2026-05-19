//! At-creation credential verification for cloud-billing auth methods.
//!
//! When a user adds a key for `aws_sigv4`, make one cheap probe call
//! upstream so we can fail-fast with a useful error message if the
//! credential is malformed, attached to the wrong account, or missing
//! required IAM grants. Without this the user only discovers
//! misconfiguration when a `/daily` skill runs hours later and the
//! proxy returns the cloud's raw `AccessDenied` blob.
//!
//! Probe choice per auth method:
//!
//! - **AWS Cost Explorer** (`aws_sigv4` + `ce.us-east-1.amazonaws.com`):
//!   `GetCostAndUsage` for a one-day window. Returns 200 with empty
//!   results if the credential is OK but the account has no usage;
//!   returns 4xx with `AccessDenied` if IAM is wrong or the credential
//!   is from a linked account (Cost Explorer can only be called from
//!   the management/payer account).
//!
//! A 5xx or network failure is treated as "couldn't verify, but
//! credential is plausibly fine" and the add proceeds with a warning
//! logged. That preserves the credential-store add when AWS is itself
//! flaky.

use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use nyxid_cloud_auth::aws_sigv4::{self, AwsCredentials};

use crate::errors::{AppError, AppResult};

/// How long to wait on the probe before giving up. Keep short so a
/// flaky cloud doesn't block credential creation indefinitely.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(8);

/// Probe an `aws_sigv4` credential against the catalog endpoint.
///
/// On 401/403/permission-shaped 400s: returns
/// [`AppError::BadRequest`] with a message that names the IAM policy
/// the user likely forgot. On 5xx or network failure: logs a warning
/// and returns `Ok(())` so a transient AWS outage doesn't block adds.
pub async fn verify_aws_sigv4(
    http_client: &reqwest::Client,
    credential_json: &str,
    base_url: &str,
) -> AppResult<()> {
    let creds = AwsCredentials::from_json(credential_json).map_err(|e| {
        AppError::BadRequest(format!(
            "AWS credential JSON is malformed: {e}. Expected fields: \
             access_key_id, secret_access_key, region, service."
        ))
    })?;

    // One-day window ending today. The smallest possible probe body.
    let end = Utc::now().date_naive();
    let start = end - ChronoDuration::days(1);
    let body = serde_json::json!({
        "TimePeriod": { "Start": start.to_string(), "End": end.to_string() },
        "Granularity": "DAILY",
        "Metrics": ["UnblendedCost"],
    })
    .to_string();
    let body_bytes = body.as_bytes();

    let probe_url = base_url.trim_end_matches('/').to_string() + "/";
    let headers = vec![
        (
            "content-type".to_string(),
            "application/x-amz-json-1.1".to_string(),
        ),
        (
            "x-amz-target".to_string(),
            "AWSInsightsServiceV20210101.GetCostAndUsage".to_string(),
        ),
    ];
    let signed = aws_sigv4::sign_request("POST", &probe_url, &headers, body_bytes, &creds)
        .map_err(|e| AppError::Internal(format!("AWS verify: SigV4 sign failed: {e}")))?;

    let mut req = http_client.post(&probe_url).body(body.clone());
    for (n, v) in &headers {
        req = req.header(n, v);
    }
    for h in &signed {
        req = req.header(&h.name, &h.value);
    }

    let response = match tokio::time::timeout(VERIFY_TIMEOUT, req.send()).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!(
                error = %e,
                base_url = %base_url,
                "AWS credential verify network error; allowing add"
            );
            return Ok(());
        }
        Err(_) => {
            tracing::warn!(
                base_url = %base_url,
                timeout = ?VERIFY_TIMEOUT,
                "AWS credential verify timed out; allowing add"
            );
            return Ok(());
        }
    };

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status.is_server_error() {
        tracing::warn!(
            status = %status,
            base_url = %base_url,
            "AWS credential verify hit 5xx; allowing add"
        );
        return Ok(());
    }

    // 4xx — capture a bounded slice of the body so the error message
    // points the user at the actual cause without dumping a huge XML
    // response into the log.
    let body = response.text().await.unwrap_or_default();
    let snippet: String = body.chars().take(400).collect();
    let hint = if body.contains("not authorized") || body.contains("AccessDenied") {
        " — verify the IAM policy includes ce:GetCostAndUsage and that this is a credential from the AWS Organizations management (payer) account, not a linked account."
    } else if body.contains("InvalidSignatureException") {
        " — the signature didn't match; double-check the secret_access_key is correct and matches the access_key_id."
    } else {
        ""
    };
    Err(AppError::BadRequest(format!(
        "AWS rejected the credential ({status}){hint} Response: {snippet}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    #[derive(Clone, Default)]
    struct MockState {
        hits: Arc<AtomicUsize>,
        response_status: Arc<std::sync::Mutex<u16>>,
        response_body: Arc<std::sync::Mutex<String>>,
    }

    async fn mock_handler(State(state): State<MockState>) -> impl IntoResponse {
        state.hits.fetch_add(1, Ordering::SeqCst);
        let status = *state.response_status.lock().unwrap();
        let body = state.response_body.lock().unwrap().clone();
        (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body,
        )
    }

    async fn start_aws_mock(status: u16, body: &str) -> (String, MockState) {
        let state = MockState {
            hits: Arc::new(AtomicUsize::new(0)),
            response_status: Arc::new(std::sync::Mutex::new(status)),
            response_body: Arc::new(std::sync::Mutex::new(body.to_string())),
        };
        let app = Router::new()
            .route("/", post(mock_handler))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (format!("http://{addr}"), state)
    }

    fn valid_aws_credential() -> String {
        r#"{"access_key_id":"AKIDEXAMPLE","secret_access_key":"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY","region":"us-east-1","service":"ce"}"#.to_string()
    }

    #[tokio::test]
    async fn aws_verify_accepts_2xx_response() {
        let (base_url, mock) = start_aws_mock(200, "{}").await;
        let client = reqwest::Client::new();
        let result = verify_aws_sigv4(&client, &valid_aws_credential(), &base_url).await;
        assert!(result.is_ok(), "expected Ok for 2xx, got {result:?}");
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn aws_verify_hard_fails_on_403_with_iam_hint() {
        let (base_url, _) = start_aws_mock(
            403,
            "{\"__type\":\"AccessDeniedException\",\"Message\":\"User is not authorized to perform: ce:GetCostAndUsage\"}",
        )
        .await;
        let client = reqwest::Client::new();
        let err = verify_aws_sigv4(&client, &valid_aws_credential(), &base_url)
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("403"), "missing status code in error: {msg}");
        assert!(
            msg.contains("ce:GetCostAndUsage")
                || msg.contains("management")
                || msg.contains("payer"),
            "missing IAM hint in error: {msg}"
        );
    }

    #[tokio::test]
    async fn aws_verify_soft_warns_on_503() {
        let (base_url, _) = start_aws_mock(503, "<html>backend overloaded</html>").await;
        let client = reqwest::Client::new();
        let result = verify_aws_sigv4(&client, &valid_aws_credential(), &base_url).await;
        // 5xx should NOT block credential creation — we treat it as
        // "AWS itself is flaky, allow the add".
        assert!(result.is_ok(), "expected soft-warn on 5xx, got {result:?}");
    }

    #[tokio::test]
    async fn aws_verify_rejects_malformed_credential_json_without_hitting_network() {
        let (base_url, mock) = start_aws_mock(200, "{}").await;
        let client = reqwest::Client::new();
        let err = verify_aws_sigv4(&client, "not even json", &base_url)
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("malformed"),
            "expected malformed-credential error, got: {msg}"
        );
        assert_eq!(
            mock.hits.load(Ordering::SeqCst),
            0,
            "should not hit AWS for a malformed credential"
        );
    }
}
