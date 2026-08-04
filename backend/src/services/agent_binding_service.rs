use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::{
    AgentServiceBinding, COLLECTION_NAME as AGENT_BINDINGS,
};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

/// Look up a credential override for a specific agent + service combination.
/// Returns the UserApiKey ID to use, or None if no override exists.
pub async fn resolve_credential_override(
    db: &mongodb::Database,
    api_key_id: &str,
    user_service_id: &str,
    user_id: &str,
) -> AppResult<Option<String>> {
    let binding = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find_one(doc! {
            "api_key_id": api_key_id,
            "user_service_id": user_service_id,
            "user_id": user_id,
        })
        .await?;

    Ok(binding.map(|b| b.user_api_key_id))
}

/// Create a new agent-service credential binding.
pub async fn create_binding(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    user_service_id: &str,
    user_api_key_id: &str,
) -> AppResult<AgentServiceBinding> {
    // Validate ownership: api_key must belong to user
    let api_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": api_key_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    // Validate ownership: user_service must belong to user
    let _user_service = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": user_service_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("User service not found".to_string()))?;

    // Validate ownership: user_api_key must belong to user
    let _credential = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": user_api_key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("External credential not found".to_string()))?;

    // Check for existing binding (unique constraint will catch race, but give better error)
    let existing = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find_one(doc! {
            "api_key_id": api_key_id,
            "user_service_id": user_service_id,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "Binding already exists for this API key and service".to_string(),
        ));
    }

    let now = Utc::now();
    let binding = AgentServiceBinding {
        id: Uuid::new_v4().to_string(),
        api_key_id: api_key_id.to_string(),
        user_service_id: user_service_id.to_string(),
        user_api_key_id: user_api_key_id.to_string(),
        user_id: user_id.to_string(),
        created_at: now,
        updated_at: now,
    };

    db.collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .insert_one(&binding)
        .await?;

    // If the key has explicit scope (allow_all_services: false), ensure the
    // newly bound service is in allowed_service_ids so the proxy allows it.
    if !api_key.allow_all_services
        && !api_key
            .allowed_service_ids
            .contains(&user_service_id.to_string())
    {
        db.collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! { "_id": api_key_id },
                doc! { "$addToSet": { "allowed_service_ids": user_service_id } },
            )
            .await?;
    }

    Ok(binding)
}

/// List all bindings for a specific API key.
pub async fn list_bindings(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
) -> AppResult<Vec<AgentServiceBinding>> {
    // Verify key ownership
    let _key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": api_key_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let bindings: Vec<AgentServiceBinding> = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find(doc! { "api_key_id": api_key_id })
        .limit(100)
        .await?
        .try_collect()
        .await?;

    Ok(bindings)
}

/// Look up a single binding by ID, scoped to a key + owner. Used by
/// the per-binding scope check on org-owned API keys before deletion.
pub async fn get_binding(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    binding_id: &str,
) -> AppResult<AgentServiceBinding> {
    db.collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find_one(doc! {
            "_id": binding_id,
            "api_key_id": api_key_id,
            "user_id": user_id,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Binding not found".to_string()))
}

/// Delete a binding by ID.
pub async fn delete_binding(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    binding_id: &str,
) -> AppResult<()> {
    let binding = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find_one(doc! {
            "_id": binding_id,
            "api_key_id": api_key_id,
            "user_id": user_id,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Binding not found".to_string()))?;

    let result = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .delete_one(doc! { "_id": binding_id })
        .await?;

    if result.deleted_count == 0 {
        return Err(AppError::NotFound("Binding not found".to_string()));
    }

    // If the key has explicit scope, remove the service from allowed_service_ids
    let api_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": api_key_id })
        .await?;

    if let Some(key) = api_key
        && !key.allow_all_services
    {
        db.collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! { "_id": api_key_id },
                doc! { "$pull": { "allowed_service_ids": &binding.user_service_id } },
            )
            .await?;
    }

    Ok(())
}

/// Delete all bindings that reference a specific `UserService`. Called
/// from `deactivate_user_service` so the Agent Key detail page does not
/// show orphan bindings pointing at a missing/inactive service.
///
/// Also pulls the service id from `allowed_service_ids` on every
/// affected scoped `ApiKey`, mirroring the single-binding delete path.
/// Returns the number of bindings removed.
pub async fn cleanup_bindings_for_user_service(
    db: &mongodb::Database,
    user_id: &str,
    user_service_id: &str,
) -> AppResult<u64> {
    let bindings: Vec<AgentServiceBinding> = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find(doc! {
            "user_id": user_id,
            "user_service_id": user_service_id,
        })
        .await?
        .try_collect()
        .await?;

    if bindings.is_empty() {
        return Ok(0);
    }

    let affected_keys: HashSet<String> = bindings.iter().map(|b| b.api_key_id.clone()).collect();

    let result = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .delete_many(doc! {
            "user_id": user_id,
            "user_service_id": user_service_id,
        })
        .await?;

    for key_id in affected_keys {
        let api_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &key_id })
            .await?;
        if let Some(key) = api_key
            && !key.allow_all_services
        {
            db.collection::<ApiKey>(API_KEYS)
                .update_one(
                    doc! { "_id": &key_id },
                    doc! { "$pull": { "allowed_service_ids": user_service_id } },
                )
                .await?;
        }
    }

    Ok(result.deleted_count)
}

/// Delete all bindings that reference a specific external credential
/// (`UserApiKey`). Called from `delete_api_key` so the Agent Key detail
/// page does not keep showing bindings pointing at a missing credential
/// (which otherwise degrade `credential_label` to a raw UUID).
///
/// Pulls the corresponding service ids from `allowed_service_ids` on
/// each affected scoped `ApiKey`, so the scoped allow-list stays in sync
/// with the bindings. Returns the number of bindings removed.
pub async fn cleanup_bindings_for_credential(
    db: &mongodb::Database,
    user_id: &str,
    user_api_key_id: &str,
) -> AppResult<u64> {
    let bindings: Vec<AgentServiceBinding> = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find(doc! {
            "user_id": user_id,
            "user_api_key_id": user_api_key_id,
        })
        .await?
        .try_collect()
        .await?;

    if bindings.is_empty() {
        return Ok(0);
    }

    // Group service ids per affected api key so each key gets a single
    // `$pull` update rather than one per binding.
    let mut per_key: HashMap<String, HashSet<String>> = HashMap::new();
    for binding in &bindings {
        per_key
            .entry(binding.api_key_id.clone())
            .or_default()
            .insert(binding.user_service_id.clone());
    }

    let result = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .delete_many(doc! {
            "user_id": user_id,
            "user_api_key_id": user_api_key_id,
        })
        .await?;

    for (key_id, service_ids) in per_key {
        let api_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &key_id })
            .await?;
        if let Some(key) = api_key
            && !key.allow_all_services
        {
            let ids: Vec<String> = service_ids.into_iter().collect();
            db.collection::<ApiKey>(API_KEYS)
                .update_one(
                    doc! { "_id": &key_id },
                    doc! { "$pull": { "allowed_service_ids": { "$in": ids } } },
                )
                .await?;
        }
    }

    Ok(result.deleted_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_api_key::UserApiKey;
    use crate::test_utils::*;

    fn make_api_key(id: &str, user_id: &str, allow_all: bool) -> ApiKey {
        ApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            name: "test-agent-key".to_string(),
            key_prefix: "nyxid_ag".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "proxy".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: Utc::now(),
            description: None,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: allow_all,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: Some("claude-code".to_string()),
            callback_url: None,
        }
    }

    fn make_user_service(id: &str, user_id: &str) -> UserService {
        test_user_service(
            id,
            user_id,
            "test-svc",
            &Uuid::new_v4().to_string(),
            None,
            None,
        )
    }

    fn make_user_api_key(id: &str, user_id: &str) -> UserApiKey {
        UserApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            label: "test-credential".to_string(),
            credential_type: "api_key".to_string(),
            credential_encrypted: Some(vec![1, 2, 3]),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: None,
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    async fn seed_fixtures(db: &mongodb::Database, user_id: &str) -> (String, String, String) {
        let ak_id = Uuid::new_v4().to_string();
        let us_id = Uuid::new_v4().to_string();
        let uak_id = Uuid::new_v4().to_string();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(&ak_id, user_id, true))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(make_user_service(&us_id, user_id))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(make_user_api_key(&uak_id, user_id))
            .await
            .unwrap();

        (ak_id, us_id, uak_id)
    }

    #[tokio::test]
    async fn test_create_binding_happy_path() {
        let Some(db) = connect_test_database("agent_bind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;

        let binding = create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();

        assert_eq!(binding.api_key_id, ak_id);
        assert_eq!(binding.user_service_id, us_id);
        assert_eq!(binding.user_api_key_id, uak_id);
        assert_eq!(binding.user_id, user_id);
    }

    #[tokio::test]
    async fn test_create_binding_rejects_duplicate() {
        let Some(db) = connect_test_database("agent_bind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;

        create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();
        let err = create_binding(&db, &user_id, &ak_id, &us_id, &uak_id).await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_create_binding_missing_api_key() {
        let Some(db) = connect_test_database("agent_bind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let us_id = Uuid::new_v4().to_string();
        let uak_id = Uuid::new_v4().to_string();

        db.collection::<UserService>(USER_SERVICES)
            .insert_one(make_user_service(&us_id, &user_id))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(make_user_api_key(&uak_id, &user_id))
            .await
            .unwrap();

        let err = create_binding(&db, &user_id, &Uuid::new_v4().to_string(), &us_id, &uak_id).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_resolve_credential_override() {
        let Some(db) = connect_test_database("agent_bind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;

        let none = resolve_credential_override(&db, &ak_id, &us_id, &user_id)
            .await
            .unwrap();
        assert!(none.is_none());

        create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();

        let found = resolve_credential_override(&db, &ak_id, &us_id, &user_id)
            .await
            .unwrap();
        assert_eq!(found, Some(uak_id));
    }

    #[tokio::test]
    async fn test_list_bindings() {
        let Some(db) = connect_test_database("agent_bind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;

        create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();

        let bindings = list_bindings(&db, &user_id, &ak_id).await.unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].user_service_id, us_id);
    }

    #[tokio::test]
    async fn test_delete_binding() {
        let Some(db) = connect_test_database("agent_bind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;

        let binding = create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();
        delete_binding(&db, &user_id, &ak_id, &binding.id)
            .await
            .unwrap();

        let bindings = list_bindings(&db, &user_id, &ak_id).await.unwrap();
        assert!(bindings.is_empty());
    }

    #[tokio::test]
    async fn test_cleanup_bindings_for_user_service() {
        let Some(db) = connect_test_database("agent_bind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;

        create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();

        let removed = cleanup_bindings_for_user_service(&db, &user_id, &us_id)
            .await
            .unwrap();
        assert_eq!(removed, 1);

        let bindings = list_bindings(&db, &user_id, &ak_id).await.unwrap();
        assert!(bindings.is_empty());

        let zero = cleanup_bindings_for_user_service(&db, &user_id, &us_id)
            .await
            .unwrap();
        assert_eq!(zero, 0);
    }
}
