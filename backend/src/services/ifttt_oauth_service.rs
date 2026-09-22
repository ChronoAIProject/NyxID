//! Register this NyxID installation's IFTTT OAuth client on first connection.

use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};
use zeroize::Zeroizing;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::provider_config::{COLLECTION_NAME, ProviderConfig},
};

pub const PROVIDER_SLUG: &str = "ifttt-mcp";
pub const AUTHORIZE_URL: &str = "https://ifttt.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://ifttt.com/oauth/token";
const REGISTRATION_URL: &str = "https://ifttt.com/oauth/register";

pub async fn validate_credential_route(
    db: &Database,
    provider_id: Option<&str>,
    auth_method: &str,
    base_url: &str,
    node_id: Option<&str>,
) -> AppResult<()> {
    let Some(provider_id) = provider_id else {
        return Ok(());
    };
    let is_ifttt = db
        .collection::<ProviderConfig>(COLLECTION_NAME)
        .count_documents(doc! {"_id": provider_id, "slug": PROVIDER_SLUG})
        .await?
        != 0;
    if is_ifttt {
        validate_ifttt_route(auth_method, base_url, node_id)?;
    }
    Ok(())
}

fn validate_ifttt_route(auth_method: &str, base_url: &str, node_id: Option<&str>) -> AppResult<()> {
    if auth_method != nyxid_service_adapters::ifttt_mcp::AUTH_METHOD
        || base_url != nyxid_service_adapters::ifttt_mcp::BASE_URL
        || node_id.is_some_and(|id| !id.is_empty())
    {
        return Err(AppError::ValidationError(
            "IFTTT OAuth credentials require the fixed IFTTT MCP destination and server routing"
                .into(),
        ));
    }
    Ok(())
}

pub async fn validate_key_route(
    db: &Database,
    key_id: Option<&str>,
    auth_method: &str,
    endpoint_id: &str,
    replacement_url: Option<&str>,
    node_id: Option<&str>,
) -> AppResult<()> {
    use crate::models::{user_api_key, user_endpoint};
    let Some(key_id) = key_id else {
        return Ok(());
    };
    let Some(key) = db
        .collection::<user_api_key::UserApiKey>(user_api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": key_id})
        .await?
    else {
        return Ok(());
    };
    let Some(provider_id) = key.provider_config_id.as_deref() else {
        return Ok(());
    };
    if db
        .collection::<ProviderConfig>(COLLECTION_NAME)
        .count_documents(doc! {"_id": provider_id, "slug": PROVIDER_SLUG})
        .await?
        == 0
    {
        return Ok(());
    }
    let endpoint;
    let url = if let Some(url) = replacement_url {
        url
    } else {
        endpoint = db
            .collection::<user_endpoint::UserEndpoint>(user_endpoint::COLLECTION_NAME)
            .find_one(doc! {"_id": endpoint_id})
            .await?
            .ok_or_else(|| AppError::NotFound("Endpoint not found".into()))?;
        &endpoint.url
    };
    validate_ifttt_route(auth_method, url, node_id)
}

pub fn is_managed_provider(provider: &ProviderConfig) -> bool {
    provider.slug == PROVIDER_SLUG
        && provider.provider_type == "oauth2"
        && provider.authorization_url.as_deref() == Some(AUTHORIZE_URL)
        && provider.token_url.as_deref() == Some(TOKEN_URL)
        && provider.credential_mode == "admin"
        && provider.supports_pkce
        && provider.token_endpoint_auth_method == "client_secret_post"
}

pub async fn ensure_registered(
    db: &Database,
    keys: &EncryptionKeys,
    base_url: &str,
    provider: ProviderConfig,
) -> AppResult<ProviderConfig> {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        registration_client(reqwest::Client::builder()).expect("IFTTT registration client")
    });
    ensure_registered_with_client(db, keys, base_url, provider, &CLIENT).await
}

async fn ensure_registered_with_client(
    db: &Database,
    keys: &EncryptionKeys,
    base_url: &str,
    provider: ProviderConfig,
    client: &reqwest::Client,
) -> AppResult<ProviderConfig> {
    if !is_managed_provider(&provider)
        || provider.client_id_encrypted.is_some()
        || provider.client_secret_encrypted.is_some()
    {
        return Ok(provider);
    }
    let callback = format!(
        "{}/api/v1/providers/callback",
        base_url.trim_end_matches('/')
    );
    let url = reqwest::Url::parse(&callback)
        .map_err(|_| AppError::Internal("Invalid OAuth callback URL".into()))?;
    if url.scheme() != "https"
        && !(url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        return Err(AppError::BadRequest("IFTTT OAuth requires an HTTPS callback URL (HTTP is allowed for localhost development)".into()));
    }
    let lease = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let collection = db.collection::<ProviderConfig>(COLLECTION_NAME);
    let claimed = collection.update_one(doc! {
        "_id": &provider.id, "is_active": true,
        "updated_at": bson::DateTime::from_chrono(provider.updated_at),
        "client_id_encrypted": bson::Bson::Null, "client_secret_encrypted": bson::Bson::Null,
        "$or": [
            {"oauth_registration_until": {"$exists": false}},
            {"oauth_registration_until": {"$lt": bson::DateTime::from_chrono(now)}}
        ],
    }, doc! {"$set": {
        "oauth_registration_lease": &lease,
        "oauth_registration_until": bson::DateTime::from_chrono(now + chrono::Duration::seconds(90)),
    }}).await?;
    if claimed.modified_count == 0 {
        let latest = collection
            .find_one(doc! {"_id": &provider.id, "is_active": true})
            .await?
            .ok_or_else(|| AppError::NotFound("IFTTT provider unavailable".into()))?;
        if latest.client_id_encrypted.is_some() && latest.client_secret_encrypted.is_some() {
            return Ok(latest);
        }
        return Err(AppError::Conflict(
            "IFTTT connection setup is busy or temporarily unavailable. Try connecting again shortly.".into(),
        ));
    }
    let result = register(client, &callback).await;
    let result = match result {
        Ok((client_id, secret)) => async {
            let cid = keys.encrypt(client_id.as_bytes()).await?;
            let sec = keys.encrypt(secret.as_bytes()).await?;
            let saved = collection.update_one(doc! {
                "_id": &provider.id, "is_active": true, "oauth_registration_lease": &lease,
                "updated_at": bson::DateTime::from_chrono(provider.updated_at),
                "slug": PROVIDER_SLUG, "provider_type": "oauth2", "credential_mode": "admin",
                "supports_pkce": true, "token_endpoint_auth_method": "client_secret_post",
                "authorization_url": AUTHORIZE_URL, "token_url": TOKEN_URL,
                "client_id_encrypted": bson::Bson::Null, "client_secret_encrypted": bson::Bson::Null,
            }, doc! {"$set": {
                "client_id_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: cid},
                "client_secret_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: sec},
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }}).await?;
            if saved.modified_count != 1 {
                return Err(AppError::Conflict("IFTTT provider configuration changed during setup. Connect again.".into()));
            }
            collection.find_one(doc! {"_id": &provider.id, "is_active": true}).await?
                .ok_or_else(|| AppError::NotFound("IFTTT provider unavailable".into()))
        }.await,
        Err(error) => Err(error),
    };
    // Keep the failed attempt's lease until expiry to bound registration retries.
    if result.is_ok() {
        let _ = collection
            .update_one(
                doc! {"_id": &provider.id, "oauth_registration_lease": &lease},
                doc! {"$unset": {"oauth_registration_lease": "", "oauth_registration_until": ""}},
            )
            .await;
    }
    result
}

async fn register(
    client: &reqwest::Client,
    callback: &str,
) -> AppResult<(Zeroizing<String>, Zeroizing<String>)> {
    let mut response = client
        .post(REGISTRATION_URL)
        .json(&serde_json::json!({
            "client_name": "NyxID",
            "redirect_uris": [callback],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "client_secret_post",
            "scope": "mcp"
        }))
        .send()
        .await
        .map_err(|_| registration_error())?;
    if !response.status().is_success() {
        return Err(registration_error());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| registration_error())? {
        if bytes.len() + chunk.len() > 32 * 1024 {
            return Err(registration_error());
        }
        bytes.extend_from_slice(&chunk);
    }
    #[derive(serde::Deserialize)]
    struct Registration {
        client_id: Zeroizing<String>,
        client_secret: Zeroizing<String>,
        token_endpoint_auth_method: Option<String>,
        redirect_uris: Vec<String>,
        client_secret_expires_at: Option<i64>,
    }
    let registered: Registration =
        serde_json::from_slice(&bytes).map_err(|_| registration_error())?;
    if registered.client_id.is_empty()
        || registered.client_secret.is_empty()
        || registered
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|m| m != "client_secret_post")
        || registered.redirect_uris != [callback]
        || registered
            .client_secret_expires_at
            .is_some_and(|expiry| expiry != 0)
    {
        return Err(registration_error());
    }
    Ok((registered.client_id, registered.client_secret))
}

fn registration_client(builder: reqwest::ClientBuilder) -> Result<reqwest::Client, reqwest::Error> {
    builder
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

fn registration_error() -> AppError {
    AppError::BadRequest("IFTTT OAuth client registration failed. An administrator can register the NyxID callback with IFTTT and configure the provider's OAuth client credentials.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyxid_service_adapters::test_support::scripted_fixture;

    const CALLBACK: &str = "https://nyxid.example/api/v1/providers/callback";

    #[tokio::test]
    async fn ifttt_registration_is_leased_encrypted_and_preserved_across_seeding() {
        use crate::test_utils::{connect_test_database, test_encryption_keys};
        let db = connect_test_database("ifttt_registration").await.unwrap();
        let keys = test_encryption_keys();
        super::super::provider_service::seed_default_providers(&db, &keys)
            .await
            .unwrap();
        let collection = db.collection::<ProviderConfig>(COLLECTION_NAME);
        let provider = collection
            .find_one(doc! {"slug": PROVIDER_SLUG})
            .await
            .unwrap()
            .unwrap();
        assert!(is_managed_provider(&provider));
        let payload = serde_json::json!({"client_id":"fixture-client", "client_secret":"fixture-secret", "redirect_uris":[CALLBACK]}).to_string();
        let mut fixture = scripted_fixture("ifttt.com", vec![format!("HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len())]).await;
        let client = registration_client(fixture.builder.take().unwrap()).unwrap();
        let (first, second) = tokio::join!(
            ensure_registered_with_client(
                &db,
                &keys,
                "https://nyxid.example",
                provider.clone(),
                &client
            ),
            ensure_registered_with_client(
                &db,
                &keys,
                "https://nyxid.example",
                provider.clone(),
                &client
            ),
        );
        assert!(first.is_ok() || second.is_ok());
        for result in [first, second] {
            if let Err(error) = result {
                assert!(matches!(error, AppError::Conflict(_)));
            }
        }
        let saved = collection
            .find_one(doc! {"_id": &provider.id})
            .await
            .unwrap()
            .unwrap();
        let encrypted_id = saved.client_id_encrypted.as_ref().unwrap();
        let encrypted_secret = saved.client_secret_encrypted.as_ref().unwrap();
        assert_ne!(encrypted_secret.as_slice(), b"fixture-secret");
        assert_eq!(keys.decrypt(encrypted_id).await.unwrap(), b"fixture-client");
        assert_eq!(
            keys.decrypt(encrypted_secret).await.unwrap(),
            b"fixture-secret"
        );
        let document = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &provider.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!document.contains_key("oauth_registration_lease"));
        super::super::provider_service::seed_default_providers(&db, &keys)
            .await
            .unwrap();
        let again = collection
            .find_one(doc! {"_id": &provider.id})
            .await
            .unwrap()
            .unwrap();
        let again =
            ensure_registered_with_client(&db, &keys, "https://nyxid.example", again, &client)
                .await
                .unwrap();
        assert_eq!(again.client_secret_encrypted, saved.client_secret_encrypted);
        fixture.requests.recv().await.unwrap();
        assert!(fixture.requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn ifttt_failed_registration_retains_a_cooldown() {
        use crate::test_utils::{connect_test_database, test_encryption_keys};
        let db = connect_test_database("ifttt_registration_cooldown")
            .await
            .unwrap();
        let keys = test_encryption_keys();
        super::super::provider_service::seed_default_providers(&db, &keys)
            .await
            .unwrap();
        let provider = db
            .collection::<ProviderConfig>(COLLECTION_NAME)
            .find_one(doc! {"slug":PROVIDER_SLUG})
            .await
            .unwrap()
            .unwrap();
        let mut fixture = scripted_fixture("ifttt.com", vec!["HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()]).await;
        let client = registration_client(fixture.builder.take().unwrap()).unwrap();
        assert!(
            ensure_registered_with_client(
                &db,
                &keys,
                "https://nyxid.example",
                provider.clone(),
                &client
            )
            .await
            .is_err()
        );
        assert!(matches!(
            ensure_registered_with_client(&db, &keys, "https://nyxid.example", provider, &client)
                .await,
            Err(AppError::Conflict(_))
        ));
        fixture.requests.recv().await.unwrap();
        assert!(fixture.requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn ifttt_registration_uses_exact_callback_and_confidential_pkce_client() {
        let payload = serde_json::json!({
            "client_id":"fixture-client", "client_secret":"fixture-secret",
            "token_endpoint_auth_method":"client_secret_post",
            "redirect_uris":[CALLBACK], "client_secret_expires_at":0
        })
        .to_string();
        let mut fixture = scripted_fixture("ifttt.com", vec![format!("HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len())]).await;
        let client = registration_client(fixture.builder.take().unwrap()).unwrap();
        let (id, secret) = register(&client, CALLBACK).await.unwrap();
        assert_eq!(id.as_str(), "fixture-client");
        assert_eq!(secret.as_str(), "fixture-secret");
        let request = fixture.requests.recv().await.unwrap();
        assert!(String::from_utf8_lossy(&request).starts_with("POST /oauth/register "));
        let offset = request.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
        let body: serde_json::Value = serde_json::from_slice(&request[offset..]).unwrap();
        assert_eq!(body["redirect_uris"], serde_json::json!([CALLBACK]));
        assert_eq!(body["scope"], "mcp");
        assert_eq!(body["token_endpoint_auth_method"], "client_secret_post");
        assert_eq!(
            body["grant_types"],
            serde_json::json!(["authorization_code", "refresh_token"])
        );
    }

    #[tokio::test]
    async fn ifttt_registration_rejects_redirects_and_invalid_credentials_without_leaking_response()
    {
        for (status, body) in [
            ("302 Found", "fixture-secret".to_owned()),
            ("201 Created", serde_json::json!({"client_id":"id","client_secret":"fixture-secret","redirect_uris":["https://wrong.example/callback"]}).to_string()),
            ("201 Created", serde_json::json!({"client_id":"id","client_secret":"fixture-secret","redirect_uris":[CALLBACK],"client_secret_expires_at":123}).to_string()),
            ("201 Created", "x".repeat(33 * 1024)),
        ] {
            let mut fixture = scripted_fixture("ifttt.com", vec![format!("HTTP/1.1 {status}\r\nLocation: https://wrong.example\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())]).await;
            let client = registration_client(fixture.builder.take().unwrap()).unwrap();
            let err = register(&client, CALLBACK).await.unwrap_err();
            assert!(!err.to_string().contains("fixture-secret"));
            fixture.requests.recv().await.unwrap();
            assert!(fixture.requests.try_recv().is_err());
        }
    }
}
