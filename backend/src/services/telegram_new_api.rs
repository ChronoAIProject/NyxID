use serde_json::{Value, json};
use std::time::Duration;
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};
pub use crate::models::telegram_bot_request::{
    TelegramDiagnostic, TelegramFailure, TelegramReason,
};

/// Preserve ordinary reqwest redirect behavior, but never redirect an ORIGINAL
/// Bot API request: /bot{nonempty credential}/{single nonempty method}. The
/// original URL (previous[0]) determines the policy, including local API fixtures;
/// an ordinary request redirected toward a Bot-shaped URL keeps ordinary policy.
pub fn app_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let telegram = attempt.previous().first().is_some_and(|url| {
            let mut segments = url.path().split('/');
            segments.next() == Some("")
                && segments
                    .next()
                    .is_some_and(|segment| segment.starts_with("bot") && segment.len() > 3)
                && segments.next().is_some_and(|segment| !segment.is_empty())
                && segments.next().is_none()
        });
        if telegram {
            attempt.stop()
        } else {
            reqwest::redirect::Policy::limited(10).redirect(attempt)
        }
    })
}
pub const CALL_TIMEOUT_SECS: u64 = 20;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct TelegramApi<'a> {
    pub http: &'a reqwest::Client,
    pub base_url: &'a str,
}

impl TelegramDiagnostic {
    fn new(failure: TelegramFailure, status: Option<u16>, code: Option<i32>) -> Self {
        let status = status.filter(|status| (100..=599).contains(status));
        let reason = match code.and_then(|code| u16::try_from(code).ok()).or(status) {
            Some(429) => TelegramReason::RateLimited,
            Some(401) => TelegramReason::Unauthorized,
            Some(403) => TelegramReason::Forbidden,
            Some(400) => TelegramReason::BadRequest,
            Some(500..=599) => TelegramReason::ProviderUnavailable,
            _ => TelegramReason::Unknown,
        };
        Self {
            failure,
            http_status: status,
            error_code: code,
            reason,
            retry_after: None,
        }
    }

    pub fn uncertain(&self) -> bool {
        !matches!(
            self.failure,
            TelegramFailure::HttpRejected | TelegramFailure::ProviderRejected
        )
    }

    pub fn app_error(&self) -> AppError {
        AppError::ChannelPlatformError(
            "Telegram request failed; retry or check the manager configuration".into(),
        )
    }
}

impl<'a> TelegramApi<'a> {
    pub fn new(http: &'a reqwest::Client) -> Self {
        Self {
            http,
            base_url: "https://api.telegram.org",
        }
    }

    /// Returns closed, bounded diagnostics only. Provider descriptions and transport
    /// errors may contain credentials/private text and never cross this boundary.
    pub async fn detailed_call(
        &self,
        token: &str,
        method: &str,
        input: Value,
    ) -> Result<Value, TelegramDiagnostic> {
        let mut response = self
            .http
            .post(format!("{}/bot{token}/{method}", self.base_url))
            .timeout(Duration::from_secs(CALL_TIMEOUT_SECS))
            .json(&input)
            .send()
            .await
            .map_err(|error| {
                TelegramDiagnostic::new(
                    if error.is_timeout() {
                        TelegramFailure::Timeout
                    } else {
                        TelegramFailure::Network
                    },
                    None,
                    None,
                )
            })?;
        let status = response.status().as_u16();
        let rejected = !response.status().is_success();
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            TelegramDiagnostic::new(
                if error.is_timeout() {
                    TelegramFailure::Timeout
                } else {
                    TelegramFailure::InvalidResponse
                },
                Some(status),
                None,
            )
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(TelegramDiagnostic::new(
                    TelegramFailure::ResponseTooLarge,
                    Some(status),
                    None,
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut envelope: Value = serde_json::from_slice(&bytes).map_err(|_| {
            TelegramDiagnostic::new(
                if rejected {
                    TelegramFailure::HttpRejected
                } else {
                    TelegramFailure::InvalidResponse
                },
                Some(status),
                None,
            )
        })?;
        if rejected || envelope["ok"] == false {
            let code = envelope["error_code"]
                .as_i64()
                .and_then(|code| i32::try_from(code).ok())
                .filter(|code| *code > 0);
            let mut diagnostic = TelegramDiagnostic::new(
                if rejected {
                    TelegramFailure::HttpRejected
                } else {
                    TelegramFailure::ProviderRejected
                },
                Some(status),
                code,
            );
            diagnostic.retry_after = envelope["parameters"]["retry_after"]
                .as_u64()
                .filter(|seconds| (1..=86400).contains(seconds))
                .map(|seconds| seconds as u32);
            return Err(diagnostic);
        }
        if envelope["ok"] != true || envelope.get("result").is_none_or(Value::is_null) {
            return Err(TelegramDiagnostic::new(
                TelegramFailure::InvalidResponse,
                Some(status),
                None,
            ));
        }
        Ok(envelope["result"].take())
    }

    pub async fn send_prompt(&self, token: &str, input: Value) -> Result<(), TelegramDiagnostic> {
        let outcome = async {
            let result = self.detailed_call(token, "sendMessage", input).await?;
            if result["message_id"].as_i64().is_none_or(|id| id <= 0) {
                return Err(TelegramDiagnostic::new(
                    TelegramFailure::InvalidResponse,
                    Some(200),
                    None,
                ));
            }
            Ok(())
        }
        .await;
        if let Err(diagnostic) = &outcome {
            tracing::warn!(failure = ?diagnostic.failure, reason = ?diagnostic.reason,
                http_status = diagnostic.http_status, error_code = diagnostic.error_code,
                retry_after = diagnostic.retry_after, "Telegram prompt delivery unconfirmed");
        }
        outcome
    }

    pub async fn call(&self, token: &str, method: &str, input: Value) -> AppResult<Value> {
        self.detailed_call(token, method, input)
            .await
            .map_err(|diagnostic| diagnostic.app_error())
    }

    pub async fn token(&self, manager: &str, bot_id: i64) -> AppResult<Zeroizing<String>> {
        match self
            .call(manager, "getManagedBotToken", json!({"user_id": bot_id}))
            .await?
        {
            Value::String(value) if !value.is_empty() => Ok(Zeroizing::new(value)),
            _ => Err(AppError::ChannelPlatformError(
                "Telegram returned an invalid bot token".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    #[derive(Clone)]
    struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for LogCapture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn telegram_new_diagnostics_are_bounded_closed_and_secret_free() {
        let captured = LogCapture(Default::default());
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let server = MockServer::start().await;
        let client = reqwest::Client::builder()
            .redirect(app_redirect_policy())
            .build()
            .unwrap();
        let base = server.uri();
        let api = TelegramApi {
            http: &client,
            base_url: &base,
        };
        for (http, code, reason) in [
            (400, 400, TelegramReason::BadRequest),
            (401, 401, TelegramReason::Unauthorized),
            (403, 403, TelegramReason::Forbidden),
            (429, 429, TelegramReason::RateLimited),
            (503, 503, TelegramReason::ProviderUnavailable),
            (200, 400, TelegramReason::BadRequest),
            (600, 600, TelegramReason::Unknown),
            (700, 700, TelegramReason::Unknown),
        ] {
            server.reset().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(http).set_body_json(json!({"ok": false,"error_code":code,"description":"PRIVATE-SENTINEL botTOKEN-SENTINEL","parameters":{"retry_after":15}}))).mount(&server).await;
            let diagnostic = api
                .send_prompt("TOKEN-SENTINEL", json!({"text":"PRIVATE-SENTINEL"}))
                .await
                .unwrap_err();
            assert_eq!(diagnostic.reason, reason);
            assert_eq!(
                diagnostic.http_status,
                (100..=599).contains(&http).then_some(http)
            );
            assert_eq!(diagnostic.error_code, Some(code));
            assert_eq!(diagnostic.retry_after, Some(15));
            assert!(!diagnostic.uncertain());
            let exposed = format!("{diagnostic:?} {}", diagnostic.app_error());
            assert!(!exposed.contains("SENTINEL"));
            assert!(!exposed.contains(&base));
        }
        for body in [
            json!({"ok":true}),
            json!({"ok":true,"result":{}}),
            json!({"ok":true,"result":{"message_id":0}}),
            json!({"ok":"true","result":{"message_id":2}}),
        ] {
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            assert_eq!(
                api.send_prompt("TOKEN-SENTINEL", json!({}))
                    .await
                    .unwrap_err()
                    .failure,
                TelegramFailure::InvalidResponse
            );
        }
        for status in [200, 400] {
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string("x".repeat(MAX_RESPONSE_BYTES + 1)),
                )
                .mount(&server)
                .await;
            assert_eq!(
                api.send_prompt("TOKEN-SENTINEL", json!({}))
                    .await
                    .unwrap_err()
                    .failure,
                TelegramFailure::ResponseTooLarge
            );
        }
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":1}})),
            )
            .mount(&server)
            .await;
        api.send_prompt("TOKEN-SENTINEL", json!({})).await.unwrap();
        let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("Telegram prompt delivery unconfirmed"));
        assert!(logs.contains("http_status=429"));
        assert!(!logs.contains("SENTINEL"));
        assert!(!logs.contains(&base));
    }

    #[tokio::test]
    async fn telegram_new_injected_app_client_preserves_headers_and_refuses_307_308() {
        let server = MockServer::start().await;
        let target = MockServer::start().await;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-telegram-fixture-client", "configured".parse().unwrap());
        let client = reqwest::Client::builder()
            .redirect(app_redirect_policy())
            .default_headers(headers)
            .build()
            .unwrap();
        let base = server.uri();
        let api = TelegramApi {
            http: &client,
            base_url: &base,
        };
        for status in [307, 308] {
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("Location", format!("{}/leak", target.uri())),
                )
                .mount(&server)
                .await;
            let diagnostic = api
                .send_prompt("TOKEN-SENTINEL", json!({"text":"PRIVATE-SENTINEL"}))
                .await
                .unwrap_err();
            assert_eq!(diagnostic.http_status, Some(status));
            assert_eq!(diagnostic.failure, TelegramFailure::HttpRejected);
            assert!(target.received_requests().await.unwrap().is_empty());
            let source = server.received_requests().await.unwrap();
            assert_eq!(source.len(), 1);
            assert_eq!(
                source[0].headers.get("x-telegram-fixture-client").unwrap(),
                "configured"
            );
        }
        // The ORIGINAL path governs redirects. Ordinary traffic still follows
        // redirects even when an intermediate path happens to look like Bot API.
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/botordinary/method", target.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(wiremock::matchers::path("/botordinary/method"))
            .respond_with(
                ResponseTemplate::new(308)
                    .insert_header("Location", format!("{}/ordinary-final", target.uri())),
            )
            .mount(&target)
            .await;
        Mock::given(wiremock::matchers::path("/ordinary-final"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ordinary response"))
            .mount(&target)
            .await;
        let response = client.get(format!("{base}/ordinary")).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "ordinary response");
        assert_eq!(target.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn telegram_new_timeout_and_interrupted_body_remain_unknown() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(CALL_TIMEOUT_SECS + 1))
                    .set_body_json(json!({"ok":true,"result":{"message_id":1}})),
            )
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(app_redirect_policy())
            .build()
            .unwrap();
        let base = server.uri();
        let api = TelegramApi {
            http: &client,
            base_url: &base,
        };
        let diagnostic = api
            .send_prompt("TOKEN-SENTINEL", json!({}))
            .await
            .unwrap_err();
        assert_eq!(diagnostic.failure, TelegramFailure::Timeout);
        assert!(diagnostic.uncertain());
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (sent, ()) = tokio::join!(
            async {
                let api = TelegramApi {
                    http: &client,
                    base_url: &base,
                };
                api.send_prompt("TOKEN-SENTINEL", json!({}))
                    .await
                    .unwrap_err()
            },
            async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 4096];
                // This read is a request-arrival barrier; the response below is
                // deliberately truncated regardless of the request's chunking.
                let received = socket.read(&mut bytes).await.unwrap();
                assert!(received > 0, "fixture expected HTTP request bytes");
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{\"ok\":true}").await.unwrap();
            }
        );
        assert_eq!(sent.failure, TelegramFailure::InvalidResponse);
        assert!(sent.uncertain());
        assert!(!format!("{sent:?}").contains("SENTINEL"));
    }
}
