use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::oauth_client::{OauthClient, COLLECTION_NAME as OAUTH_CLIENTS};

/// Well-known client ID for native MCP clients (Cursor, Claude Code, etc.).
const MCP_CLIENT_ID: &str = "nyx-mcp";

/// Seed default OAuth clients at startup (idempotent).
///
/// Creates the `nyx-mcp` public client used by MCP desktop apps. The client
/// has no registered redirect URIs because loopback URIs are validated
/// dynamically per RFC 8252 section 7.3.
pub async fn seed_default_clients(db: &mongodb::Database) -> AppResult<()> {
    let collection = db.collection::<OauthClient>(OAUTH_CLIENTS);

    if collection
        .find_one(doc! { "_id": MCP_CLIENT_ID })
        .await?
        .is_some()
    {
        return Ok(());
    }

    let now = Utc::now();
    let client = OauthClient {
        id: MCP_CLIENT_ID.to_string(),
        client_name: "NyxID MCP Client".to_string(),
        client_secret_hash: "NONE".to_string(),
        redirect_uris: vec![],
        allowed_scopes: "openid profile email".to_string(),
        grant_types: "authorization_code".to_string(),
        client_type: "public".to_string(),
        is_active: true,
        delegation_scopes: String::new(),
        created_by: Some("system".to_string()),
        created_at: now,
        updated_at: now,
    };

    collection.insert_one(&client).await?;
    tracing::info!("Seeded default MCP OAuth client (id={MCP_CLIENT_ID})");

    Ok(())
}

/// Create a new OAuth client.
///
/// Returns the persisted client and, for confidential clients, the raw client
/// secret (which is only available at creation time -- only the hash is stored).
pub async fn create_client(
    db: &mongodb::Database,
    name: &str,
    redirect_uris: &[String],
    client_type: &str,
    created_by: &str,
    delegation_scopes: &str,
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
        allowed_scopes: "openid profile email".to_string(),
        grant_types: "authorization_code".to_string(),
        client_type: client_type.to_string(),
        is_active: true,
        delegation_scopes: delegation_scopes.to_string(),
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
pub async fn update_client_for_creator(
    db: &mongodb::Database,
    client_id: &str,
    created_by: &str,
    client_name: Option<&str>,
    redirect_uris: Option<&[String]>,
    delegation_scopes: Option<&str>,
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
