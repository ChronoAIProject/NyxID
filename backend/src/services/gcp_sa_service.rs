//! Google Cloud service-account credentials for the proxy.
//!
//! User OAuth tokens for Google Cloud Platform scopes (BigQuery, Cloud
//! Billing) cannot be refreshed unattended: Google enforces a ~16-hour
//! session-length reauthentication policy on those sensitive scopes and
//! rejects `grant_type=refresh_token` with `invalid_grant` /
//! `error_subtype=invalid_rapt` once the session lapses. A
//! `refresh_token` grant has no way to carry the interactive reauth proof
//! (RAPT) Google demands, so no amount of proactive refreshing keeps such
//! a credential alive (see `user_token_service::refresh_expiring_oauth_keys`).
//!
//! Service accounts are Google's sanctioned mechanism for unattended,
//! server-to-server Cloud API access. They are NOT subject to session
//! reauth: NyxID signs a short-lived JWT with the service-account private
//! key and exchanges it for a 1-hour access token via
//! `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer`. The token is
//! cached on the `UserApiKey` row (`access_token_encrypted` + `expires_at`)
//! exactly like an OAuth access token and re-minted lazily at proxy time
//! when it nears expiry — so it renews forever with no human in the loop.
//!
//! The durable secret (the service-account JSON key) lives encrypted in
//! `UserApiKey.credential_encrypted`; the minted access token lives in
//! `access_token_encrypted`. The mint flow mirrors the FCM token exchange
//! in `push_service.rs`.

use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};

/// Default OAuth scope minted for GCP service-account tokens. The service
/// account's IAM role bindings are the real authorization boundary; the
/// token scope only has to be broad enough to reach the Cloud APIs.
/// Callers may override per-key via `UserApiKey.token_scopes`.
pub const DEFAULT_GCP_SA_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// Google's default OAuth 2.0 token endpoint. A service-account key file
/// carries its own `token_uri` (always this value in practice); we honor
/// the key's value so tests can point it at a local server.
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

/// Parsed Google service-account JSON key. Only the fields we need to
/// mint an access token; any other fields in the key file are ignored.
#[derive(Deserialize)]
struct GcpServiceAccountKey {
    client_email: String,
    private_key: String,
    #[serde(default)]
    token_uri: Option<String>,
}

impl std::fmt::Debug for GcpServiceAccountKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never log the private key.
        f.debug_struct("GcpServiceAccountKey")
            .field("client_email", &self.client_email)
            .field("private_key", &"[REDACTED]")
            .field("token_uri", &self.token_uri)
            .finish()
    }
}

/// JWT claims for the service-account assertion.
#[derive(serde::Serialize)]
struct Assertion<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// A freshly minted Google access token.
pub struct MintedToken {
    pub access_token: String,
    /// Lifetime in seconds reported by Google (defaults to 3600 if absent).
    pub expires_in_secs: i64,
}

impl std::fmt::Debug for MintedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the access token (e.g. via test `unwrap_err`).
        f.debug_struct("MintedToken")
            .field("access_token", &"[REDACTED]")
            .field("expires_in_secs", &self.expires_in_secs)
            .finish()
    }
}

/// Why a mint attempt failed, classified the same way the OAuth refresh
/// path classifies token-endpoint failures: terminal errors are a
/// configuration problem (bad key, disabled service account, malformed
/// JSON) and should mark the credential `failed`; transient errors (5xx /
/// 429 / network) leave it usable for a later retry.
#[derive(Debug)]
pub enum MintError {
    Terminal(String),
    Transient(String),
}

impl std::fmt::Display for MintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MintError::Terminal(msg) | MintError::Transient(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for MintError {}

/// Mint a Google access token from a service-account JSON key by signing a
/// JWT and exchanging it via the `jwt-bearer` grant. Pure (no DB / no
/// caching); callers decide how to store the result.
/// Resolve and validate the OAuth token endpoint. Only Google's canonical
/// endpoint is accepted in production; any other value (a malicious key
/// file pointing at an internal address) is rejected. Tests redirect the
/// mint to a local mock server, allowed only under `cfg(test)`.
fn resolve_token_uri(from_key: Option<&str>) -> Result<String, MintError> {
    let uri = from_key.unwrap_or(DEFAULT_TOKEN_URI);
    if uri == DEFAULT_TOKEN_URI {
        return Ok(uri.to_string());
    }
    #[cfg(test)]
    if uri.starts_with("http://127.0.0.1:") || uri.starts_with("http://localhost:") {
        return Ok(uri.to_string());
    }
    Err(MintError::Terminal(format!(
        "untrusted service account token_uri (must be {DEFAULT_TOKEN_URI})"
    )))
}

pub async fn mint_access_token(sa_json: &str, scope: &str) -> Result<MintedToken, MintError> {
    let sa: GcpServiceAccountKey = serde_json::from_str(sa_json)
        .map_err(|e| MintError::Terminal(format!("invalid service account JSON: {e}")))?;

    if sa.client_email.is_empty() || sa.private_key.is_empty() {
        return Err(MintError::Terminal(
            "service account JSON missing client_email or private_key".to_string(),
        ));
    }

    // SSRF guard: the key file is uploaded by the user, so its `token_uri`
    // is attacker-controlled. Trusting it would let an authenticated user
    // point NyxID's signed request at an internal address (link-local
    // metadata, internal services, localhost). Real Google service-account
    // keys always carry `https://oauth2.googleapis.com/token`, so we pin to
    // it and reject anything else — losing nothing legitimate.
    let token_uri = resolve_token_uri(sa.token_uri.as_deref())?;

    let now = Utc::now();
    let assertion_claims = Assertion {
        iss: &sa.client_email,
        scope,
        aud: &token_uri,
        iat: now.timestamp(),
        exp: (now + Duration::hours(1)).timestamp(),
    };

    // Service-account private keys are PKCS#8 RSA PEM ("BEGIN PRIVATE
    // KEY"); `from_rsa_pem` accepts them (same call push_service uses for
    // FCM). A bad key is a permanent configuration error, not transient.
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(sa.private_key.as_bytes())
        .map_err(|e| MintError::Terminal(format!("invalid service account private key: {e}")))?;
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    let assertion = jsonwebtoken::encode(&header, &assertion_claims, &key)
        .map_err(|e| MintError::Terminal(format!("failed to sign assertion: {e}")))?;

    let response = crate::services::oauth_flow::token_exchange_client()
        .post(&token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .send()
        .await
        // Network errors are transient — the key is still valid.
        .map_err(|e| MintError::Transient(format!("token request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let truncated: String = body.chars().take(300).collect();
        // 5xx / 429 mean the token endpoint is momentarily unavailable;
        // the service-account key is unaffected. Everything else (4xx:
        // invalid_grant for a disabled/deleted SA, invalid_client, etc.)
        // is a terminal configuration error.
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(MintError::Transient(format!(
                "token endpoint transiently unavailable: {status} {truncated}"
            )));
        }
        return Err(MintError::Terminal(format!(
            "token endpoint rejected service account: {status} {truncated}"
        )));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        #[serde(default)]
        expires_in: Option<i64>,
    }

    let token: TokenResponse = response
        .json()
        .await
        .map_err(|e| MintError::Transient(format!("failed to parse token response: {e}")))?;

    Ok(MintedToken {
        access_token: token.access_token,
        expires_in_secs: token.expires_in.unwrap_or(3600),
    })
}

/// Mint a fresh access token for a `gcp_service_account` `UserApiKey` and
/// persist it on the row (`access_token_encrypted` + `expires_at`,
/// `status: "active"`). Mirrors `user_token_service::refresh_user_api_key_in_place`:
///
/// - **Terminal failure** (bad key / disabled SA / 4xx): writes
///   `status: "failed"` + `error_message` so the dashboard surfaces the
///   broken credential, and returns `Err`.
/// - **Transient failure** (5xx / 429 / network): leaves the row `active`
///   and returns `Err` so the proxy can fall back on any still-valid
///   cached token and a later request retries.
///
/// The success write is guarded by a `status` predicate so it never
/// resurrects a row a concurrent revoke / failure moved to a terminal
/// state. Concurrent successful mints keep last-write-wins — both tokens
/// are valid, so a later one overwriting an earlier one is harmless.
pub async fn mint_and_store(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    api_key: &UserApiKey,
) -> AppResult<UserApiKey> {
    let enc = api_key.credential_encrypted.as_ref().ok_or_else(|| {
        AppError::Internal("GCP service account key has no stored credential".to_string())
    })?;
    let dec = Zeroizing::new(encryption_keys.decrypt(enc).await?);
    let sa_json = String::from_utf8((*dec).clone()).map_err(|e| {
        AppError::Internal(format!("Failed to decode GCP service account JSON: {e}"))
    })?;

    let scope = api_key
        .token_scopes
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_GCP_SA_SCOPE);

    let minted = match mint_access_token(&sa_json, scope).await {
        Ok(minted) => minted,
        Err(MintError::Transient(msg)) => {
            tracing::warn!(
                api_key_id = %api_key.id,
                "GCP service account token mint hit a transient error; leaving key active for retry: {msg}"
            );
            return Err(AppError::Internal(format!(
                "GCP token mint transiently failed: {msg}"
            )));
        }
        Err(MintError::Terminal(msg)) => {
            let now = Utc::now();
            let truncated: String = msg.chars().take(300).collect();
            // CAS on `updated_at` + `status`: don't clobber a row a
            // concurrent operation already moved off our snapshot.
            let snapshot_updated_at = bson::DateTime::from_chrono(api_key.updated_at);
            db.collection::<UserApiKey>(USER_API_KEYS)
                .update_one(
                    doc! {
                        "_id": &api_key.id,
                        "updated_at": &snapshot_updated_at,
                        "status": { "$nin": ["revoked", "failed"] },
                    },
                    doc! { "$set": {
                        "status": "failed",
                        "error_message": format!("GCP token mint failed: {truncated}"),
                        "updated_at": bson::DateTime::from_chrono(now),
                    }},
                )
                .await?;
            return Err(AppError::Internal(format!(
                "GCP token mint failed: {truncated}"
            )));
        }
    };

    let now = Utc::now();
    let expires_at = now + Duration::seconds(minted.expires_in_secs);
    let access_enc = encryption_keys
        .encrypt(minted.access_token.as_bytes())
        .await?;

    let write = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .update_one(
            doc! {
                "_id": &api_key.id,
                "status": { "$nin": ["revoked", "failed"] },
            },
            doc! { "$set": {
                "access_token_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: access_enc,
                },
                "expires_at": bson::DateTime::from_chrono(expires_at),
                "status": "active",
                "error_message": bson::Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // A concurrent revoke / terminal-failure moved the row off our
    // snapshot between the read and this write, so the freshly minted
    // token was NOT persisted. Don't report success with an unpersisted
    // token: surface as a (transient-style) error so the proxy falls back
    // and the downstream `status != "active"` check rejects the now-
    // terminal key instead of using a stale token.
    if write.matched_count == 0 {
        return Err(AppError::Internal(
            "GCP SA key changed status concurrently during mint".to_string(),
        ));
    }

    db.collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": &api_key.id })
        .await?
        .ok_or_else(|| AppError::Internal("GCP SA key disappeared after mint".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_api_key::UserApiKey;
    use crate::test_utils::{connect_test_database, test_encryption_keys};
    use uuid::Uuid;

    /// A throwaway 2048-bit RSA private key (PKCS#8 PEM). The mock token
    /// server never verifies the signature, so this only needs to be a
    /// parseable, signable key.
    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCxOfGio6jS5FhN\nxWOq8diF22dhiHhJ+IHxHM7NP40+ljQri6sRnfFzbEZoS2JcXgX7vuWBwjopYgR0\nawMK+fjhOzuy1bEltJ940ZyFgtVIMxgAVosI9fz38faLd1hqc1X/S2KADLYFdt2I\nTucnPg3W5eLlwXrggCBR5TuGBkSGO2uX4H48pZ54vEVrT4APz3GF6kn378lM/04G\nXKfuR3VBCQtQ1N1t+uSDHVEZCOXqnOm1KDgBuBvGCwn+nDAo8X7vSUZ53CvzIsgX\nmHCf7u2cHdYw9LRYlZdMeNuuIRX/2pH5chuIGoVKgywG3svb3/STJG6jT2oUpM7c\nCYu0p7SVAgMBAAECggEAFcPQdZFUy+WIJLDvnxBJb5L03MkGQMtYpfRMP2+lGIEY\n0ho6fZTgkLTE5s0PPNm9MWANzoQ8YVWsx2FXA9OUKZD9MWbF9SP8C7nuV4UsTUwd\nD/mQ5J5VHVwlU5ZqENSuRIaNB73H4t7osPNDtxGLYI9l8KJ0xTpm/bfBuiFt6/AO\nvJoCT12m5gZzF7cLHk3Gb8a9YSlj86rM3eJJF+L0UZ0gpob//RDqnX58SaqeW3sM\nIRXPL9ZHUsKZ9i2Ke68DMox9ACi3gFmnsyaiB0yhBjOiBvTpIjgAT7ucmdFKP15D\ndPgphTxM6cnxGLRE37PiSqW6GDzA7itly8zPRi2OpwKBgQDls4wyeaMNQtgSvXWM\nvSImzgyk7/KagmWtniSYw8Kh8pAL0vHUz38XL8PpVDBTCp8N7pSHnu8brxXOwwYU\n/a9kJzgmncYHogkrcsDXskx4czUx6BO7p8qMBSYh2dCI9iHIJejl3Be+tWmWdEPk\nXn7WCOzq3mzJVfubdMuAqGTpEwKBgQDFhGAJHSMIsEInEHMDCmHy7cX5pDOJoX9K\nB2SjQTpHXmTS6LjrpAFSodyuM3lr/M/coVk8FAwwGfNAlViaEotQBlbkU/HkekkM\n+iNvlMKm8YL2fMpCHQNDI/S9sjiI0Yi7unPFnlbmpCY7NDCWGJsm0x5IsDs4sKfF\nQ8ISheGItwKBgQDgOu3ZODSbdW1InfpqcRctmmdte27wtepcGczP9AnD3e4QHNRG\nUmhWUiKFW9HwvqWWDBiia9wuwjQfqvH8+8iDlGWUDOCMAvnAmDz4Uu2jh5OeLFdX\nEO0A0uXulZqkmOFRaPB5sujbGm0Amm7MOBLJDd15SbgYsv7zOoiOB9S6UQKBgCDZ\nx288nVsQlbARmE9lJq1Uxpyipr+5UIZrfF16t8qu9G3vrvHiMSYhLab7gLJpNdko\nLMNFQlGtvzt6m2Xkt67znvgSziSGAihaYhJo14cUnAeK8cjVMnm0PTxfq+91ihxP\nAnpXv3RU0Nb/8yTDqupmKp9EUFU5bG3uuxSBl+U5AoGBAL+NOw9adup24YiPJ/Gc\nMC3YWJLHTMmWthhQl2zoST3B2qyF59herT0OapF9uvSA/3R7l2/hjY7Y62qHdvlp\nyvwM98ObxwlT/Cip3pDK1E/cek9QwqxyAsRDdy/Tr1PnISowhaNRtv/6yjpjDMRq\n36i//64vyzDNvwtlnvGWhsCs\n-----END PRIVATE KEY-----\n";

    async fn spawn_token_server(
        response: serde_json::Value,
        status: axum::http::StatusCode,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(move || {
                let resp = response.clone();
                async move { (status, axum::Json(resp)) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/token"), handle)
    }

    fn sa_json(token_uri: &str) -> String {
        serde_json::json!({
            "type": "service_account",
            "project_id": "test-project",
            "private_key_id": "abc123",
            "private_key": TEST_PRIVATE_KEY,
            "client_email": "svc@test-project.iam.gserviceaccount.com",
            "client_id": "1234567890",
            "token_uri": token_uri,
        })
        .to_string()
    }

    #[tokio::test]
    async fn mint_access_token_returns_token_on_success() {
        let (token_uri, _handle) = spawn_token_server(
            serde_json::json!({ "access_token": "ya29.minted", "expires_in": 3599 }),
            axum::http::StatusCode::OK,
        )
        .await;

        let minted = mint_access_token(&sa_json(&token_uri), DEFAULT_GCP_SA_SCOPE)
            .await
            .expect("mint should succeed");
        assert_eq!(minted.access_token, "ya29.minted");
        assert_eq!(minted.expires_in_secs, 3599);
    }

    #[tokio::test]
    async fn mint_access_token_defaults_expiry_when_absent() {
        let (token_uri, _handle) = spawn_token_server(
            serde_json::json!({ "access_token": "ya29.noexp" }),
            axum::http::StatusCode::OK,
        )
        .await;

        let minted = mint_access_token(&sa_json(&token_uri), DEFAULT_GCP_SA_SCOPE)
            .await
            .unwrap();
        assert_eq!(minted.expires_in_secs, 3600);
    }

    #[tokio::test]
    async fn mint_access_token_terminal_on_4xx() {
        let (token_uri, _handle) = spawn_token_server(
            serde_json::json!({ "error": "invalid_grant" }),
            axum::http::StatusCode::BAD_REQUEST,
        )
        .await;

        let err = mint_access_token(&sa_json(&token_uri), DEFAULT_GCP_SA_SCOPE)
            .await
            .unwrap_err();
        assert!(matches!(err, MintError::Terminal(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn mint_access_token_transient_on_5xx() {
        let (token_uri, _handle) = spawn_token_server(
            serde_json::json!({ "error": "backend_error" }),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;

        let err = mint_access_token(&sa_json(&token_uri), DEFAULT_GCP_SA_SCOPE)
            .await
            .unwrap_err();
        assert!(matches!(err, MintError::Transient(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn mint_access_token_rejects_malformed_key() {
        let bad = serde_json::json!({
            "client_email": "svc@x.iam.gserviceaccount.com",
            "private_key": "-----BEGIN PRIVATE KEY-----\nnot-a-real-key\n-----END PRIVATE KEY-----\n",
        })
        .to_string();
        let err = mint_access_token(&bad, DEFAULT_GCP_SA_SCOPE)
            .await
            .unwrap_err();
        assert!(matches!(err, MintError::Terminal(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn mint_access_token_rejects_untrusted_token_uri() {
        // SSRF guard: a key file pointing token_uri at a non-Google host
        // (e.g. the cloud metadata endpoint) must be rejected outright,
        // even with an otherwise-valid key.
        let evil = serde_json::json!({
            "client_email": "svc@x.iam.gserviceaccount.com",
            "private_key": TEST_PRIVATE_KEY,
            "token_uri": "http://169.254.169.254/latest/meta-data/",
        })
        .to_string();
        let err = mint_access_token(&evil, DEFAULT_GCP_SA_SCOPE)
            .await
            .unwrap_err();
        assert!(matches!(err, MintError::Terminal(_)), "got {err:?}");
    }

    fn make_gcp_key(sa_json_enc: Vec<u8>) -> UserApiKey {
        let now = Utc::now();
        UserApiKey {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            label: "GCP Service Account".to_string(),
            credential_type: "gcp_service_account".to_string(),
            credential_encrypted: Some(sa_json_enc),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: Some(DEFAULT_GCP_SA_SCOPE.to_string()),
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn mint_and_store_persists_token_and_expiry() {
        let Some(db) = connect_test_database("gcp_mint_and_store_persists").await else {
            eprintln!("skipping mint_and_store test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_uri, _handle) = spawn_token_server(
            serde_json::json!({ "access_token": "ya29.persisted", "expires_in": 3600 }),
            axum::http::StatusCode::OK,
        )
        .await;

        let sa_enc = encryption_keys
            .encrypt(sa_json(&token_uri).as_bytes())
            .await
            .unwrap();
        let key = make_gcp_key(sa_enc);
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();

        let refreshed = mint_and_store(&db, &encryption_keys, &key).await.unwrap();
        assert_eq!(refreshed.status, "active");
        assert!(refreshed.access_token_encrypted.is_some());
        assert!(refreshed.expires_at.is_some_and(|e| e > Utc::now()));

        let decrypted = encryption_keys
            .decrypt(refreshed.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(decrypted).unwrap(), "ya29.persisted");
    }

    #[tokio::test]
    async fn mint_and_store_marks_failed_on_terminal_error() {
        let Some(db) = connect_test_database("gcp_mint_and_store_failed").await else {
            eprintln!("skipping mint_and_store failure test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let (token_uri, _handle) = spawn_token_server(
            serde_json::json!({ "error": "invalid_grant" }),
            axum::http::StatusCode::BAD_REQUEST,
        )
        .await;

        let sa_enc = encryption_keys
            .encrypt(sa_json(&token_uri).as_bytes())
            .await
            .unwrap();
        let key = make_gcp_key(sa_enc);
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();

        let err = mint_and_store(&db, &encryption_keys, &key).await;
        assert!(err.is_err());

        let stored = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, "failed");
        assert!(stored.error_message.is_some());
    }
}
