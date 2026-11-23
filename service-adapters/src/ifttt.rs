//! IFTTT Webhooks appends the raw key only at egress, after the normal
//! NyxID authorization pipeline.

use std::sync::LazyLock;
use std::time::Duration;

use reqwest::{Method, Response};

pub const AUTH_METHOD: &str = "ifttt_webhook";
pub const BASE_URL: &str = "https://maker.ifttt.com";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IFTTT Webhooks requires the fixed https://maker.ifttt.com destination")]
    Destination,
    #[error("IFTTT Webhooks supports POST only; WebSocket and other methods are not supported")]
    Method,
    #[error(
        "Use /trigger/EVENT or /trigger/EVENT/json; EVENT must contain only ASCII letters, numbers, and underscores"
    )]
    Path,
    #[error("IFTTT Webhooks accepts values in the JSON request body, not query parameters")]
    Query,
    #[error(
        "Enter the raw IFTTT Webhooks key, not a URL; only ASCII letters, numbers, hyphens, and underscores are accepted"
    )]
    Credential,
    #[error("IFTTT Webhooks requires a valid JSON request body")]
    Json,
    #[error("Standard IFTTT events accept only optional string fields value1, value2, and value3")]
    Values,
    #[error("IFTTT request outcome is unknown; check IFTTT Activity before retrying")]
    Transport(#[source] reqwest::Error),
}

pub fn validate_destination(base_url: &str) -> Result<(), Error> {
    if base_url != BASE_URL && base_url != "https://maker.ifttt.com/" {
        return Err(Error::Destination);
    }
    Ok(())
}

pub fn validate_credential(key: &str) -> Result<(), Error> {
    if key.is_empty()
        || key.len() > 512
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(Error::Credential);
    }
    Ok(())
}

/// Validate the public, credential-free operation, including raw proxy calls.
pub fn validate_request(
    base_url: &str,
    method: &Method,
    path: &str,
    query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<(), Error> {
    validate_destination(base_url)?;
    if method != Method::POST {
        return Err(Error::Method);
    }
    if query.is_some_and(|q| !q.is_empty()) {
        return Err(Error::Query);
    }
    let path = path.strip_prefix('/').unwrap_or(path);
    let tail = path.strip_prefix("trigger/").ok_or(Error::Path)?;
    let (event, json_event) = match tail.strip_suffix("/json") {
        Some(event) => (event, true),
        None => (tail, false),
    };
    if event.is_empty()
        || !event
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(Error::Path);
    }
    let body = if json_event {
        body.ok_or(Error::Json)?
    } else {
        body.filter(|body| !body.is_empty()).unwrap_or(b"{}")
    };
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| Error::Json)?;
    if !json_event {
        let object = value.as_object().ok_or(Error::Values)?;
        if object
            .iter()
            .any(|(k, v)| !matches!(k.as_str(), "value1" | "value2" | "value3") || !v.is_string())
        {
            return Err(Error::Values);
        }
    }
    Ok(())
}

/// The dedicated client always disables redirects and retries, even when the
/// caller customizes DNS/TLS (for example, to exercise a local TLS fixture).
pub struct Client(reqwest::Client);

impl Client {
    pub fn new(builder: reqwest::ClientBuilder) -> Result<Self, reqwest::Error> {
        builder
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map(Self)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn forward(
        &self,
        base_url: &str,
        method: &Method,
        path: &str,
        query: Option<&str>,
        key: &str,
        body: Option<&[u8]>,
        headers: &[(String, String)],
    ) -> Result<Response, Error> {
        validate_request(base_url, method, path, query, body)?;
        validate_credential(key)?;
        let url = format!("{BASE_URL}/{}/with/key/{key}", path.trim_start_matches('/'));
        let mut request = self.0.post(url);
        for (name, value) in headers {
            if !matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization"
                    | "proxy-authorization"
                    | "host"
                    | "content-type"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "upgrade"
            ) && !name.to_ascii_lowercase().starts_with("x-nyxid-")
            {
                request = request.header(name, value);
            }
        }
        let response = request
            .header("Content-Type", "application/json")
            .body(
                body.filter(|body| !body.is_empty())
                    .unwrap_or(b"{}")
                    .to_vec(),
            )
            .send()
            .await
            .map_err(|error| Error::Transport(error.without_url()))?;
        // IFTTT only acknowledges event receipt. Its body and headers can echo
        // the credential-bearing URL, so expose a fixed receipt instead.
        let upstream_status = response.status();
        let accepted = upstream_status.is_success();
        let status = if accepted {
            // A 204 cannot carry the receipt body over HTTP.
            http::StatusCode::OK
        } else if upstream_status.is_redirection() {
            http::StatusCode::BAD_GATEWAY
        } else {
            upstream_status
        };
        let receipt = serde_json::json!({
            "accepted": accepted,
            "upstream_status": upstream_status.as_u16(),
            "message": if accepted {
                "IFTTT accepted the event. Applet completion is not confirmed; check IFTTT Activity."
            } else {
                "IFTTT did not acknowledge the event. Check IFTTT Activity before retrying."
            }
        });
        Ok(http::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(receipt.to_string())
            .expect("fixed IFTTT receipt")
            .into())
    }
}

pub fn client() -> &'static Client {
    static CLIENT: LazyLock<Client> =
        LazyLock::new(|| Client::new(reqwest::Client::builder()).expect("IFTTT HTTP client"));
    &CLIENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;

    const KEY: &str = "ifttt_test_key-NOT_A_REAL_CREDENTIAL";

    #[tokio::test]
    async fn ifttt_sends_both_contracts_and_sanitizes_receipts() {
        for (path, body) in [
            (
                "trigger/Test_123",
                br#"{"value1":"hello","value2":"world","value3":"!"}"#.as_slice(),
            ),
            (
                "/trigger/Test_123/json",
                br#"{"event":"payload","body":[1,true],"authorization":"payload only"}"#.as_slice(),
            ),
            ("trigger/Test_123/json", b"[1,true,null]".as_slice()),
        ] {
            let echoed = format!("/trigger/Test_123/with/key/{KEY}");
            let mut server = fixture(&format!("HTTP/1.1 200 OK\r\nLocation: {echoed}\r\nX-Echo: {KEY}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{echoed}", echoed.len())).await;
            let response = server
                .client
                .forward(
                    BASE_URL,
                    &Method::POST,
                    path,
                    None,
                    KEY,
                    Some(body),
                    &[
                        ("User-Agent".into(), "my-device/1".into()),
                        ("Authorization".into(), "private-nyx-token".into()),
                        ("X-NyxID-User-Id".into(), "private-identity".into()),
                    ],
                )
                .await
                .unwrap();
            assert!(response.headers().get("location").is_none());
            assert!(response.headers().get("x-echo").is_none());
            let receipt = response.text().await.unwrap();
            assert!(!receipt.contains(KEY));
            assert!(!receipt.contains("with/key"));
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&receipt).unwrap()["accepted"],
                true
            );
            let request = server.requests.recv().await.unwrap();
            let split = request.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
            let headers = String::from_utf8_lossy(&request[..split]);
            assert!(headers.starts_with(&format!(
                "POST /{}/with/key/{KEY} HTTP/1.1",
                path.trim_start_matches('/')
            )));
            assert!(headers.contains("content-type: application/json"));
            assert!(headers.contains("user-agent: my-device/1"));
            assert!(!headers.contains("private-"));
            assert_eq!(&request[split + 4..], body);
        }
    }

    #[tokio::test]
    async fn ifttt_rejects_malformed_inputs_without_dispatch() {
        let mut server = fixture("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        for method in [
            Method::GET,
            Method::HEAD,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
            Method::PATCH,
        ] {
            assert!(
                server
                    .client
                    .forward(
                        BASE_URL,
                        &method,
                        "trigger/event",
                        None,
                        KEY,
                        Some(b"{}"),
                        &[]
                    )
                    .await
                    .is_err()
            );
        }
        for path in [
            "",
            "trigger/",
            "//trigger/event",
            "trigger/../event",
            "trigger/a-b",
            "trigger/a b",
            "trigger/é",
            "trigger/e/json/extra",
            "trigger/e/with/key/foo",
            "trigger/%65",
            "trigger/e?x",
            "trigger/e#x",
            "trigger/e\\x",
            "trigger/%252e%252e",
        ] {
            assert!(
                server
                    .client
                    .forward(BASE_URL, &Method::POST, path, None, KEY, Some(b"{}"), &[])
                    .await
                    .is_err(),
                "{path}"
            );
        }
        for base in [
            "https://evil.invalid",
            "http://maker.ifttt.com",
            "https://maker.ifttt.com/extra",
            "https://maker.ifttt.com?key=secret",
            "https://maker.ifttt.com@evil.invalid",
            "https://maker.ifttt.com:8443",
        ] {
            assert!(
                server
                    .client
                    .forward(
                        base,
                        &Method::POST,
                        "trigger/event",
                        None,
                        KEY,
                        Some(b"{}"),
                        &[]
                    )
                    .await
                    .is_err()
            );
        }
        for key in [
            "",
            "https://maker.ifttt.com/use/key",
            "key/path",
            "key%2F",
            "key?",
            "key#",
            "key\r\n",
            "ü",
            "..",
        ] {
            let error = server
                .client
                .forward(
                    BASE_URL,
                    &Method::POST,
                    "trigger/event",
                    None,
                    key,
                    Some(b"{}"),
                    &[],
                )
                .await
                .unwrap_err();
            assert!(matches!(error, Error::Credential));
        }
        for body in [
            br#"{"value1":1}"#.as_slice(),
            br#"{"value4":"no"}"#,
            b"[]",
            b"null",
            b"invalid",
        ] {
            assert!(
                server
                    .client
                    .forward(
                        BASE_URL,
                        &Method::POST,
                        "trigger/event",
                        None,
                        KEY,
                        Some(body),
                        &[]
                    )
                    .await
                    .is_err()
            );
        }
        assert!(
            server
                .client
                .forward(
                    BASE_URL,
                    &Method::POST,
                    "trigger/event/json",
                    None,
                    KEY,
                    None,
                    &[]
                )
                .await
                .is_err()
        );
        assert!(
            server
                .client
                .forward(
                    BASE_URL,
                    &Method::POST,
                    "trigger/event",
                    Some("value1=hi"),
                    KEY,
                    Some(b"{}"),
                    &[]
                )
                .await
                .is_err()
        );
        assert!(server.requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn ifttt_receipt_survives_empty_success_and_provider_rejection() {
        for (status, accepted, public_status) in [(204, true, 200), (401, false, 401)] {
            let server = fixture(&format!(
                "HTTP/1.1 {status} Status\r\nX-Echo: {KEY}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )).await;
            let response = server
                .client
                .forward(
                    BASE_URL,
                    &Method::POST,
                    "trigger/event",
                    None,
                    KEY,
                    Some(b""),
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), public_status);
            assert!(response.headers().get("x-echo").is_none());
            let receipt: serde_json::Value =
                serde_json::from_str(&response.text().await.unwrap()).unwrap();
            assert_eq!(receipt["accepted"], accepted);
            assert_eq!(receipt["upstream_status"], status);
            assert!(!receipt.to_string().contains(KEY));
        }
    }

    #[tokio::test]
    async fn ifttt_does_not_follow_redirect_or_retry_unknown_outcome() {
        let mut server = fixture(&format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {BASE_URL}/trigger/second/with/key/{KEY}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")).await;
        let response = server
            .client
            .forward(
                BASE_URL,
                &Method::POST,
                "trigger/event",
                None,
                KEY,
                None,
                &[],
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 502);
        let receipt: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(receipt["accepted"], false);
        assert_eq!(receipt["upstream_status"], 307);
        assert!(server.requests.recv().await.is_some());
        assert!(server.requests.try_recv().is_err());

        let mut server = fixture("").await;
        let error = server
            .client
            .forward(
                BASE_URL,
                &Method::POST,
                "trigger/event",
                None,
                KEY,
                None,
                &[],
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Transport(_)));
        assert!(!format!("{error:?}").contains(KEY));
        assert!(error.to_string().contains("outcome is unknown"));
        assert!(server.requests.recv().await.is_some());
        assert!(server.requests.try_recv().is_err());
    }
}
