//! Node-native OAuth flow: device code, authorization code, and token refresh.
//!
//! Fetches OAuth config from NyxID catalog or uses CLI-provided URLs.
//! Runs the flow, stores tokens locally, never sends them to NyxID.

use std::collections::HashMap;
use std::net::TcpListener;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use super::error::{Error, Result};

/// OAuth config fetched from NyxID catalog or provided via CLI.
#[allow(dead_code)]
pub struct OAuthConfig {
    pub authorization_url: Option<String>,
    pub token_url: String,
    pub device_code_url: Option<String>,
    pub device_verification_url: Option<String>,
    pub device_token_url: Option<String>,
    pub default_scopes: Vec<String>,
    pub supports_pkce: bool,
    pub device_code_format: String, // "rfc8628" | "openai"
    pub token_endpoint_auth_method: String,
    pub extra_auth_params: Option<HashMap<String, String>>,
    pub oauth_client_id: Option<String>,
    pub client_id_param_name: Option<String>,
}

/// Token response from OAuth token endpoint.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: Option<String>,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

/// Device code response (RFC 8628).
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: Option<String>,
    verification_url: Option<String>,
    expires_in: u64,
    #[serde(deserialize_with = "deserialize_interval")]
    interval: u64,
}

impl DeviceCodeResponse {
    fn verification_uri(&self) -> &str {
        self.verification_uri
            .as_deref()
            .or(self.verification_url.as_deref())
            .unwrap_or("(no verification URL provided)")
    }
}

fn deserialize_interval<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        Number(u64),
        Str(String),
    }

    match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::Number(n) => Ok(n),
        StringOrNumber::Str(s) => s
            .parse()
            .map_err(|_| serde::de::Error::custom("interval must be a number")),
    }
}

impl OAuthConfig {
    fn client_id_param_name(&self) -> &str {
        self.client_id_param_name.as_deref().unwrap_or("client_id")
    }
}

pub fn oauth_config_from_catalog_value(body: &serde_json::Value) -> Result<OAuthConfig> {
    let token_url = body["token_url"]
        .as_str()
        .ok_or_else(|| Error::Config("Catalog entry has no token_url".to_string()))?
        .to_string();

    Ok(OAuthConfig {
        authorization_url: body["authorization_url"].as_str().map(String::from),
        token_url,
        device_code_url: body["device_code_url"].as_str().map(String::from),
        device_verification_url: body["device_verification_url"].as_str().map(String::from),
        device_token_url: body["device_token_url"].as_str().map(String::from),
        default_scopes: body["default_scopes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        supports_pkce: body["supports_pkce"].as_bool().unwrap_or(false),
        device_code_format: body["device_code_format"]
            .as_str()
            .unwrap_or("rfc8628")
            .to_string(),
        token_endpoint_auth_method: body["token_endpoint_auth_method"]
            .as_str()
            .unwrap_or("client_secret_post")
            .to_string(),
        extra_auth_params: body["extra_auth_params"].as_object().map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|text| (key.clone(), text.to_string()))
                })
                .collect()
        }),
        oauth_client_id: body["oauth_client_id"].as_str().map(String::from),
        client_id_param_name: body["client_id_param_name"].as_str().map(String::from),
    })
}

/// Fetch OAuth config from NyxID catalog API.
pub async fn fetch_catalog_oauth_config(
    api_base_url: &str,
    access_token: Option<&str>,
    service_slug: &str,
) -> Result<OAuthConfig> {
    let client = reqwest::Client::new();
    let url = format!("{api_base_url}/api/v1/catalog/{service_slug}");

    let mut req = client.get(&url);
    if let Some(token) = access_token {
        req = req.bearer_auth(token);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| Error::Config(format!("Failed to fetch catalog: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::Config(format!(
            "Catalog returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::Config(format!("Failed to parse catalog response: {e}")))?;

    oauth_config_from_catalog_value(&body)
}

/// Run RFC 8628 device code flow.
pub async fn run_device_code_flow(
    config: &OAuthConfig,
    client_id: &str,
    client_secret: Option<&str>,
    scopes: &str,
) -> Result<TokenResponse> {
    let client = reqwest::Client::new();
    let device_code_url = config
        .device_code_url
        .as_deref()
        .ok_or_else(|| Error::Config("No device_code_url available".to_string()))?;
    let client_id_param_name = config.client_id_param_name().to_string();

    // Step 1: Request device code
    let mut request_form = vec![
        (client_id_param_name.clone(), client_id.to_string()),
        ("scope".to_string(), scopes.to_string()),
    ];
    if let Some(secret) = client_secret {
        request_form.push(("client_secret".to_string(), secret.to_string()));
    }

    let resp = client
        .post(device_code_url)
        .form(&request_form)
        .send()
        .await
        .map_err(|e| Error::Config(format!("Device code request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Config(format!(
            "Device code request failed {status}: {text}"
        )));
    }

    let device_resp: DeviceCodeResponse = resp
        .json()
        .await
        .map_err(|e| Error::Config(format!("Failed to parse device code response: {e}")))?;

    // Step 2: Display code to user
    println!();
    println!("  Your code: {}", device_resp.user_code);
    println!("  Visit: {}", device_resp.verification_uri());
    println!();
    println!("  Waiting for authorization...");

    // Step 3: Poll for token
    let token_poll_url = config
        .device_token_url
        .as_deref()
        .unwrap_or(&config.token_url);

    let mut interval = std::time::Duration::from_secs(device_resp.interval.max(1));
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(device_resp.expires_in);

    loop {
        tokio::time::sleep(interval).await;

        if std::time::Instant::now() > deadline {
            return Err(Error::Config("Device code expired".to_string()));
        }

        let mut form = vec![
            (
                "grant_type".to_string(),
                "urn:ietf:params:oauth:grant-type:device_code".to_string(),
            ),
            ("device_code".to_string(), device_resp.device_code.clone()),
            (client_id_param_name.clone(), client_id.to_string()),
        ];
        if let Some(secret) = client_secret {
            form.push(("client_secret".to_string(), secret.to_string()));
        }

        let resp = client.post(token_poll_url).form(&form).send().await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let token: TokenResponse = r
                    .json()
                    .await
                    .map_err(|e| Error::Config(format!("Failed to parse token: {e}")))?;
                println!("  Authorization successful.");
                return Ok(token);
            }
            Ok(r) if r.status().as_u16() == 428 || r.status().as_u16() == 400 => {
                // authorization_pending or slow_down
                let body: serde_json::Value = r.json().await.unwrap_or_default();
                let error = body["error"].as_str().unwrap_or("authorization_pending");
                match error {
                    "slow_down" => {
                        interval += std::time::Duration::from_secs(5);
                    }
                    "authorization_pending" => {}
                    "expired_token" => {
                        return Err(Error::Config("Device code expired".to_string()));
                    }
                    "access_denied" => {
                        return Err(Error::Config("Authorization denied".to_string()));
                    }
                    other => {
                        return Err(Error::Config(format!("OAuth error: {other}")));
                    }
                }
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                return Err(Error::Config(format!("Token poll error {status}: {text}")));
            }
            Err(e) => {
                tracing::warn!(error = %e, "Token poll request failed, retrying");
            }
        }
    }
}

pub async fn run_authorization_code_flow(
    config: &OAuthConfig,
    client_id: &str,
    client_secret: Option<&str>,
    scopes: &str,
) -> Result<TokenResponse> {
    let authorization_url = config
        .authorization_url
        .as_deref()
        .ok_or_else(|| Error::Config("No authorization_url available".to_string()))?;

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| Error::Config(format!("Failed to bind local callback server: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Config(format!("Failed to get callback address: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = generate_state();
    let code_verifier = config.supports_pkce.then(generate_pkce_verifier);
    let code_challenge = code_verifier
        .as_deref()
        .map(pkce_code_challenge)
        .transpose()?;

    let mut url = Url::parse(authorization_url)
        .map_err(|e| Error::Config(format!("Invalid authorization URL: {e}")))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair(config.client_id_param_name(), client_id);
        pairs.append_pair("redirect_uri", &redirect_uri);
        pairs.append_pair("response_type", "code");
        pairs.append_pair("state", &state);
        if !scopes.is_empty() {
            pairs.append_pair("scope", scopes);
        }
        if let Some(ref challenge) = code_challenge {
            pairs.append_pair("code_challenge", challenge);
            pairs.append_pair("code_challenge_method", "S256");
        }
        if let Some(ref params) = config.extra_auth_params {
            for (key, value) in params {
                if !is_reserved_oauth_param(key) {
                    pairs.append_pair(key, value);
                }
            }
        }
    }

    let authorization_url: String = url.into();

    println!("Opening browser for OAuth authorization...");
    println!();
    println!("If the browser does not open, visit:");
    println!("  {authorization_url}");
    println!();

    let _ = crate::browser::open_browser(&authorization_url);

    let code = wait_for_authorization_code(listener, &state).await?;
    exchange_authorization_code(
        config,
        client_id,
        client_secret,
        &code,
        &redirect_uri,
        code_verifier.as_deref(),
    )
    .await
}

/// Refresh an OAuth token using refresh_token grant.
pub async fn refresh_token(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_tok: &str,
    auth_method: &str,
    client_id_param_name: Option<&str>,
) -> Result<TokenResponse> {
    let client = reqwest::Client::new();
    let client_id_param_name = client_id_param_name.unwrap_or("client_id");

    let mut req = client.post(token_url);

    match auth_method {
        "client_secret_basic" => {
            req = req.basic_auth(client_id, client_secret);
            req = req.form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_tok),
            ]);
        }
        _ => {
            // client_secret_post (default)
            let mut form = vec![
                ("grant_type".to_string(), "refresh_token".to_string()),
                ("refresh_token".to_string(), refresh_tok.to_string()),
                (client_id_param_name.to_string(), client_id.to_string()),
            ];
            if let Some(secret) = client_secret {
                form.push(("client_secret".to_string(), secret.to_string()));
            }
            req = req.form(&form);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| Error::Config(format!("Token refresh failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Config(format!(
            "Token refresh error {status}: {text}"
        )));
    }

    resp.json()
        .await
        .map_err(|e| Error::Config(format!("Failed to parse refresh response: {e}")))
}

fn is_reserved_oauth_param(key: &str) -> bool {
    matches!(
        key,
        "client_id"
            | "client_secret"
            | "redirect_uri"
            | "response_type"
            | "state"
            | "code"
            | "code_challenge"
            | "code_challenge_method"
            | "scope"
    )
}

fn generate_state() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn generate_pkce_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_code_challenge(verifier: &str) -> Result<String> {
    let digest = Sha256::digest(verifier.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(digest))
}

enum CallbackResult {
    Success(String),
    Error(String),
    Ignore,
}

async fn wait_for_authorization_code(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::Config(format!("Failed to set callback listener non-blocking: {e}")))?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| Error::Config(format!("Failed to create async callback listener: {e}")))?;

    let timeout = tokio::time::sleep(std::time::Duration::from_secs(180));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (mut stream, _) = accept
                    .map_err(|e| Error::Config(format!("Failed to accept callback connection: {e}")))?;
                let mut buf = vec![0u8; 8192];
                let n = stream
                    .read(&mut buf)
                    .await
                    .map_err(|e| Error::Config(format!("Failed to read callback request: {e}")))?;
                let request = String::from_utf8_lossy(&buf[..n]);

                match parse_callback_request(&request, expected_state) {
                    CallbackResult::Success(code) => {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{}",
                            callback_success_html()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        return Ok(code);
                    }
                    CallbackResult::Error(message) => {
                        let response = format!(
                            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{}",
                            callback_error_html()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        return Err(Error::Config(message));
                    }
                    CallbackResult::Ignore => {
                        let response = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(response.as_bytes()).await;
                    }
                }
            }
            () = &mut timeout => {
                return Err(Error::Config("OAuth authorization timed out after 180s".to_string()));
            }
        }
    }
}

fn parse_callback_request(request: &str, expected_state: &str) -> CallbackResult {
    let Some(first_line) = request.lines().next() else {
        return CallbackResult::Ignore;
    };
    let Some(path) = first_line.split_whitespace().nth(1) else {
        return CallbackResult::Ignore;
    };
    if !path.starts_with("/callback") {
        return CallbackResult::Ignore;
    }

    let Some(query) = path.split('?').nth(1) else {
        return CallbackResult::Error(
            "OAuth callback did not include query parameters".to_string(),
        );
    };
    let params: HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    let state = params
        .get("state")
        .and_then(|value| urlencoding::decode(value).ok())
        .map(|value| value.into_owned());
    if state.as_deref() != Some(expected_state) {
        return CallbackResult::Error("OAuth callback state mismatch".to_string());
    }

    if let Some(error) = params
        .get("error")
        .and_then(|value| urlencoding::decode(value).ok())
        .map(|value| value.into_owned())
    {
        let description = params
            .get("error_description")
            .and_then(|value| urlencoding::decode(value).ok())
            .map(|value| value.into_owned())
            .unwrap_or(error.clone());
        return CallbackResult::Error(format!("OAuth authorization failed: {description}"));
    }

    let Some(code) = params
        .get("code")
        .and_then(|value| urlencoding::decode(value).ok())
        .map(|value| value.into_owned())
        .filter(|value| !value.is_empty())
    else {
        return CallbackResult::Error(
            "OAuth callback did not include an authorization code".to_string(),
        );
    };

    CallbackResult::Success(code)
}

async fn exchange_authorization_code(
    config: &OAuthConfig,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
) -> Result<TokenResponse> {
    let client = reqwest::Client::new();
    let mut req = client.post(&config.token_url);
    let client_id_param_name = config.client_id_param_name().to_string();

    match config.token_endpoint_auth_method.as_str() {
        "client_secret_basic" => {
            req = req.basic_auth(client_id, client_secret);
            let mut form = vec![
                ("grant_type".to_string(), "authorization_code".to_string()),
                ("code".to_string(), code.to_string()),
                ("redirect_uri".to_string(), redirect_uri.to_string()),
            ];
            if client_secret.is_none() {
                form.push((client_id_param_name, client_id.to_string()));
            }
            if let Some(verifier) = code_verifier {
                form.push(("code_verifier".to_string(), verifier.to_string()));
            }
            req = req.form(&form);
        }
        _ => {
            let mut form = vec![
                ("grant_type".to_string(), "authorization_code".to_string()),
                ("code".to_string(), code.to_string()),
                ("redirect_uri".to_string(), redirect_uri.to_string()),
                (client_id_param_name, client_id.to_string()),
            ];
            if let Some(secret) = client_secret {
                form.push(("client_secret".to_string(), secret.to_string()));
            }
            if let Some(verifier) = code_verifier {
                form.push(("code_verifier".to_string(), verifier.to_string()));
            }
            req = req.form(&form);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| Error::Config(format!("Token exchange failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Config(format!(
            "Token exchange error {status}: {text}"
        )));
    }

    resp.json()
        .await
        .map_err(|e| Error::Config(format!("Failed to parse token response: {e}")))
}

fn callback_success_html() -> &'static str {
    r#"<!doctype html>
<html>
<head><title>NyxID Node OAuth</title></head>
<body style="display:flex;align-items:center;justify-content:center;min-height:100vh;font-family:system-ui;background:#0f172a;color:#e2e8f0">
<div style="text-align:center">
<h2>Authorization complete</h2>
<p style="color:#94a3b8">You can close this tab and return to your terminal.</p>
</div>
</body>
</html>"#
}

fn callback_error_html() -> &'static str {
    r#"<!doctype html>
<html>
<head><title>NyxID Node OAuth</title></head>
<body style="display:flex;align-items:center;justify-content:center;min-height:100vh;font-family:system-ui;background:#0f172a;color:#e2e8f0">
<div style="text-align:center">
<h2>Authorization failed</h2>
<p style="color:#94a3b8">Return to your terminal for the error details.</p>
</div>
</body>
</html>"#
}

#[cfg(test)]
mod tests {
    use super::{
        CallbackResult, DeviceCodeResponse, OAuthConfig, exchange_authorization_code,
        fetch_catalog_oauth_config, is_reserved_oauth_param, oauth_config_from_catalog_value,
        parse_callback_request, pkce_code_challenge, refresh_token,
    };
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_catalog_oauth_config_with_provider_specific_fields() {
        let body = serde_json::json!({
            "authorization_url": "https://example.com/oauth/authorize",
            "token_url": "https://example.com/oauth/token",
            "default_scopes": ["profile", "email"],
            "supports_pkce": true,
            "token_endpoint_auth_method": "client_secret_basic",
            "oauth_client_id": "shared-client-id",
            "client_id_param_name": "client_key",
            "extra_auth_params": {
                "access_type": "offline"
            }
        });

        let config = oauth_config_from_catalog_value(&body).expect("config");
        assert_eq!(
            config.authorization_url.as_deref(),
            Some("https://example.com/oauth/authorize")
        );
        assert_eq!(config.token_url, "https://example.com/oauth/token");
        assert_eq!(config.oauth_client_id.as_deref(), Some("shared-client-id"));
        assert_eq!(config.client_id_param_name.as_deref(), Some("client_key"));
        assert!(config.supports_pkce);
        assert_eq!(
            config
                .extra_auth_params
                .as_ref()
                .and_then(|params| params.get("access_type"))
                .map(String::as_str),
            Some("offline")
        );
    }

    #[test]
    fn parses_valid_authorization_callback() {
        let request =
            "GET /callback?code=auth_code_123&state=deadbeef HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        match parse_callback_request(request, "deadbeef") {
            CallbackResult::Success(code) => assert_eq!(code, "auth_code_123"),
            other => panic!(
                "unexpected callback result: {:?}",
                callback_result_name(&other)
            ),
        }
    }

    #[test]
    fn rejects_callback_with_wrong_state() {
        let request =
            "GET /callback?code=auth_code_123&state=wrong HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        match parse_callback_request(request, "deadbeef") {
            CallbackResult::Error(message) => {
                assert!(message.contains("state mismatch"));
            }
            other => panic!(
                "unexpected callback result: {:?}",
                callback_result_name(&other)
            ),
        }
    }

    fn callback_result_name(result: &CallbackResult) -> &'static str {
        match result {
            CallbackResult::Success(_) => "success",
            CallbackResult::Error(_) => "error",
            CallbackResult::Ignore => "ignore",
        }
    }

    #[test]
    fn ignores_non_callback_path() {
        let request = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        assert!(matches!(
            parse_callback_request(request, "x"),
            CallbackResult::Ignore
        ));
    }

    #[test]
    fn callback_missing_query_is_error() {
        let request = "GET /callback HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        assert!(matches!(
            parse_callback_request(request, "x"),
            CallbackResult::Error(_)
        ));
    }

    #[test]
    fn callback_with_error_param_returns_error() {
        let request =
            "GET /callback?error=access_denied&error_description=denied&state=s1 HTTP/1.1\r\n";
        match parse_callback_request(request, "s1") {
            CallbackResult::Error(msg) => assert!(msg.contains("denied")),
            other => panic!("expected error, got {}", callback_result_name(&other)),
        }
    }

    #[test]
    fn callback_missing_code_is_error() {
        let request = "GET /callback?state=s1 HTTP/1.1\r\n";
        match parse_callback_request(request, "s1") {
            CallbackResult::Error(msg) => assert!(msg.contains("authorization code")),
            other => panic!("expected error, got {}", callback_result_name(&other)),
        }
    }

    #[test]
    fn ignores_empty_request() {
        assert!(matches!(
            parse_callback_request("", "x"),
            CallbackResult::Ignore
        ));
    }

    #[test]
    fn catalog_config_missing_token_url_errors() {
        let body = serde_json::json!({ "authorization_url": "https://example.com" });
        assert!(oauth_config_from_catalog_value(&body).is_err());
    }

    #[test]
    fn catalog_config_defaults() {
        let body = serde_json::json!({ "token_url": "https://example.com/token" });
        let config = oauth_config_from_catalog_value(&body).expect("config");
        assert!(!config.supports_pkce);
        assert_eq!(config.device_code_format, "rfc8628");
        assert_eq!(config.token_endpoint_auth_method, "client_secret_post");
        assert!(config.default_scopes.is_empty());
        assert!(config.authorization_url.is_none());
        assert!(config.oauth_client_id.is_none());
    }

    #[test]
    fn client_id_param_name_defaults_to_client_id() {
        let body = serde_json::json!({ "token_url": "https://example.com/token" });
        let config = oauth_config_from_catalog_value(&body).expect("config");
        assert_eq!(config.client_id_param_name(), "client_id");
    }

    #[test]
    fn device_code_response_prefers_verification_uri_over_legacy_url() {
        let response: DeviceCodeResponse = serde_json::from_value(serde_json::json!({
            "device_code": "dev-1",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://provider.example/device",
            "verification_url": "https://legacy.example/device",
            "expires_in": 900,
            "interval": 7
        }))
        .expect("device response");

        assert_eq!(
            response.verification_uri(),
            "https://provider.example/device"
        );
        assert_eq!(response.interval, 7);
    }

    #[test]
    fn device_code_response_falls_back_to_verification_url_and_default_text() {
        let legacy: DeviceCodeResponse = serde_json::from_value(serde_json::json!({
            "device_code": "dev-1",
            "user_code": "ABCD-EFGH",
            "verification_url": "https://legacy.example/device",
            "expires_in": 900,
            "interval": "11"
        }))
        .expect("legacy device response");
        assert_eq!(legacy.verification_uri(), "https://legacy.example/device");
        assert_eq!(legacy.interval, 11);

        let missing: DeviceCodeResponse = serde_json::from_value(serde_json::json!({
            "device_code": "dev-1",
            "user_code": "ABCD-EFGH",
            "expires_in": 900,
            "interval": 5
        }))
        .expect("missing verification URL response");
        assert_eq!(missing.verification_uri(), "(no verification URL provided)");
    }

    #[test]
    fn device_code_response_rejects_non_numeric_interval_string() {
        let err = serde_json::from_value::<DeviceCodeResponse>(serde_json::json!({
            "device_code": "dev-1",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://provider.example/device",
            "expires_in": 900,
            "interval": "slow"
        }))
        .expect_err("invalid interval should fail");

        assert!(err.to_string().contains("interval must be a number"));
    }

    #[test]
    fn catalog_config_ignores_non_string_scopes_and_extra_params() {
        let body = serde_json::json!({
            "token_url": "https://example.com/token",
            "default_scopes": ["profile", 42, "email"],
            "extra_auth_params": {
                "prompt": "consent",
                "ignored": true
            }
        });
        let config = oauth_config_from_catalog_value(&body).expect("config");

        assert_eq!(config.default_scopes, vec!["profile", "email"]);
        assert_eq!(
            config
                .extra_auth_params
                .as_ref()
                .and_then(|params| params.get("prompt"))
                .map(String::as_str),
            Some("consent")
        );
        assert!(
            !config
                .extra_auth_params
                .as_ref()
                .expect("params")
                .contains_key("ignored")
        );
    }

    #[test]
    fn reserved_oauth_param_filter_covers_builtin_authorization_fields() {
        for key in [
            "client_id",
            "client_secret",
            "redirect_uri",
            "response_type",
            "state",
            "code",
            "code_challenge",
            "code_challenge_method",
            "scope",
        ] {
            assert!(is_reserved_oauth_param(key), "{key} should be reserved");
        }
        assert!(!is_reserved_oauth_param("access_type"));
        assert!(!is_reserved_oauth_param("prompt"));
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        let challenge =
            pkce_code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").expect("challenge");
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn callback_decodes_url_encoded_code_and_description() {
        let request =
            "GET /callback?code=auth%2Fcode%2B1&state=state%201 HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        match parse_callback_request(request, "state 1") {
            CallbackResult::Success(code) => assert_eq!(code, "auth/code+1"),
            other => panic!("expected success, got {}", callback_result_name(&other)),
        }

        let denied = "GET /callback?error=access_denied&error_description=needs%20approval&state=s HTTP/1.1\r\n";
        match parse_callback_request(denied, "s") {
            CallbackResult::Error(message) => {
                assert!(message.contains("needs approval"), "{message}");
            }
            other => panic!("expected error, got {}", callback_result_name(&other)),
        }
    }

    #[tokio::test]
    async fn fetch_catalog_oauth_config_sends_bearer_and_parses_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/catalog/openai"))
            .and(header("authorization", "Bearer access-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "authorization_url": "https://provider.example/oauth/authorize",
                "token_url": "https://provider.example/oauth/token",
                "supports_pkce": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = fetch_catalog_oauth_config(&server.uri(), Some("access-1"), "openai")
            .await
            .expect("catalog config");

        assert_eq!(
            config.authorization_url.as_deref(),
            Some("https://provider.example/oauth/authorize")
        );
        assert_eq!(config.token_url, "https://provider.example/oauth/token");
        assert!(config.supports_pkce);
    }

    #[tokio::test]
    async fn fetch_catalog_oauth_config_surfaces_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/catalog/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .expect(1)
            .mount(&server)
            .await;

        let err = match fetch_catalog_oauth_config(&server.uri(), None, "missing").await {
            Ok(_) => panic!("missing catalog should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Catalog returned 404"));
    }

    #[tokio::test]
    async fn refresh_token_uses_custom_client_id_param_for_post_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string(
                "grant_type=refresh_token&refresh_token=ref-1&client_key=client-1&client_secret=secret-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-2",
                "refresh_token": "ref-2",
                "expires_in": 3600,
                "token_type": "Bearer",
                "scope": "read write"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let token = refresh_token(
            &format!("{}/token", server.uri()),
            "client-1",
            Some("secret-1"),
            "ref-1",
            "client_secret_post",
            Some("client_key"),
        )
        .await
        .expect("refresh token");

        assert_eq!(token.access_token, "access-2");
        assert_eq!(token.refresh_token.as_deref(), Some("ref-2"));
        assert_eq!(token.expires_in, Some(3600));
        assert_eq!(token.token_type.as_deref(), Some("Bearer"));
        assert_eq!(token.scope.as_deref(), Some("read write"));
    }

    #[tokio::test]
    async fn refresh_token_uses_basic_auth_without_client_secret_in_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header("authorization", "Basic Y2xpZW50LTE6c2VjcmV0LTE="))
            .and(body_string("grant_type=refresh_token&refresh_token=ref-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-2"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let token = refresh_token(
            &format!("{}/token", server.uri()),
            "client-1",
            Some("secret-1"),
            "ref-1",
            "client_secret_basic",
            None,
        )
        .await
        .expect("refresh token");

        assert_eq!(token.access_token, "access-2");
        assert!(token.refresh_token.is_none());
    }

    #[tokio::test]
    async fn exchange_authorization_code_posts_code_verifier_and_custom_client_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string(
                "grant_type=authorization_code&code=code-1&redirect_uri=http%3A%2F%2F127.0.0.1%3A9000%2Fcallback&client_key=client-1&client_secret=secret-1&code_verifier=verifier-1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-3",
                "refresh_token": "ref-3"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = OAuthConfig {
            authorization_url: Some("https://provider.example/authorize".to_string()),
            token_url: format!("{}/token", server.uri()),
            device_code_url: None,
            device_verification_url: None,
            device_token_url: None,
            default_scopes: vec![],
            supports_pkce: true,
            device_code_format: "rfc8628".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            oauth_client_id: None,
            client_id_param_name: Some("client_key".to_string()),
        };

        let token = exchange_authorization_code(
            &config,
            "client-1",
            Some("secret-1"),
            "code-1",
            "http://127.0.0.1:9000/callback",
            Some("verifier-1"),
        )
        .await
        .expect("token exchange");

        assert_eq!(token.access_token, "access-3");
        assert_eq!(token.refresh_token.as_deref(), Some("ref-3"));
    }
}
