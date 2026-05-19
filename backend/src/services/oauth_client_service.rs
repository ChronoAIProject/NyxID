use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, Binary, doc, spec::BinarySubtype};
use uuid::Uuid;

use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::consent::COLLECTION_NAME as CONSENTS;
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::refresh_token::COLLECTION_NAME as REFRESH_TOKENS;

/// Known scopes supported by NyxID. Used for validation of
/// `allowed_scopes` on OAuth clients.
///
/// The list mixes OIDC-standard scopes (openid, profile, email, roles,
/// groups) with NyxID-specific extensions (proxy, urn:nyxid:scope:*).
/// `urn:nyxid:scope:broker_binding` opts a client into the OAuth broker
/// pattern when present in their allowed_scopes.
pub const KNOWN_OIDC_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "roles",
    "groups",
    "proxy",
    "urn:nyxid:scope:broker_binding",
];

/// Default allowed scopes for new OAuth clients.
pub const DEFAULT_ALLOWED_SCOPES: &str = "openid profile email";

/// Default scopes for the built-in MCP OAuth client and dynamic registrations.
///
/// Includes `roles` and `groups` so MCP clients (Cursor, Claude Code, Codex,
/// etc.) that request RBAC claims pass scope validation. Token issuance is
/// still gated by what the client requests at `/oauth/authorize` and what the
/// user consents to.
pub const DEFAULT_MCP_ALLOWED_SCOPES: &str = "openid profile email roles groups proxy";

/// Validate and canonicalize `allowed_scopes`.
///
/// - Every scope must be in [`KNOWN_OIDC_SCOPES`].
/// - `openid` is always required (auto-prepended if missing).
/// - Duplicates are removed.
/// - Returns a deduplicated, space-separated string.
pub fn validate_allowed_scopes(scopes: &str) -> AppResult<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for s in scopes.split_whitespace() {
        if !KNOWN_OIDC_SCOPES.contains(&s) {
            return Err(AppError::ValidationError(format!(
                "Unknown OIDC scope '{s}'. Must be one of: {}",
                KNOWN_OIDC_SCOPES.join(", ")
            )));
        }
        if seen.insert(s) {
            out.push(s);
        }
    }

    // openid is mandatory per OIDC spec
    if !seen.contains("openid") {
        out.insert(0, "openid");
    }

    Ok(out.join(" "))
}

/// Validate and canonicalize `allowed_scopes` supplied as an API list.
///
/// An explicit empty list is normalized to `openid`, while omission should be
/// handled by the caller when the endpoint wants to apply
/// [`DEFAULT_ALLOWED_SCOPES`].
pub fn validate_allowed_scopes_list(scopes: &[String]) -> AppResult<String> {
    validate_allowed_scopes(&scopes.join(" "))
}

/// Well-known client ID for native MCP clients (Cursor, Claude Code, etc.).
const MCP_CLIENT_ID: &str = "nyx-mcp";

/// Seed default OAuth clients at startup (idempotent).
///
/// Creates the `nyx-mcp` public client used by MCP desktop apps. The client
/// has no registered redirect URIs because loopback URIs are validated
/// dynamically per RFC 8252 section 7.3.
pub async fn seed_default_clients(db: &mongodb::Database) -> AppResult<()> {
    let collection = db.collection::<OauthClient>(OAUTH_CLIENTS);

    if let Some(existing) = collection.find_one(doc! { "_id": MCP_CLIENT_ID }).await? {
        if let Some(updated_scopes) = merge_missing_default_mcp_scopes(&existing.allowed_scopes)? {
            collection
                .update_one(
                    doc! { "_id": MCP_CLIENT_ID },
                    doc! { "$set": {
                        "allowed_scopes": &updated_scopes,
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }},
                )
                .await?;

            tracing::info!(
                allowed_scopes = %updated_scopes,
                "Upgraded default MCP OAuth client to include latest default scopes"
            );
        }

        return Ok(());
    }

    let now = Utc::now();
    let client = OauthClient {
        id: MCP_CLIENT_ID.to_string(),
        client_name: "NyxID MCP Client".to_string(),
        client_secret_hash: "NONE".to_string(),
        redirect_uris: vec![],
        allowed_scopes: DEFAULT_MCP_ALLOWED_SCOPES.to_string(),
        grant_types: "authorization_code".to_string(),
        client_type: "public".to_string(),
        is_active: true,
        delegation_scopes: String::new(),
        broker_capability_enabled: false,
        revocation_webhook_url: None,
        revocation_webhook_secret_encrypted: None,
        created_by: Some("system".to_string()),
        created_at: now,
        updated_at: now,
    };

    collection.insert_one(&client).await?;
    tracing::info!("Seeded default MCP OAuth client (id={MCP_CLIENT_ID})");

    Ok(())
}

/// If `existing` is missing any scope from [`DEFAULT_MCP_ALLOWED_SCOPES`],
/// returns the merged, validated, canonical scope string. Returns `None` when
/// the existing scopes already cover the defaults (so callers can skip the
/// write).
fn merge_missing_default_mcp_scopes(existing: &str) -> AppResult<Option<String>> {
    let existing_set: std::collections::HashSet<&str> = existing.split_whitespace().collect();
    let missing: Vec<&str> = DEFAULT_MCP_ALLOWED_SCOPES
        .split_whitespace()
        .filter(|scope| !existing_set.contains(scope))
        .collect();

    if missing.is_empty() {
        return Ok(None);
    }

    let merged = format!("{existing} {}", missing.join(" "));
    Ok(Some(validate_allowed_scopes(&merged)?))
}

/// Backfill default MCP scopes onto OAuth clients created via Dynamic Client
/// Registration before the current scope set landed.
///
/// DCR is used by MCP clients (Cursor, Claude Code, Codex, etc.). Whenever
/// [`DEFAULT_MCP_ALLOWED_SCOPES`] grows, older DCR records would otherwise
/// fail authorization with `invalid_scope` (issue #434 was triggered by Codex
/// requesting `roles`/`groups`). This sweep upgrades them in place so existing
/// client_id caches keep working without re-registration.
///
/// Idempotent: clients that already cover the default set are skipped.
pub async fn migrate_dynamic_clients_grant_default_mcp_scopes(
    db: &mongodb::Database,
) -> AppResult<()> {
    let collection = db.collection::<OauthClient>(OAUTH_CLIENTS);

    let candidates: Vec<OauthClient> = collection
        .find(doc! { "created_by": "dynamic_registration" })
        .await?
        .try_collect()
        .await?;

    if candidates.is_empty() {
        return Ok(());
    }

    let now = bson::DateTime::from_chrono(Utc::now());
    let mut upgraded = 0_usize;

    for client in &candidates {
        let Some(updated_scopes) = merge_missing_default_mcp_scopes(&client.allowed_scopes)? else {
            continue;
        };

        collection
            .update_one(
                doc! { "_id": &client.id },
                doc! { "$set": {
                    "allowed_scopes": &updated_scopes,
                    "updated_at": now,
                }},
            )
            .await?;

        upgraded += 1;
    }

    if upgraded > 0 {
        tracing::info!(
            upgraded,
            "Backfilled default MCP scopes on dynamic-registration OAuth clients"
        );
    }

    Ok(())
}

/// Create a new OAuth client.
///
/// Returns the persisted client and, for confidential clients, the raw client
/// secret (which is only available at creation time -- only the hash is stored).
///
/// `allowed_scopes` must contain only known OIDC scopes (validated by the
/// caller). Pass [`DEFAULT_ALLOWED_SCOPES`] for the standard set.
#[allow(clippy::too_many_arguments)]
pub async fn create_client(
    db: &mongodb::Database,
    name: &str,
    redirect_uris: &[String],
    client_type: &str,
    created_by: &str,
    delegation_scopes: &str,
    allowed_scopes: &str,
    broker_capability_enabled: bool,
    revocation_webhook_url: Option<&str>,
    revocation_webhook_secret_encrypted: Option<Vec<u8>>,
) -> AppResult<(OauthClient, Option<String>)> {
    let client_id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let (secret_hash, raw_secret) = if client_type == "confidential" {
        let secret = generate_random_token();
        let hash = hash_token(&secret);
        (hash, Some(secret))
    } else {
        ("NONE".to_string(), None)
    };

    let client = OauthClient {
        id: client_id,
        client_name: name.to_string(),
        client_secret_hash: secret_hash,
        redirect_uris: redirect_uris.to_vec(),
        allowed_scopes: allowed_scopes.to_string(),
        grant_types: "authorization_code".to_string(),
        client_type: client_type.to_string(),
        is_active: true,
        delegation_scopes: delegation_scopes.to_string(),
        broker_capability_enabled,
        revocation_webhook_url: revocation_webhook_url.map(str::to_string),
        revocation_webhook_secret_encrypted,
        created_by: Some(created_by.to_string()),
        created_at: now,
        updated_at: now,
    };

    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .insert_one(&client)
        .await?;

    Ok((client, raw_secret))
}

/// List all OAuth clients (active and inactive).
pub async fn list_clients(db: &mongodb::Database) -> AppResult<Vec<OauthClient>> {
    let clients: Vec<OauthClient> = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find(doc! {})
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(clients)
}

/// List OAuth clients created by a specific user.
pub async fn list_clients_by_creator(
    db: &mongodb::Database,
    created_by: &str,
) -> AppResult<Vec<OauthClient>> {
    let clients: Vec<OauthClient> = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find(doc! { "created_by": created_by })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(clients)
}

/// Fetch a single OAuth client by ID.
pub async fn get_client(db: &mongodb::Database, client_id: &str) -> AppResult<OauthClient> {
    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id })
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))
}

/// Fetch a single OAuth client by ID and owner.
pub async fn get_client_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
) -> AppResult<OauthClient> {
    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id, "created_by": created_by })
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))
}

/// Update the redirect URIs on an OAuth client.
pub async fn update_redirect_uris(
    db: &mongodb::Database,
    client_id: &str,
    redirect_uris: &[String],
) -> AppResult<()> {
    let now = Utc::now();
    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "is_active": true },
            doc! { "$set": {
                "redirect_uris": bson::to_bson(redirect_uris).map_err(|e| {
                    AppError::Internal(format!("Failed to convert redirect_uris to bson: {e}"))
                })?,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    Ok(())
}

/// Update mutable fields on an OAuth client owned by a specific user.
#[allow(clippy::too_many_arguments)]
pub async fn update_client_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
    client_name: Option<&str>,
    redirect_uris: Option<&[String]>,
    delegation_scopes: Option<&str>,
    allowed_scopes: Option<&str>,
    broker_capability_enabled: Option<bool>,
    revocation_webhook_url: Option<&str>,
    revocation_webhook_secret_encrypted: Option<Vec<u8>>,
) -> AppResult<OauthClient> {
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(name) = client_name {
        set_doc.insert("client_name", name);
    }

    if let Some(uris) = redirect_uris {
        set_doc.insert(
            "redirect_uris",
            bson::to_bson(uris).map_err(|e| {
                AppError::Internal(format!("Failed to convert redirect_uris to bson: {e}"))
            })?,
        );
    }

    if let Some(scopes) = delegation_scopes {
        set_doc.insert("delegation_scopes", scopes);
    }

    if let Some(scopes) = allowed_scopes {
        set_doc.insert("allowed_scopes", scopes);
    }

    if let Some(enabled) = broker_capability_enabled {
        set_doc.insert("broker_capability_enabled", enabled);
    }

    if let Some(url) = revocation_webhook_url {
        set_doc.insert("revocation_webhook_url", url);
    }

    if let Some(secret) = revocation_webhook_secret_encrypted {
        set_doc.insert(
            "revocation_webhook_secret_encrypted",
            Binary {
                subtype: BinarySubtype::Generic,
                bytes: secret,
            },
        );
    }

    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "created_by": created_by, "is_active": true },
            doc! { "$set": set_doc },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    get_client_for_creator(db, client_id, created_by).await
}

/// Soft-delete an OAuth client by marking it inactive.
pub async fn delete_client(db: &mongodb::Database, client_id: &str) -> AppResult<()> {
    let now = Utc::now();

    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    cascade_client_deactivation(db, client_id).await?;

    Ok(())
}

/// Soft-delete an OAuth client owned by a specific user.
pub async fn delete_client_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
) -> AppResult<()> {
    let now = Utc::now();
    let result = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "created_by": created_by },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }

    cascade_client_deactivation(db, client_id).await?;

    Ok(())
}

// Mirrors org-delete cascade in org_service.rs so stale consents/refresh tokens
// do not linger after a single client is deactivated (issue #498).
async fn cascade_client_deactivation(db: &mongodb::Database, client_id: &str) -> AppResult<()> {
    db.collection::<bson::Document>(CONSENTS)
        .delete_many(doc! { "client_id": client_id })
        .await?;
    db.collection::<bson::Document>(REFRESH_TOKENS)
        .delete_many(doc! { "client_id": client_id })
        .await?;
    Ok(())
}

/// Rotate client secret for a confidential OAuth client owned by a specific user.
pub async fn rotate_client_secret_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
) -> AppResult<(OauthClient, String)> {
    let client = get_client_for_creator(db, client_id, created_by).await?;

    if client.client_type != "confidential" {
        return Err(AppError::BadRequest(
            "Only confidential clients can rotate secret".to_string(),
        ));
    }

    let new_secret = generate_random_token();
    let new_hash = hash_token(&new_secret);

    db.collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": client_id, "created_by": created_by, "is_active": true },
            doc! { "$set": {
                "client_secret_hash": new_hash,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .await?;

    let updated = get_client_for_creator(db, client_id, created_by).await?;
    Ok((updated, new_secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_default_scopes() {
        let result = validate_allowed_scopes("openid profile email").unwrap();
        assert_eq!(result, "openid profile email");
    }

    #[test]
    fn valid_with_roles_and_groups() {
        let result = validate_allowed_scopes("openid profile email roles groups").unwrap();
        assert_eq!(result, "openid profile email roles groups");
    }

    #[test]
    fn valid_minimal_openid_only() {
        let result = validate_allowed_scopes("openid").unwrap();
        assert_eq!(result, "openid");
    }

    #[test]
    fn valid_roles_without_profile() {
        let result = validate_allowed_scopes("openid roles").unwrap();
        assert_eq!(result, "openid roles");
    }

    #[test]
    fn auto_prepends_openid_when_missing() {
        let result = validate_allowed_scopes("profile email").unwrap();
        assert!(result.starts_with("openid"));
        assert!(result.contains("profile"));
        assert!(result.contains("email"));
    }

    #[test]
    fn deduplicates_scopes() {
        let result = validate_allowed_scopes("openid openid profile profile").unwrap();
        assert_eq!(result, "openid profile");
    }

    #[test]
    fn valid_with_proxy_scope() {
        let result = validate_allowed_scopes("openid profile email proxy").unwrap();
        assert_eq!(result, "openid profile email proxy");
    }

    #[test]
    fn rejects_unknown_scope() {
        let result = validate_allowed_scopes("openid admin");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("admin"));
    }

    #[test]
    fn rejects_arbitrary_scope() {
        let result = validate_allowed_scopes("openid read:users");
        assert!(result.is_err());
    }

    #[test]
    fn empty_string_gets_openid() {
        let result = validate_allowed_scopes("").unwrap();
        assert_eq!(result, "openid");
    }

    #[test]
    fn empty_list_gets_openid() {
        let result = validate_allowed_scopes_list(&[]).unwrap();
        assert_eq!(result, "openid");
    }

    #[test]
    fn default_mcp_scopes_include_roles_and_groups() {
        // Issue #434: Codex requests `roles` and `groups`; the DCR default
        // must allow both or scope validation rejects authorization.
        let scopes: Vec<&str> = DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace().collect();
        assert!(scopes.contains(&"openid"));
        assert!(scopes.contains(&"profile"));
        assert!(scopes.contains(&"email"));
        assert!(scopes.contains(&"roles"));
        assert!(scopes.contains(&"groups"));
        assert!(scopes.contains(&"proxy"));
    }

    #[test]
    fn default_mcp_scopes_validate() {
        // Guard against typos / unknown scopes ever entering the constant.
        validate_allowed_scopes(DEFAULT_MCP_ALLOWED_SCOPES).unwrap();
    }

    #[test]
    fn merge_returns_none_when_defaults_already_present() {
        let merged = merge_missing_default_mcp_scopes(DEFAULT_MCP_ALLOWED_SCOPES).unwrap();
        assert!(merged.is_none(), "no-op when nothing is missing");
    }

    #[test]
    fn merge_adds_only_missing_scopes() {
        // Pre-issue-#434 DCR records had `openid profile email proxy` but no
        // `roles`/`groups`. The merge must add exactly the missing pieces and
        // remain stable thereafter.
        let merged = merge_missing_default_mcp_scopes("openid profile email proxy")
            .unwrap()
            .expect("missing scopes should be merged in");

        let merged_set: std::collections::HashSet<&str> = merged.split_whitespace().collect();
        for scope in DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace() {
            assert!(merged_set.contains(scope), "missing {scope} after merge");
        }
        // Idempotent: a second pass produces no change.
        assert!(merge_missing_default_mcp_scopes(&merged).unwrap().is_none());
    }

    #[test]
    fn merge_preserves_existing_extras_and_dedupes() {
        // A client with everything already plus a duplicate should stay valid
        // and not regress.
        let merged =
            merge_missing_default_mcp_scopes("openid profile profile email roles").unwrap();
        let final_scopes = merged.expect("groups + proxy should be added");
        let parts: Vec<&str> = final_scopes.split_whitespace().collect();
        let unique: std::collections::HashSet<&str> = parts.iter().copied().collect();
        assert_eq!(parts.len(), unique.len(), "merge must dedupe");
    }

    mod mongo {
        use super::*;
        use crate::models::consent::Consent;
        use crate::models::refresh_token::RefreshToken;
        use crate::test_utils::connect_test_database;

        async fn insert_dcr_client(
            db: &mongodb::Database,
            id: &str,
            allowed_scopes: &str,
        ) -> OauthClient {
            let now = Utc::now();
            let client = OauthClient {
                id: id.to_string(),
                client_name: "DCR Test Client".to_string(),
                client_secret_hash: "NONE".to_string(),
                redirect_uris: vec![],
                allowed_scopes: allowed_scopes.to_string(),
                grant_types: "authorization_code".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                created_by: Some("dynamic_registration".to_string()),
                created_at: now,
                updated_at: now,
            };
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_one(&client)
                .await
                .expect("insert dcr fixture");
            client
        }

        async fn insert_client_with_consent_and_refresh_token(
            db: &mongodb::Database,
            client_id: &str,
            created_by: &str,
        ) {
            let now = Utc::now();
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_one(&OauthClient {
                    id: client_id.to_string(),
                    client_name: "Cascade Test Client".to_string(),
                    client_secret_hash: "NONE".to_string(),
                    redirect_uris: vec!["http://localhost:3000/callback".to_string()],
                    allowed_scopes: DEFAULT_ALLOWED_SCOPES.to_string(),
                    grant_types: "authorization_code".to_string(),
                    client_type: "public".to_string(),
                    is_active: true,
                    delegation_scopes: String::new(),
                    broker_capability_enabled: false,
                    revocation_webhook_url: None,
                    revocation_webhook_secret_encrypted: None,
                    created_by: Some(created_by.to_string()),
                    created_at: now,
                    updated_at: now,
                })
                .await
                .expect("insert oauth client fixture");

            db.collection::<Consent>(CONSENTS)
                .insert_one(&Consent {
                    id: format!("consent-{client_id}"),
                    user_id: "user-with-consent".to_string(),
                    client_id: client_id.to_string(),
                    scopes: DEFAULT_ALLOWED_SCOPES.to_string(),
                    granted_at: now,
                    expires_at: None,
                })
                .await
                .expect("insert consent fixture");

            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .insert_one(&RefreshToken {
                    id: format!("refresh-{client_id}"),
                    jti: format!("jti-{client_id}"),
                    client_id: client_id.to_string(),
                    user_id: "user-with-refresh-token".to_string(),
                    session_id: Some(format!("session-{client_id}")),
                    expires_at: now + chrono::Duration::days(1),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    created_at: now,
                })
                .await
                .expect("insert refresh token fixture");
        }

        async fn count_consents(db: &mongodb::Database, client_id: &str) -> u64 {
            db.collection::<Consent>(CONSENTS)
                .count_documents(doc! { "client_id": client_id })
                .await
                .expect("count consents")
        }

        async fn count_refresh_tokens(db: &mongodb::Database, client_id: &str) -> u64 {
            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .count_documents(doc! { "client_id": client_id })
                .await
                .expect("count refresh tokens")
        }

        async fn assert_client_deactivated_and_cascaded(db: &mongodb::Database, client_id: &str) {
            let client = get_client(db, client_id)
                .await
                .expect("client tombstone remains");
            assert!(!client.is_active, "client should be soft-deleted");
            assert_eq!(count_consents(db, client_id).await, 0);
            assert_eq!(count_refresh_tokens(db, client_id).await, 0);
        }

        #[tokio::test]
        async fn delete_client_for_creator_deactivates_and_cascades_grants() {
            let Some(db) = connect_test_database("oc_del_creator").await else {
                eprintln!("skipping oc_del_creator test: no local MongoDB available");
                return;
            };

            let client_id = "owned-client";
            insert_client_with_consent_and_refresh_token(&db, client_id, "owner").await;

            delete_client_for_creator(&db, client_id, "owner")
                .await
                .expect("delete owned client");

            assert_client_deactivated_and_cascaded(&db, client_id).await;
        }

        #[tokio::test]
        async fn delete_client_for_creator_does_not_cascade_when_owner_mismatches() {
            let Some(db) = connect_test_database("oc_del_wrong").await else {
                eprintln!("skipping oc_del_wrong test: no local MongoDB available");
                return;
            };

            let client_id = "cross-owned-client";
            insert_client_with_consent_and_refresh_token(&db, client_id, "owner").await;

            let err = delete_client_for_creator(&db, client_id, "other-owner")
                .await
                .expect_err("wrong owner must not delete");

            assert!(matches!(err, AppError::NotFound(_)));
            let client = get_client(&db, client_id)
                .await
                .expect("client should remain");
            assert!(client.is_active, "client should remain active");
            assert_eq!(count_consents(&db, client_id).await, 1);
            assert_eq!(count_refresh_tokens(&db, client_id).await, 1);
        }

        #[tokio::test]
        async fn delete_client_deactivates_and_cascades_grants() {
            let Some(db) = connect_test_database("oc_del_admin").await else {
                eprintln!("skipping oc_del_admin test: no local MongoDB available");
                return;
            };

            let client_id = "admin-delete-client";
            insert_client_with_consent_and_refresh_token(&db, client_id, "owner").await;

            delete_client(&db, client_id)
                .await
                .expect("admin delete client");

            assert_client_deactivated_and_cascaded(&db, client_id).await;
        }

        #[tokio::test]
        async fn migration_backfills_roles_and_groups_on_legacy_dcr_clients() {
            let Some(db) = connect_test_database("oauth_dcr_migration").await else {
                eprintln!("skipping oauth_dcr_migration test: no local MongoDB available");
                return;
            };

            // Pre-#434 DCR client: has proxy but missing roles/groups.
            insert_dcr_client(&db, "legacy-dcr", "openid profile email proxy").await;
            // Already up-to-date client: should stay unchanged.
            insert_dcr_client(&db, "current-dcr", DEFAULT_MCP_ALLOWED_SCOPES).await;

            migrate_dynamic_clients_grant_default_mcp_scopes(&db)
                .await
                .expect("migration runs cleanly");

            let upgraded = get_client(&db, "legacy-dcr").await.unwrap();
            for scope in DEFAULT_MCP_ALLOWED_SCOPES.split_whitespace() {
                assert!(
                    upgraded
                        .allowed_scopes
                        .split_whitespace()
                        .any(|s| s == scope),
                    "legacy DCR client should have {scope} after migration"
                );
            }

            // Idempotent: a second pass is a no-op.
            migrate_dynamic_clients_grant_default_mcp_scopes(&db)
                .await
                .expect("migration is idempotent");
        }

        #[tokio::test]
        async fn seed_upgrades_existing_mcp_client_with_missing_default_scopes() {
            let Some(db) = connect_test_database("oauth_seed_upgrade").await else {
                eprintln!("skipping oauth_seed_upgrade test: no local MongoDB available");
                return;
            };

            let now = Utc::now();
            db.collection::<OauthClient>(OAUTH_CLIENTS)
                .insert_one(&OauthClient {
                    id: MCP_CLIENT_ID.to_string(),
                    client_name: "NyxID MCP Client".to_string(),
                    client_secret_hash: "NONE".to_string(),
                    redirect_uris: vec![],
                    allowed_scopes: "openid profile email proxy".to_string(),
                    grant_types: "authorization_code".to_string(),
                    client_type: "public".to_string(),
                    is_active: true,
                    delegation_scopes: String::new(),
                    broker_capability_enabled: false,
                    revocation_webhook_url: None,
                    revocation_webhook_secret_encrypted: None,
                    created_by: Some("system".to_string()),
                    created_at: now,
                    updated_at: now,
                })
                .await
                .expect("seed legacy mcp client");

            seed_default_clients(&db).await.expect("seed runs");

            let upgraded = get_client(&db, MCP_CLIENT_ID).await.unwrap();
            for scope in ["roles", "groups"] {
                assert!(
                    upgraded
                        .allowed_scopes
                        .split_whitespace()
                        .any(|s| s == scope),
                    "seeded mcp client should have {scope} after upgrade"
                );
            }
        }
    }
}
