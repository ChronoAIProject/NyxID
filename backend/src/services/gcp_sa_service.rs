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

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};

/// Default OAuth scope minted for GCP service-account tokens. The service
/// account's IAM role bindings are the real authorization boundary; the
/// token scope only has to be broad enough to reach the Cloud APIs.
/// Callers may override per-key via `UserApiKey.token_scopes`.
pub const DEFAULT_GCP_SA_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// Google's default OAuth 2.0 token endpoint. A service-account key file
/// carries `token_uri`; production accepts only this canonical endpoint.
/// The private test transport seam permits a loopback mock server.
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

/// Parsed Google service-account JSON key. Only the fields we need to
/// mint an access token; any other fields in the key file are ignored.
#[derive(Deserialize)]
struct GcpServiceAccountKey {
    client_email: zeroize::Zeroizing<String>,
    private_key: zeroize::Zeroizing<String>,
    #[serde(default)]
    token_uri: Option<zeroize::Zeroizing<String>>,
}

impl std::fmt::Debug for GcpServiceAccountKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Key identifiers and unvalidated input URLs may also carry secrets.
        f.debug_struct("GcpServiceAccountKey")
            .field("client_email", &"[REDACTED]")
            .field("private_key", &"[REDACTED]")
            .field("token_uri", &"[REDACTED]")
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

impl Drop for MintedToken {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.access_token.zeroize();
    }
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

fn token_request(sa_json: &str, scope: &str) -> Result<reqwest::RequestBuilder, MintError> {
    let sa: GcpServiceAccountKey = serde_json::from_str(sa_json)
        .map_err(|_| MintError::Terminal("invalid service account JSON".into()))?;

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
    let token_uri = resolve_token_uri(sa.token_uri.as_ref().map(|uri| uri.as_str()))?;

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
        .map_err(|_| MintError::Terminal("invalid service account private key".into()))?;
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    let assertion = zeroize::Zeroizing::new(
        jsonwebtoken::encode(&header, &assertion_claims, &key)
            .map_err(|_| MintError::Terminal("failed to sign assertion".into()))?,
    );

    Ok(crate::services::oauth_flow::token_exchange_client()
        .post(&token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ]))
}
#[cfg(test)]
pub async fn mint_access_token(sa_json: &str, scope: &str) -> Result<MintedToken, MintError> {
    send_token_request(token_request(sa_json, scope)?, None).await
}
/// Every production mint has durable predispatch intent, including validation
/// mints before initial creation. Local parsing/signing failures issue nothing.
pub(crate) async fn mint_for_install(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    actor: &str,
    owner: &str,
    destination: &str,
    sa_json: &str,
    scope: &str,
) -> AppResult<(String, MintedToken)> {
    let request = token_request(sa_json, scope)
        .map_err(|error| AppError::ValidationError(error.to_string()))?;
    let (attempt, canonical) =
        super::personal_erasure_service::issuance::begin_gcp(db, actor, owner, destination).await?;
    let minted = send_token_request(request, Some((db, keys, &attempt, &canonical)))
        .await
        .map_err(|error| match error {
            MintError::Terminal(message) => AppError::ValidationError(message),
            MintError::Transient(message) => AppError::Internal(message),
        })?;
    Ok((attempt, minted))
}
async fn send_token_request(
    request: reqwest::RequestBuilder,
    journal: Option<(&mongodb::Database, &EncryptionKeys, &str, &str)>,
) -> Result<MintedToken, MintError> {
    let response = request
        .send()
        .await
        .map_err(|_| MintError::Transient("token request failed".into()))?;
    let status = response.status();
    let body = zeroize::Zeroizing::new(
        response
            .text()
            .await
            .map_err(|_| MintError::Transient("failed to read token response".into()))?,
    );
    let value: crate::crypto::secret_json::SecretJson = serde_json::from_str(&body)
        .map_err(|_| MintError::Transient("failed to parse token response".into()))?;
    let access = value
        .0
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty());
    // Only an absent lifetime has the protocol default. A present malformed
    // value must not become a live one-hour token. Retain returned material
    // first, with unknown expiry, even when the success envelope is invalid.
    let lifetime = match value.0.get("expires_in") {
        None => Some(3600),
        Some(value) => value
            .as_i64()
            .filter(|seconds| (1..=3600).contains(seconds)),
    };
    let expiry = lifetime.map(|seconds| Utc::now() + Duration::seconds(seconds));
    if let Some((db, keys, attempt, owner)) = journal {
        if let Some(access) = access {
            super::personal_erasure_service::issuance::received_gcp(
                db, keys, attempt, owner, access, expiry,
            )
            .await
            .map_err(|_| {
                MintError::Transient("could not confirm durable issued-token receipt".into())
            })?;
        } else if value
            .0
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|s| !s.is_empty())
            && value
                .0
                .get("access_token")
                .is_none_or(serde_json::Value::is_null)
            && value
                .0
                .get("refresh_token")
                .is_none_or(serde_json::Value::is_null)
        {
            super::personal_erasure_service::issuance::settled(db, attempt, false)
                .await
                .map_err(|_| {
                    MintError::Transient("could not confirm no-issuance receipt".into())
                })?;
        }
    }
    if !status.is_success() {
        let code = super::provider_diagnostics::response_code(&body);
        let message = format!("token endpoint rejected service account: {status} ({code})");
        return Err(
            if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                MintError::Transient(message)
            } else {
                MintError::Terminal(message)
            },
        );
    }
    if expiry.is_none() {
        return Err(MintError::Transient(
            "invalid service account token lifetime".into(),
        ));
    }
    Ok(MintedToken {
        access_token: access
            .ok_or_else(|| MintError::Transient("token response contains no access token".into()))?
            .to_owned(),
        expires_in_secs: lifetime.expect("validated lifetime"),
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
/// Heap-own the credential lifecycle future at the production boundary. Proxy
/// callers and joined lifecycle races must not embed its transaction/HTTP
/// state in every enclosing future's stack frame.
pub fn mint_and_store<'a>(
    db: &'a mongodb::Database,
    encryption_keys: &'a EncryptionKeys,
    api_key: &'a UserApiKey,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<UserApiKey>> + Send + 'a>> {
    Box::pin(mint_and_store_inner(db, encryption_keys, api_key))
}

async fn mint_and_store_inner(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    api_key: &UserApiKey,
) -> AppResult<UserApiKey> {
    let enc = api_key.credential_encrypted.as_ref().ok_or_else(|| {
        AppError::Internal("GCP service account key has no stored credential".to_string())
    })?;
    use super::credential_record_service as records;
    let raw = db
        .collection::<bson::Document>(USER_API_KEYS)
        .find_one(doc! {"_id":&api_key.id})
        .await?
        .ok_or_else(|| {
            AppError::Conflict("Service account credential disappeared before mint".into())
        })?;
    let current: UserApiKey = bson::from_document(raw.clone())
        .map_err(|_| AppError::Internal("Invalid credential storage".into()))?;
    if current.credential_type != "gcp_service_account"
        || current.status != api_key.status
        || !matches!(current.status.as_str(), "active" | "expired")
        || current.updated_at.timestamp_millis() != api_key.updated_at.timestamp_millis()
        || current.user_id != api_key.user_id
        || current.credential_epoch != api_key.credential_epoch
        || current.credential_encrypted != api_key.credential_encrypted
        || current.secret_context_versions != api_key.secret_context_versions
    {
        return Err(AppError::Conflict(
            "Service account credential changed before mint".into(),
        ));
    }
    let predicate = records::exact_source_filter(USER_API_KEYS, &raw)?;
    let sa_json = records::CredentialRecord::key(api_key)
        .text(encryption_keys, "credential_encrypted", enc)
        .await?;

    let scope = api_key
        .token_scopes
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_GCP_SA_SCOPE);

    let lifecycle_owner = super::erasure_admission::credential_owner(db, &api_key.user_id).await?;
    let (attempt, minted) = match mint_for_install(
        db,
        encryption_keys,
        &lifecycle_owner,
        &api_key.user_id,
        &api_key.id,
        &sa_json,
        scope,
    )
    .await
    {
        Ok(minted) => minted,
        Err(AppError::Internal(msg)) => {
            tracing::warn!(
                api_key_id = %api_key.id,
                "GCP service account token mint hit a transient error; leaving key active for retry: {msg}"
            );
            return Err(AppError::Internal(format!(
                "GCP token mint transiently failed: {msg}"
            )));
        }
        Err(AppError::ValidationError(msg)) => {
            let now = Utc::now();
            let truncated: String = msg.chars().take(300).collect();
            // CAS on `updated_at` + `status`: don't clobber a row a
            // concurrent operation already moved off our snapshot.
            crate::services::service_history::collection::<UserApiKey>(db, USER_API_KEYS)
                .update_one(
                    predicate.clone(),
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
        Err(error) => return Err(error),
    };

    let now = Utc::now();
    let expires_at = now + Duration::seconds(minted.expires_in_secs);
    let access_enc = records::encrypt(
        encryption_keys,
        USER_API_KEYS,
        &api_key.id,
        &api_key.user_id,
        "access_token_encrypted",
        minted.access_token.as_bytes(),
    )
    .await?;
    let mut versions = api_key.secret_context_versions.clone();
    records::mark_version(&mut versions, encryption_keys, "access_token_encrypted");

    let set = doc! {
        "access_token_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: access_enc,
        },
        "secret_context_versions":bson::to_bson(&versions).map_err(|_|AppError::Internal("Invalid credential version map".into()))?,
        "expires_at": bson::DateTime::from_chrono(expires_at),
        "status": "active",
        "error_message": bson::Bson::Null,
        "updated_at": bson::DateTime::from_chrono(now),
    };
    use std::future::IntoFuture;
    let write = super::personal_erasure_service::issuance::installing(
        &attempt,
        super::personal_erasure_service::issuance::key_mutation(
            doc! {"_id":&api_key.id},
            &set.clone(),
            &minted.access_token,
            None,
            super::service_history::collection::<UserApiKey>(db, USER_API_KEYS)
                .update_one(predicate, doc! {"$set":set})
                .routine_refresh()
                .into_future(),
        ),
    )
    .await??;

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

    crate::services::service_history::collection::<UserApiKey>(db, USER_API_KEYS)
        .find_one(doc! { "_id": &api_key.id })
        .await?
        .ok_or_else(|| AppError::Internal("GCP SA key disappeared after mint".to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn service_account_debug_redacts_key_identifiers_and_unvalidated_url() {
        let key=super::GcpServiceAccountKey {
            client_email:"distinctive-key-email@example.test".to_owned().into(),
            private_key:"distinctive-private-material".to_owned().into(),
            token_uri:Some("https://distinctive-user:distinctive-password@example.test/token?secret=distinctive-query".to_owned().into()),
        };
        for rendered in [format!("{key:?}"), format!("{key:#?}")] {
            for distinctive in [
                "distinctive-key-email",
                "distinctive-private-material",
                "distinctive-user",
                "distinctive-password",
                "distinctive-query",
                "example.test",
            ] {
                assert!(!rendered.contains(distinctive));
            }
            assert!(rendered.contains("[REDACTED]"));
        }
    }

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
    async fn malformed_success_never_echoes_credential_response_in_mint_error() {
        let secret = "distinctive-gcp-malformed-expiry-6107";
        let (token_uri, server) = spawn_token_server(
            serde_json::json!({"access_token":secret,"expires_in":secret}),
            axum::http::StatusCode::OK,
        )
        .await;
        let error = mint_access_token(&sa_json(&token_uri), DEFAULT_GCP_SA_SCOPE)
            .await
            .err()
            .unwrap();
        assert!(matches!(error, MintError::Transient(_)));
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        assert_eq!(error.to_string(), "invalid service account token lifetime");
        server.abort();
    }

    #[tokio::test]
    async fn present_invalid_lifetime_is_rejected_and_returned_token_is_journaled() {
        use serde_json::json;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let db = crate::test_utils::connect_transaction_test_database("gcp_invalid_expiry").await;
        let keys = test_encryption_keys();
        let owner = Uuid::new_v4().to_string();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        for malformed in [
            json!(null),
            json!("3600"),
            json!(0),
            json!(-1),
            json!(3601),
            json!(3.5),
            json!(true),
        ] {
            let (uri, server) = spawn_token_server(
                json!({"access_token":"retained-invalid-expiry-token","expires_in":malformed}),
                axum::http::StatusCode::OK,
            )
            .await;
            let destination = Uuid::new_v4().to_string();
            assert!(
                mint_for_install(
                    &db,
                    &keys,
                    &owner,
                    &owner,
                    &destination,
                    &sa_json(&uri),
                    DEFAULT_GCP_SA_SCOPE
                )
                .await
                .is_err()
            );
            let receipt = db
                .collection::<bson::Document>("personal_erasure_effects")
                .find_one(doc! {"source_id":destination})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(receipt.get_str("status"), Ok("received"));
            assert!(receipt.get_binary_generic("capability_encrypted").is_ok());
            let raw = keys
                .decrypt_record(
                    receipt.get_binary_generic("capability_encrypted").unwrap(),
                    &crate::crypto::aes::RecordContext::new(
                        "personal_erasure_effects",
                        receipt.get_str("_id").unwrap(),
                        &owner,
                        &["capability_encrypted"],
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
            let capability: crate::crypto::secret_json::SecretJson =
                serde_json::from_slice(&raw).unwrap();
            assert_eq!(
                capability
                    .get("access_token")
                    .and_then(serde_json::Value::as_str),
                Some("retained-invalid-expiry-token")
            );
            assert!(
                capability
                    .get("expires_at")
                    .is_none_or(serde_json::Value::is_null)
            );
            server.abort();
        }
        db.drop().await.unwrap();
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
            secret_context_versions: Default::default(),
            oauth_app_observation: None,
            credential_source: None,
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
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            credential_epoch: 1,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn journaled_mint_installs_for_person_org_and_canonically_owned_service_account() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let db = crate::test_utils::connect_transaction_test_database("gcp_mint_owners").await;
        let keys = test_encryption_keys();
        let (uri, server) = spawn_token_server(
            serde_json::json!({"access_token":"owner-token","expires_in":3600}),
            axum::http::StatusCode::OK,
        )
        .await;
        for kind in ["person", "org", "service_account", "org_service_account"] {
            let canonical = Uuid::new_v4().to_string();
            let is_org = kind.contains("org");
            db.collection::<crate::models::user::User>("users")
                .insert_one(crate::test_utils::test_user(
                    &canonical,
                    if is_org {
                        crate::models::user::UserType::Org
                    } else {
                        crate::models::user::UserType::Person
                    },
                ))
                .await
                .unwrap();
            let subject = if kind.contains("service_account") {
                let subject = Uuid::new_v4().to_string();
                db.collection::<bson::Document>(crate::models::service_account::COLLECTION_NAME).insert_one(doc! {
                    "_id":&subject,"name":"Mint owner","client_id":Uuid::new_v4().to_string(),
                    "client_secret_hash":"unused-hash","secret_prefix":"unused","allowed_scopes":"proxy",
                    "is_active":true,"created_by":&canonical,"owner_user_id":&canonical,
                    "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
                }).await.unwrap();
                subject
            } else {
                canonical.clone()
            };
            let mut key = make_gcp_key(keys.encrypt(sa_json(&uri).as_bytes()).await.unwrap());
            key.user_id = subject.clone();
            db.collection::<UserApiKey>(USER_API_KEYS)
                .insert_one(&key)
                .await
                .unwrap();
            let live = mint_and_store(&db, &keys, &key).await.unwrap();
            assert!(live.provider_config_id.is_none());
            assert_eq!(
                super::super::credential_record_service::CredentialRecord::key(&live)
                    .text(
                        &keys,
                        "access_token_encrypted",
                        live.access_token_encrypted.as_deref().unwrap()
                    )
                    .await
                    .unwrap()
                    .as_str(),
                "owner-token"
            );
            let receipt = db
                .collection::<bson::Document>("personal_erasure_effects")
                .find_one(doc! {"source_id":&key.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(receipt.get_str("user_id"), Ok(canonical.as_str()));
            assert_eq!(receipt.get_str("target_owner_id"), Ok(subject.as_str()));
            assert_eq!(receipt.get_str("reason"), Ok("credential_installed"));
            assert!(!receipt.contains_key("capability_encrypted"));
        }
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn held_gcp_mint_after_erasure_preserves_response_and_supports_truthful_policy_resolution()
     {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let db = crate::test_utils::connect_transaction_test_database("gcp_held_erasure").await;
        super::super::personal_erasure_service::ensure_indexes(&db)
            .await
            .unwrap();
        let keys = test_encryption_keys();
        let state = crate::test_utils::test_app_state(db.clone());
        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let (entered_server, release_server) = (entered.clone(), release.clone());
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(move || {
                let (entered, release) = (entered_server.clone(), release_server.clone());
                async move {
                    entered.notify_one();
                    release.notified().await;
                    axum::Json(
                        serde_json::json!({"access_token":"late-gcp-token","expires_in":3600}),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let key = make_gcp_key(keys.encrypt(sa_json(&uri).as_bytes()).await.unwrap());
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &key.user_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        let mint = mint_and_store(&db, &keys, &key);
        let erase = async {
            entered.notified().await;
            super::super::personal_erasure_service::accept(&db, &key.user_id, &key.user_id, None)
                .await
                .unwrap();
            // Actual production worker may preserve/purge source material; the
            // upstream intent still prevents any completed account claim.
            for _ in 0..8 {
                db.collection::<bson::Document>("personal_erasure_jobs")
                    .update_one(
                        doc! {"user_id":&key.user_id},
                        doc! {"$set":{"retry_at":bson::DateTime::from_millis(0)}},
                    )
                    .await
                    .unwrap();
                super::super::personal_erasure_service::process_one(&state)
                    .await
                    .unwrap();
            }
            assert_ne!(
                super::super::personal_erasure_service::status(&db, &key.user_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                crate::models::personal_erasure::ErasureStatus::Complete
            );
            release.notify_one();
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::join!(mint, erase)
        })
        .await
        .unwrap();
        assert!(result.is_err());
        let receipt = db
            .collection::<bson::Document>("personal_erasure_effects")
            .find_one(doc! {"source_id":&key.id,"kind":"oauth_issuance"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.get_str("status"), Ok("received"));
        assert!(receipt.contains_key("capability_encrypted"));
        let id = receipt.get_str("_id").unwrap();
        db.collection::<bson::Document>("personal_erasure_effects")
            .update_one(
                doc! {"_id":id},
                doc! {"$set":{"updated_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        super::super::personal_erasure_service::process_one(&state)
            .await
            .unwrap();
        super::super::personal_erasure_service::provider::reconcile_effect(
            &db,
            &keys,
            &key.user_id,
            &key.user_id,
            &crate::services::personal_erasure_service::provider::EffectReconciliationRequest {
                effect_id: (id).to_string(),
                action: ("record_remote_revocation_unavailable").to_string(),
                evidence_reference:
                    ("fixture://gcp-individual-revocation-unavailable-and-expiry-policy")
                        .to_string(),
                expected_retry_generation: 0,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        let resolved = db
            .collection::<bson::Document>("personal_erasure_effects")
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.get_bool("remote_revocation_confirmed"), Ok(false));
        assert_eq!(
            resolved.get_str("completion_basis"),
            Ok("operator_recorded_revocation_unavailable")
        );
        assert!(!resolved.contains_key("capability_encrypted"));
        assert!(
            db.collection::<UserApiKey>(USER_API_KEYS)
                .find_one(doc! {"_id":&key.id,"access_token_encrypted":{"$ne":null}})
                .await
                .unwrap()
                .is_none()
        );
        server.abort();
        db.drop().await.unwrap();
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
        crate::test_utils::init_test_audit_chain_hmac_key();
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &key.user_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();

        let mut service = crate::test_utils::test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &key.user_id,
            "gcp-history",
            "endpoint",
            None,
            None,
        );
        service.api_key_id = Some(key.id.clone());
        db.collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(&service)
            .await
            .unwrap();
        let refreshed = mint_and_store(&db, &encryption_keys, &key).await.unwrap();
        assert_eq!(refreshed.status, "active");
        assert!(refreshed.access_token_encrypted.is_some());
        assert!(refreshed.expires_at.is_some_and(|e| e > Utc::now()));

        assert_eq!(
            db.collection::<bson::Document>(crate::models::service_change_event::COLLECTION_NAME)
                .count_documents(doc! { "service_id": &service.id })
                .await
                .unwrap(),
            0
        );
        let decrypted =
            crate::services::credential_record_service::CredentialRecord::key(&refreshed)
                .decrypt(
                    &encryption_keys,
                    "access_token_encrypted",
                    refreshed.access_token_encrypted.as_ref().unwrap(),
                )
                .await
                .unwrap()
                .to_vec();
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
        crate::test_utils::init_test_audit_chain_hmac_key();
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &key.user_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
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
