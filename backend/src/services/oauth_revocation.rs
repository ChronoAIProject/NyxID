use std::sync::LazyLock;
use std::time::Duration;

use hmac::{Hmac, Mac};
use reqwest::header::{ACCEPT, USER_AGENT};
use reqwest::{Method, StatusCode};
use sha2::Sha256;
use tokio::sync::{Semaphore, SemaphorePermit, TryAcquireError};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::models::provider_config::{ProviderConfig, RevocationConfig};
use crate::models::user_provider_token::UserProviderToken;
use crate::services::oauth_flow;
use crate::services::user_credentials_service::{self, ResolvedOAuthCredentials};

const MAX_REVOCATION_RESPONSE_BODY_SIZE: usize = 64 * 1024;
const REVOCATION_TIMEOUT: Duration = Duration::from_secs(30);
const GITHUB_USER_AGENT: &str = "nyxid";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

static REVOCATION_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REVOCATION_TIMEOUT)
        .build()
        .expect("revocation HTTP client must build")
});

static REVOCATION_SEMAPHORE: Semaphore = Semaphore::const_new(32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevocationScope {
    Grant,
    Token,
}

impl RevocationScope {
    pub const fn from_grant(revoke_grant: bool) -> Self {
        if revoke_grant {
            Self::Grant
        } else {
            Self::Token
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grant => "grant",
            Self::Token => "token",
        }
    }
}

pub struct RevocationRequest<'a> {
    pub provider: &'a ProviderConfig,
    pub scope: RevocationScope,
    pub creds: Option<ResolvedOAuthCredentials>,
    pub access_token: Option<Zeroizing<String>>,
    pub refresh_token: Option<Zeroizing<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenOutcome {
    Delivered,
    NotFound,
    SendFailed,
    Skipped(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevocationOutcome {
    pub style: Option<String>,
    pub scope: RevocationScope,
    pub access: Option<TokenOutcome>,
    pub refresh: Option<TokenOutcome>,
}

pub fn effective_revocation(provider: &ProviderConfig) -> Option<RevocationConfig> {
    provider.revocation.clone().or_else(|| {
        provider
            .revocation_url
            .as_ref()
            .map(|url| RevocationConfig {
                style: "rfc7009".to_string(),
                url: url.clone(),
                auth: "inherit".to_string(),
                revokes_grant: false,
            })
    })
}

pub fn try_acquire_revocation_permit() -> Result<SemaphorePermit<'static>, TryAcquireError> {
    REVOCATION_SEMAPHORE.try_acquire()
}

/// Best-effort remote revocation. All failures are represented in the outcome.
pub async fn revoke_remote(req: RevocationRequest<'_>) -> RevocationOutcome {
    let Some(config) = effective_revocation(req.provider) else {
        return RevocationOutcome {
            style: None,
            scope: req.scope,
            access: req
                .access_token
                .as_ref()
                .map(|_| TokenOutcome::Skipped("not_configured")),
            refresh: req
                .refresh_token
                .as_ref()
                .map(|_| TokenOutcome::Skipped("not_configured")),
        };
    };

    let style = config.style.clone();
    let access_token = req.access_token.as_ref().map(|token| token.as_str());
    let refresh_token = req.refresh_token.as_ref().map(|token| token.as_str());
    let (access, refresh) = match config.style.as_str() {
        "rfc7009" => {
            let access = revoke_rfc7009_token(
                req.provider,
                &config,
                req.creds.as_ref(),
                access_token,
                "access_token",
            )
            .await;
            let refresh = revoke_rfc7009_token(
                req.provider,
                &config,
                req.creds.as_ref(),
                refresh_token,
                "refresh_token",
            )
            .await;
            (access, refresh)
        }
        "github" => {
            let access =
                revoke_github_token(&config, req.scope, req.creds.as_ref(), access_token).await;
            let refresh = if req.scope == RevocationScope::Token {
                revoke_github_token(&config, req.scope, req.creds.as_ref(), refresh_token).await
            } else {
                None
            };
            (access, refresh)
        }
        "self_bearer" => {
            let access = revoke_self_bearer(&config, access_token).await;
            let refresh = revoke_self_bearer(&config, refresh_token).await;
            (access, refresh)
        }
        "facebook_deauth" if req.scope == RevocationScope::Token => (
            req.access_token
                .as_ref()
                .map(|_| TokenOutcome::Skipped("grant_only")),
            None,
        ),
        "facebook_deauth" => {
            let access = revoke_facebook_grant(&config, req.creds.as_ref(), access_token).await;
            (access, None)
        }
        unknown => {
            tracing::warn!(
                provider_slug = %req.provider.slug,
                revocation_style = %unknown,
                "Skipping unsupported OAuth revocation style"
            );
            (
                req.access_token
                    .as_ref()
                    .map(|_| TokenOutcome::Skipped("unsupported_style")),
                req.refresh_token
                    .as_ref()
                    .map(|_| TokenOutcome::Skipped("unsupported_style")),
            )
        }
    };

    RevocationOutcome {
        style: Some(style),
        scope: req.scope,
        access,
        refresh,
    }
}

/// Compatibility bridge for the legacy disconnect path. W3 replaces this with
/// pre-image claims and post-teardown detached execution.
pub async fn try_revoke_token_remote(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
    token: &UserProviderToken,
    scope: RevocationScope,
) -> RevocationOutcome {
    let Ok(_permit) = try_acquire_revocation_permit() else {
        return RevocationOutcome {
            style: effective_revocation(provider).map(|config| config.style),
            scope,
            access: token
                .access_token_encrypted
                .as_ref()
                .map(|_| TokenOutcome::Skipped("saturated")),
            refresh: token
                .refresh_token_encrypted
                .as_ref()
                .map(|_| TokenOutcome::Skipped("saturated")),
        };
    };
    let creds = user_credentials_service::resolve_token_oauth_credentials(
        db,
        encryption_keys,
        provider,
        token.credential_user_id.as_deref(),
    )
    .await
    .ok();
    let access_token =
        decrypt_token(encryption_keys, token.access_token_encrypted.as_deref()).await;
    let refresh_token =
        decrypt_token(encryption_keys, token.refresh_token_encrypted.as_deref()).await;

    revoke_remote(RevocationRequest {
        provider,
        scope,
        creds,
        access_token,
        refresh_token,
    })
    .await
}

async fn decrypt_token(
    encryption_keys: &EncryptionKeys,
    encrypted: Option<&[u8]>,
) -> Option<Zeroizing<String>> {
    let mut plaintext = Zeroizing::new(encryption_keys.decrypt(encrypted?).await.ok()?);
    let token = String::from_utf8(std::mem::take(&mut *plaintext)).ok()?;
    Some(Zeroizing::new(token))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rfc7009Auth {
    None,
    ClientId,
    Basic,
    Post,
}

async fn revoke_rfc7009_token(
    provider: &ProviderConfig,
    config: &RevocationConfig,
    creds: Option<&ResolvedOAuthCredentials>,
    token: Option<&str>,
    token_type_hint: &'static str,
) -> Option<TokenOutcome> {
    let token = token?;
    let auth = match config.auth.as_str() {
        "inherit" => match provider.token_endpoint_auth_method.as_str() {
            "client_secret_basic" => Rfc7009Auth::Basic,
            _ => Rfc7009Auth::Post,
        },
        "none" => Rfc7009Auth::None,
        "client_id" => Rfc7009Auth::ClientId,
        "basic" => Rfc7009Auth::Basic,
        "post" => Rfc7009Auth::Post,
        unknown => {
            tracing::warn!(
                provider_slug = %provider.slug,
                revocation_auth = %unknown,
                "Skipping unsupported RFC 7009 client authentication mode"
            );
            return Some(TokenOutcome::Skipped("unsupported_auth"));
        }
    };

    if auth == Rfc7009Auth::ClientId && creds.is_none() {
        return Some(TokenOutcome::Skipped("no_credentials"));
    }

    let mut params = vec![
        ("token".to_string(), token.to_string()),
        ("token_type_hint".to_string(), token_type_hint.to_string()),
    ];
    let mut request = REVOCATION_CLIENT.post(&config.url);

    match (auth, creds) {
        (Rfc7009Auth::Basic, Some(creds)) => {
            request = request.basic_auth(&creds.client_id, creds.client_secret.as_deref());
        }
        (Rfc7009Auth::Post, Some(creds)) => {
            params.push((
                oauth_flow::client_id_param_name(provider).to_string(),
                creds.client_id.clone(),
            ));
            if let Some(secret) = &creds.client_secret {
                params.push(("client_secret".to_string(), secret.clone()));
            }
        }
        (Rfc7009Auth::ClientId, Some(creds)) => params.push((
            oauth_flow::client_id_param_name(provider).to_string(),
            creds.client_id.clone(),
        )),
        _ => {}
    }

    request = request.form(&params);
    Some(match send_revocation_request(request).await {
        Ok(response) if response.status.is_success() => TokenOutcome::Delivered,
        Ok(_) | Err(_) => TokenOutcome::SendFailed,
    })
}

async fn revoke_github_token(
    config: &RevocationConfig,
    scope: RevocationScope,
    creds: Option<&ResolvedOAuthCredentials>,
    token: Option<&str>,
) -> Option<TokenOutcome> {
    let token = token?;
    let Some(creds) = creds.filter(|creds| creds.client_secret.is_some()) else {
        return Some(TokenOutcome::Skipped("no_credentials"));
    };
    let Some(url) = github_revocation_url(&config.url, &creds.client_id, scope) else {
        return Some(TokenOutcome::SendFailed);
    };

    let request = REVOCATION_CLIENT
        .request(Method::DELETE, url)
        .basic_auth(&creds.client_id, creds.client_secret.as_deref())
        .header(USER_AGENT, GITHUB_USER_AGENT)
        .header(ACCEPT, GITHUB_ACCEPT)
        .json(&serde_json::json!({ "access_token": token }));

    Some(match send_revocation_request(request).await {
        Ok(response) if response.status == StatusCode::NO_CONTENT => TokenOutcome::Delivered,
        Ok(response) if response.status == StatusCode::NOT_FOUND => TokenOutcome::NotFound,
        Ok(_) | Err(_) => TokenOutcome::SendFailed,
    })
}

fn github_revocation_url(
    base: &str,
    client_id: &str,
    scope: RevocationScope,
) -> Option<reqwest::Url> {
    let mut url = reqwest::Url::parse(base).ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .push(client_id)
        .push(scope.as_str());
    Some(url)
}

async fn revoke_self_bearer(
    config: &RevocationConfig,
    token: Option<&str>,
) -> Option<TokenOutcome> {
    let token = token?;
    let request = REVOCATION_CLIENT.post(&config.url).bearer_auth(token);
    Some(match send_revocation_request(request).await {
        Ok(response) if response.status.is_success() => {
            match serde_json::from_slice::<serde_json::Value>(&response.body) {
                Ok(body) if body.get("ok").and_then(serde_json::Value::as_bool) != Some(false) => {
                    TokenOutcome::Delivered
                }
                Ok(_) | Err(_) => TokenOutcome::SendFailed,
            }
        }
        Ok(_) | Err(_) => TokenOutcome::SendFailed,
    })
}

async fn revoke_facebook_grant(
    config: &RevocationConfig,
    creds: Option<&ResolvedOAuthCredentials>,
    token: Option<&str>,
) -> Option<TokenOutcome> {
    let token = token?;
    let mut request = REVOCATION_CLIENT.delete(&config.url).bearer_auth(token);
    if let Some(secret) = creds.and_then(|creds| creds.client_secret.as_deref()) {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .expect("HMAC accepts keys of any size");
        mac.update(token.as_bytes());
        let proof = hex::encode(mac.finalize().into_bytes());
        request = request.form(&[("appsecret_proof", proof)]);
    }

    Some(match send_revocation_request(request).await {
        Ok(response) if response.status.is_success() => {
            match serde_json::from_slice::<serde_json::Value>(&response.body) {
                Ok(body)
                    if body.get("success").and_then(serde_json::Value::as_bool) != Some(false) =>
                {
                    TokenOutcome::Delivered
                }
                Ok(_) | Err(_) => TokenOutcome::SendFailed,
            }
        }
        Ok(_) | Err(_) => TokenOutcome::SendFailed,
    })
}

struct RevocationHttpResponse {
    status: StatusCode,
    body: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
enum RedactedSendFailure {
    Timeout,
    Connect,
    Request,
    Body,
    ResponseTooLarge,
    Other,
}

impl RedactedSendFailure {
    fn from_reqwest(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else if error.is_connect() {
            Self::Connect
        } else if error.is_request() {
            Self::Request
        } else if error.is_body() || error.is_decode() {
            Self::Body
        } else {
            Self::Other
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::Request => "request",
            Self::Body => "body",
            Self::ResponseTooLarge => "response_too_large",
            Self::Other => "other",
        }
    }
}

async fn send_revocation_request(
    request: reqwest::RequestBuilder,
) -> Result<RevocationHttpResponse, RedactedSendFailure> {
    let mut response = request.send().await.map_err(|error| {
        let redacted = RedactedSendFailure::from_reqwest(&error);
        tracing::warn!(
            failure_kind = redacted.as_str(),
            "OAuth revocation request failed"
        );
        redacted
    })?;
    let status = response.status();
    let mut body = Vec::new();

    loop {
        let chunk = response.chunk().await.map_err(|error| {
            let redacted = RedactedSendFailure::from_reqwest(&error);
            tracing::warn!(
                failure_kind = redacted.as_str(),
                "OAuth revocation response read failed"
            );
            redacted
        })?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > MAX_REVOCATION_RESPONSE_BODY_SIZE {
            tracing::warn!(
                failure_kind = RedactedSendFailure::ResponseTooLarge.as_str(),
                "OAuth revocation response exceeded size limit"
            );
            return Err(RedactedSendFailure::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }

    Ok(RevocationHttpResponse { status, body })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::{Request, State};
    use axum::http::{HeaderMap, Response};
    use axum::routing::any;
    use chrono::Utc;
    use tokio::sync::mpsc;

    use super::*;

    #[derive(Debug)]
    struct CapturedRequest {
        method: Method,
        uri: String,
        headers: HeaderMap,
        body: Vec<u8>,
    }

    #[derive(Clone)]
    struct CaptureState {
        sender: mpsc::UnboundedSender<CapturedRequest>,
        status: StatusCode,
        body: Vec<u8>,
    }

    struct TestServer {
        base_url: String,
        receiver: mpsc::UnboundedReceiver<CapturedRequest>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn capture_request(
        State(state): State<CaptureState>,
        request: Request,
    ) -> Response<Body> {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, usize::MAX)
            .await
            .expect("read test request body")
            .to_vec();
        state
            .sender
            .send(CapturedRequest {
                method: parts.method,
                uri: parts.uri.to_string(),
                headers: parts.headers,
                body,
            })
            .expect("test receiver should remain open");

        Response::builder()
            .status(state.status)
            .header("content-type", "application/json")
            .body(Body::from(state.body))
            .expect("build test response")
    }

    async fn spawn_server(status: StatusCode, body: impl Into<Vec<u8>>) -> TestServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = listener.local_addr().expect("test listener address");
        let (sender, receiver) = mpsc::unbounded_channel();
        let state = CaptureState {
            sender,
            status,
            body: body.into(),
        };
        let app = Router::new()
            .fallback(any(capture_request))
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server should run");
        });
        TestServer {
            base_url: format!("http://{address}"),
            receiver,
            task,
        }
    }

    fn provider(style: &str, url: String, auth: &str, revokes_grant: bool) -> ProviderConfig {
        ProviderConfig {
            id: "provider-id".to_string(),
            slug: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://example.com/authorize".to_string()),
            token_url: Some("https://example.com/token".to_string()),
            revocation_url: None,
            revocation: Some(RevocationConfig {
                style: style.to_string(),
                url,
                auth: auth.to_string(),
                revokes_grant,
            }),
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "test".to_string(),
            revocation_seed_version: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn credentials() -> ResolvedOAuthCredentials {
        ResolvedOAuthCredentials {
            client_id: "client-id".to_string(),
            client_secret: Some("client-secret".to_string()),
            credential_user_id: None,
        }
    }

    fn request<'a>(
        provider: &'a ProviderConfig,
        scope: RevocationScope,
        creds: Option<ResolvedOAuthCredentials>,
        access_token: Option<&str>,
        refresh_token: Option<&str>,
    ) -> RevocationRequest<'a> {
        RevocationRequest {
            provider,
            scope,
            creds,
            access_token: access_token.map(|token| Zeroizing::new(token.to_string())),
            refresh_token: refresh_token.map(|token| Zeroizing::new(token.to_string())),
        }
    }

    fn form_fields(body: &[u8]) -> HashMap<String, String> {
        url::form_urlencoded::parse(body)
            .into_owned()
            .collect::<HashMap<_, _>>()
    }

    #[test]
    fn effective_revocation_lifts_legacy_url_without_mutating_provider() {
        let mut provider = provider(
            "rfc7009",
            "https://example.com/structured".to_string(),
            "none",
            false,
        );
        provider.revocation = None;
        provider.revocation_url = Some("https://example.com/legacy".to_string());

        let effective = effective_revocation(&provider).expect("legacy URL should be lifted");
        assert_eq!(effective.style, "rfc7009");
        assert_eq!(effective.url, "https://example.com/legacy");
        assert_eq!(effective.auth, "inherit");
        assert!(!effective.revokes_grant);
        assert!(provider.revocation.is_none());
    }

    #[tokio::test]
    async fn rfc7009_honors_auth_modes_and_never_places_tokens_in_urls() {
        let mut server = spawn_server(StatusCode::OK, Vec::new()).await;
        let mut provider = provider(
            "rfc7009",
            format!("{}/revoke", server.base_url),
            "basic",
            false,
        );

        let outcome = revoke_remote(request(
            &provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("access-secret"),
            None,
        ))
        .await;
        assert_eq!(outcome.access, Some(TokenOutcome::Delivered));
        let captured = server.receiver.recv().await.expect("basic request");
        assert_eq!(captured.method, Method::POST);
        assert_eq!(captured.uri, "/revoke");
        assert_eq!(
            captured.headers[reqwest::header::AUTHORIZATION],
            "Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ="
        );
        let fields = form_fields(&captured.body);
        assert_eq!(fields["token"], "access-secret");
        assert_eq!(fields["token_type_hint"], "access_token");
        assert!(!fields.contains_key("client_id"));

        provider.revocation.as_mut().expect("config").auth = "post".to_string();
        revoke_remote(request(
            &provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("post-secret"),
            None,
        ))
        .await;
        let captured = server.receiver.recv().await.expect("post request");
        assert!(
            captured
                .headers
                .get(reqwest::header::AUTHORIZATION)
                .is_none()
        );
        assert!(!captured.uri.contains("post-secret"));
        let fields = form_fields(&captured.body);
        assert_eq!(fields["client_id"], "client-id");
        assert_eq!(fields["client_secret"], "client-secret");

        provider.revocation.as_mut().expect("config").auth = "client_id".to_string();
        provider.client_id_param_name = Some("client_key".to_string());
        revoke_remote(request(
            &provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("client-id-secret"),
            None,
        ))
        .await;
        let captured = server.receiver.recv().await.expect("client-id request");
        let fields = form_fields(&captured.body);
        assert_eq!(fields["client_key"], "client-id");
        assert!(!fields.contains_key("client_secret"));

        provider.revocation.as_mut().expect("config").auth = "none".to_string();
        revoke_remote(request(
            &provider,
            RevocationScope::Token,
            None,
            Some("no-auth-secret"),
            None,
        ))
        .await;
        let captured = server.receiver.recv().await.expect("no-auth request");
        let fields = form_fields(&captured.body);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields["token"], "no-auth-secret");

        provider.revocation.as_mut().expect("config").auth = "basic".to_string();
        revoke_remote(request(
            &provider,
            RevocationScope::Token,
            None,
            Some("degraded-secret"),
            None,
        ))
        .await;
        let captured = server.receiver.recv().await.expect("degraded request");
        assert!(
            captured
                .headers
                .get(reqwest::header::AUTHORIZATION)
                .is_none()
        );
        let fields = form_fields(&captured.body);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields["token"], "degraded-secret");

        provider.revocation.as_mut().expect("config").auth = "inherit".to_string();
        provider.token_endpoint_auth_method = "client_secret_basic".to_string();
        revoke_remote(request(
            &provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("inherit-secret"),
            None,
        ))
        .await;
        let captured = server.receiver.recv().await.expect("inherited request");
        assert!(
            captured.headers[reqwest::header::AUTHORIZATION]
                .to_str()
                .expect("authorization header")
                .starts_with("Basic ")
        );
    }

    #[tokio::test]
    async fn rfc7009_attempts_refresh_after_access_failure() {
        let mut server = spawn_server(StatusCode::BAD_GATEWAY, Vec::new()).await;
        let provider = provider(
            "rfc7009",
            format!("{}/revoke", server.base_url),
            "none",
            false,
        );

        let outcome = revoke_remote(request(
            &provider,
            RevocationScope::Grant,
            None,
            Some("access-token"),
            Some("refresh-token"),
        ))
        .await;

        assert_eq!(outcome.access, Some(TokenOutcome::SendFailed));
        assert_eq!(outcome.refresh, Some(TokenOutcome::SendFailed));
        let access = server.receiver.recv().await.expect("access request");
        let refresh = server.receiver.recv().await.expect("refresh request");
        assert_eq!(form_fields(&access.body)["token_type_hint"], "access_token");
        assert_eq!(
            form_fields(&refresh.body)["token_type_hint"],
            "refresh_token"
        );
    }

    #[tokio::test]
    async fn github_uses_scoped_paths_basic_auth_and_required_headers() {
        let mut server = spawn_server(StatusCode::NO_CONTENT, Vec::new()).await;
        let provider = provider(
            "github",
            format!("{}/applications/", server.base_url),
            "inherit",
            true,
        );

        let grant = revoke_remote(request(
            &provider,
            RevocationScope::Grant,
            Some(credentials()),
            Some("github-access"),
            Some("github-refresh"),
        ))
        .await;
        assert_eq!(grant.access, Some(TokenOutcome::Delivered));
        assert_eq!(grant.refresh, None);
        let captured = server.receiver.recv().await.expect("grant request");
        assert_eq!(captured.method, Method::DELETE);
        assert_eq!(captured.uri, "/applications/client-id/grant");
        assert_eq!(captured.headers[USER_AGENT], GITHUB_USER_AGENT);
        assert_eq!(captured.headers[ACCEPT], GITHUB_ACCEPT);
        assert!(
            captured.headers[reqwest::header::AUTHORIZATION]
                .to_str()
                .expect("authorization header")
                .starts_with("Basic ")
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&captured.body)
                .expect("github request json")["access_token"],
            "github-access"
        );
        assert!(!captured.uri.contains("github-access"));
        assert!(server.receiver.try_recv().is_err());

        let token = revoke_remote(request(
            &provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("github-token-only"),
            None,
        ))
        .await;
        assert_eq!(token.access, Some(TokenOutcome::Delivered));
        let captured = server.receiver.recv().await.expect("token request");
        assert_eq!(captured.uri, "/applications/client-id/token");
    }

    #[tokio::test]
    async fn github_distinguishes_not_found_and_requires_credentials() {
        let mut server = spawn_server(StatusCode::NOT_FOUND, Vec::new()).await;
        let provider = provider(
            "github",
            format!("{}/applications", server.base_url),
            "inherit",
            true,
        );

        let not_found = revoke_remote(request(
            &provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("missing-token"),
            None,
        ))
        .await;
        assert_eq!(not_found.access, Some(TokenOutcome::NotFound));
        server.receiver.recv().await.expect("not-found request");

        let skipped = revoke_remote(request(
            &provider,
            RevocationScope::Token,
            None,
            Some("unattempted-token"),
            None,
        ))
        .await;
        assert_eq!(
            skipped.access,
            Some(TokenOutcome::Skipped("no_credentials"))
        );
        assert!(server.receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn self_bearer_rejects_ok_false_and_keeps_token_out_of_url() {
        let mut server = spawn_server(StatusCode::OK, br#"{"ok":false}"#.to_vec()).await;
        let provider = provider(
            "self_bearer",
            format!("{}/slack/revoke", server.base_url),
            "inherit",
            false,
        );

        let outcome = revoke_remote(request(
            &provider,
            RevocationScope::Token,
            None,
            Some("slack-secret"),
            None,
        ))
        .await;
        assert_eq!(outcome.access, Some(TokenOutcome::SendFailed));
        let captured = server.receiver.recv().await.expect("slack request");
        assert_eq!(captured.method, Method::POST);
        assert_eq!(captured.uri, "/slack/revoke");
        assert_eq!(
            captured.headers[reqwest::header::AUTHORIZATION],
            "Bearer slack-secret"
        );
        assert!(captured.body.is_empty());
    }

    #[tokio::test]
    async fn facebook_token_scope_skips_and_grant_uses_bearer_with_proof() {
        let mut server = spawn_server(StatusCode::OK, br#"{"success":true}"#.to_vec()).await;
        let provider = provider(
            "facebook_deauth",
            format!("{}/me/permissions", server.base_url),
            "inherit",
            true,
        );

        let skipped = revoke_remote(request(
            &provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("facebook-secret"),
            None,
        ))
        .await;
        assert_eq!(skipped.access, Some(TokenOutcome::Skipped("grant_only")));
        assert!(server.receiver.try_recv().is_err());

        let delivered = revoke_remote(request(
            &provider,
            RevocationScope::Grant,
            Some(credentials()),
            Some("facebook-secret"),
            None,
        ))
        .await;
        assert_eq!(delivered.access, Some(TokenOutcome::Delivered));
        let captured = server.receiver.recv().await.expect("facebook request");
        assert_eq!(captured.method, Method::DELETE);
        assert_eq!(captured.uri, "/me/permissions");
        assert_eq!(
            captured.headers[reqwest::header::AUTHORIZATION],
            "Bearer facebook-secret"
        );
        let fields = form_fields(&captured.body);
        let proof = fields.get("appsecret_proof").expect("appsecret proof");
        assert_eq!(proof.len(), 64);
        assert!(proof.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn oversized_response_and_unknown_style_fail_without_leaking_or_sending() {
        let mut oversized_server = spawn_server(
            StatusCode::OK,
            vec![b'x'; MAX_REVOCATION_RESPONSE_BODY_SIZE + 1],
        )
        .await;
        let oversized_provider = provider(
            "rfc7009",
            format!("{}/revoke", oversized_server.base_url),
            "none",
            false,
        );
        let outcome = revoke_remote(request(
            &oversized_provider,
            RevocationScope::Token,
            None,
            Some("oversized-secret"),
            None,
        ))
        .await;
        assert_eq!(outcome.access, Some(TokenOutcome::SendFailed));
        let captured = oversized_server
            .receiver
            .recv()
            .await
            .expect("oversized request");
        assert!(!captured.uri.contains("oversized-secret"));

        let mut unused_server = spawn_server(StatusCode::OK, Vec::new()).await;
        let unknown_provider = provider(
            "future_style",
            format!("{}/must-not-run", unused_server.base_url),
            "inherit",
            false,
        );
        let skipped = revoke_remote(request(
            &unknown_provider,
            RevocationScope::Token,
            Some(credentials()),
            Some("unknown-secret"),
            None,
        ))
        .await;
        assert_eq!(
            skipped.access,
            Some(TokenOutcome::Skipped("unsupported_style"))
        );
        assert!(unused_server.receiver.try_recv().is_err());
    }
}
