use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::{
    AgentServiceBinding, COLLECTION_NAME as AGENT_BINDINGS,
};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::{
    api_key_mutation_service as key_mutations,
    api_key_scope_service::{self, ScopeAuthorization},
};

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
#[allow(dead_code)]
pub async fn create_binding(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    user_service_id: &str,
    user_api_key_id: &str,
) -> AppResult<AgentServiceBinding> {
    create_binding_with_scope_authorization(
        db,
        user_id,
        None,
        api_key_id,
        user_service_id,
        user_api_key_id,
    )
    .await
}

pub async fn create_binding_with_scope_authorization(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    api_key_id: &str,
    user_service_id: &str,
    user_api_key_id: &str,
) -> AppResult<AgentServiceBinding> {
    create_binding_with_scope_authorization_inner(
        db,
        user_id,
        scope_actor_user_id,
        api_key_id,
        user_service_id,
        user_api_key_id,
        #[cfg(test)]
        None,
    )
    .await
}

#[cfg(test)]
async fn create_binding_with_collision_hook(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    user_service_id: &str,
    user_api_key_id: &str,
    collision_hook: key_mutations::TransactionCollisionHook,
) -> AppResult<AgentServiceBinding> {
    create_binding_with_scope_authorization_inner(
        db,
        user_id,
        None,
        api_key_id,
        user_service_id,
        user_api_key_id,
        Some(collision_hook),
    )
    .await
}

async fn create_binding_with_scope_authorization_inner(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    api_key_id: &str,
    user_service_id: &str,
    user_api_key_id: &str,
    #[cfg(test)] collision_hook: Option<key_mutations::TransactionCollisionHook>,
) -> AppResult<AgentServiceBinding> {
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

    let db = db.clone();
    let user_id = user_id.to_string();
    let api_key_id = api_key_id.to_string();
    let user_service_id = user_service_id.to_string();
    let user_api_key_id = user_api_key_id.to_string();
    let actor_id = scope_actor_user_id.map(str::to_string);
    let binding_for_transaction = binding.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            #[cfg(test)]
            if let Some(hook) = collision_hook.as_ref() {
                hook.begin_attempt();
            }
            let operation: AppResult<()> = async {
                let api_key = db
                    .collection::<ApiKey>(API_KEYS)
                    .find_one(doc! {
                        "_id": &api_key_id,
                        "user_id": &user_id,
                        "is_active": true,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

                let authorization = ScopeAuthorization::for_actor(actor_id.as_deref());
                api_key_scope_service::validate_owner_service_write_with_session(
                    &db,
                    &user_id,
                    &user_service_id,
                    authorization,
                    &mut *session,
                )
                .await?;

                db.collection::<UserService>(USER_SERVICES)
                    .find_one(doc! {
                        "_id": &user_service_id,
                        "user_id": &user_id,
                        "is_active": true,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("User service not found".to_string()))?;

                db.collection::<UserApiKey>(USER_API_KEYS)
                    .find_one(doc! { "_id": &user_api_key_id, "user_id": &user_id })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound("External credential not found".to_string())
                    })?;

                if db
                    .collection::<AgentServiceBinding>(AGENT_BINDINGS)
                    .find_one(doc! {
                        "api_key_id": &api_key_id,
                        "user_service_id": &user_service_id,
                    })
                    .session(&mut *session)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Conflict(
                        "Binding already exists for this API key and service".to_string(),
                    ));
                }

                #[cfg(test)]
                if let Some(hook) = collision_hook.as_ref() {
                    hook.after_reads().await;
                }

                db.collection::<AgentServiceBinding>(AGENT_BINDINGS)
                    .insert_one(&binding_for_transaction)
                    .session(&mut *session)
                    .await?;

                let key_update = if !api_key.allow_all_services
                    && !api_key.allowed_service_ids.contains(&user_service_id)
                {
                    doc! { "$addToSet": { "allowed_service_ids": &user_service_id } }
                } else {
                    doc! {}
                };
                let key_result = key_mutations::update_one(
                    &db,
                    doc! {
                        "_id": &api_key_id,
                        "user_id": &user_id,
                        "is_active": true,
                    },
                    key_update,
                    Some(&mut *session),
                )
                .await?;
                if key_result.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "API key changed while creating the binding".to_string(),
                    ));
                }
                Ok(())
            }
            .await;
            key_mutations::transaction_result(operation)
        })
        .await
        .map_err(key_mutations::map_transaction_error)?;

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
#[allow(dead_code)]
pub async fn delete_binding(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    binding_id: &str,
) -> AppResult<()> {
    delete_binding_with_scope_authorization(db, user_id, None, api_key_id, binding_id).await
}

pub async fn delete_binding_with_scope_authorization(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    api_key_id: &str,
    binding_id: &str,
) -> AppResult<()> {
    delete_binding_with_scope_authorization_inner(
        db,
        user_id,
        scope_actor_user_id,
        api_key_id,
        binding_id,
        #[cfg(test)]
        None,
    )
    .await
}

#[cfg(test)]
async fn delete_binding_with_collision_hook(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    binding_id: &str,
    collision_hook: key_mutations::TransactionCollisionHook,
) -> AppResult<()> {
    delete_binding_with_scope_authorization_inner(
        db,
        user_id,
        None,
        api_key_id,
        binding_id,
        Some(collision_hook),
    )
    .await
}

async fn delete_binding_with_scope_authorization_inner(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    api_key_id: &str,
    binding_id: &str,
    #[cfg(test)] collision_hook: Option<key_mutations::TransactionCollisionHook>,
) -> AppResult<()> {
    let db = db.clone();
    let user_id = user_id.to_string();
    let api_key_id = api_key_id.to_string();
    let binding_id = binding_id.to_string();
    let actor_id = scope_actor_user_id.map(str::to_string);
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            #[cfg(test)]
            if let Some(hook) = collision_hook.as_ref() {
                hook.begin_attempt();
            }
            let operation: AppResult<()> = async {
                let api_key = db
                    .collection::<ApiKey>(API_KEYS)
                    .find_one(doc! {
                        "_id": &api_key_id,
                        "user_id": &user_id,
                        "is_active": true,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;
                let binding = db
                    .collection::<AgentServiceBinding>(AGENT_BINDINGS)
                    .find_one(doc! {
                        "_id": &binding_id,
                        "api_key_id": &api_key_id,
                        "user_id": &user_id,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Binding not found".to_string()))?;

                api_key_scope_service::validate_owner_service_write_with_session(
                    &db,
                    &user_id,
                    &binding.user_service_id,
                    ScopeAuthorization::for_actor(actor_id.as_deref()),
                    &mut *session,
                )
                .await?;

                #[cfg(test)]
                if let Some(hook) = collision_hook.as_ref() {
                    hook.after_reads().await;
                }

                let deleted = db
                    .collection::<AgentServiceBinding>(AGENT_BINDINGS)
                    .delete_one(doc! { "_id": &binding_id })
                    .session(&mut *session)
                    .await?;
                if deleted.deleted_count != 1 {
                    return Err(AppError::Conflict(
                        "binding changed while deleting it".to_string(),
                    ));
                }

                let key_update = if api_key.allow_all_services {
                    doc! {}
                } else {
                    doc! { "$pull": { "allowed_service_ids": &binding.user_service_id } }
                };
                let key_result = key_mutations::update_one(
                    &db,
                    doc! {
                        "_id": &api_key_id,
                        "user_id": &user_id,
                        "is_active": true,
                    },
                    key_update,
                    Some(&mut *session),
                )
                .await?;
                if key_result.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "API key changed while deleting the binding".to_string(),
                    ));
                }
                Ok(())
            }
            .await;
            key_mutations::transaction_result(operation)
        })
        .await
        .map_err(key_mutations::map_transaction_error)?;

    Ok(())
}

/// Delete all bindings that reference a specific `UserService`. Called
/// from `deactivate_user_service` so the Agent Key detail page does not
/// show orphan bindings pointing at a missing/inactive service.
///
/// Also pulls the service id from `allowed_service_ids` on every
/// affected scoped `ApiKey`, mirroring the single-binding delete path.
/// Returns the number of bindings removed.
async fn cleanup_key_bindings(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    mut binding_filter: Document,
    service_ids: Vec<String>,
) -> AppResult<u64> {
    binding_filter.insert("api_key_id", api_key_id);
    let db = db.clone();
    let user_id = user_id.to_string();
    let api_key_id = api_key_id.to_string();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<u64> = async {
                let key = db
                    .collection::<ApiKey>(API_KEYS)
                    .find_one(doc! { "_id": &api_key_id, "user_id": &user_id })
                    .session(&mut *session)
                    .await?;
                let deleted = db
                    .collection::<AgentServiceBinding>(AGENT_BINDINGS)
                    .delete_many(binding_filter.clone())
                    .session(&mut *session)
                    .await?;

                if let Some(key) = key {
                    let key_update = if key.allow_all_services || service_ids.is_empty() {
                        doc! {}
                    } else {
                        doc! {
                            "$pull": {
                                "allowed_service_ids": { "$in": service_ids.clone() }
                            }
                        }
                    };
                    let updated = key_mutations::update_one(
                        &db,
                        doc! { "_id": &api_key_id, "user_id": &user_id },
                        key_update,
                        Some(&mut *session),
                    )
                    .await?;
                    if updated.matched_count != 1 {
                        return Err(AppError::Conflict(
                            "API key changed while cleaning bindings".to_string(),
                        ));
                    }
                }

                Ok(deleted.deleted_count)
            }
            .await;
            key_mutations::transaction_result(operation)
        })
        .await
        .map_err(key_mutations::map_transaction_error)
}

pub async fn cleanup_bindings_for_user_service(
    db: &mongodb::Database,
    user_id: &str,
    user_service_id: &str,
) -> AppResult<u64> {
    let mut removed = 0;
    loop {
        let affected_keys: HashSet<String> = db
            .collection::<AgentServiceBinding>(AGENT_BINDINGS)
            .find(doc! {
                "user_id": user_id,
                "user_service_id": user_service_id,
            })
            .await?
            .map_ok(|binding| binding.api_key_id)
            .try_collect()
            .await?;
        if affected_keys.is_empty() {
            return Ok(removed);
        }

        for key_id in affected_keys {
            removed += cleanup_key_bindings(
                db,
                user_id,
                &key_id,
                doc! {
                    "user_id": user_id,
                    "user_service_id": user_service_id,
                },
                vec![user_service_id.to_string()],
            )
            .await?;
        }
    }
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
    let mut removed = 0;
    loop {
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
            return Ok(removed);
        }

        let mut per_key: HashMap<String, HashSet<String>> = HashMap::new();
        for binding in bindings {
            per_key
                .entry(binding.api_key_id)
                .or_default()
                .insert(binding.user_service_id);
        }
        for (key_id, service_ids) in per_key {
            removed += cleanup_key_bindings(
                db,
                user_id,
                &key_id,
                doc! {
                    "user_id": user_id,
                    "user_api_key_id": user_api_key_id,
                },
                service_ids.into_iter().collect(),
            )
            .await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_api_key::UserApiKey;
    use crate::services::key_service::{self, ApiKeyRotationOutcome};
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
            rotation_predecessor_id: None,
            state_version: 1,
            updated_at: Some(Utc::now()),
            description: None,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: allow_all,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: Some("claude-code".to_string()),
            callback_url: None,
            purpose: Default::default(),
            scheduled_write_enabled: false,
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
            credential_source: None,
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
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: None,
            source_id: None,
            credential_epoch: 1,
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
        let db = connect_transaction_test_database("agent_bind").await;
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;

        let binding = create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();

        assert_eq!(binding.api_key_id, ak_id);
        assert_eq!(binding.user_service_id, us_id);
        assert_eq!(binding.user_api_key_id, uak_id);
        assert_eq!(binding.user_id, user_id);
        let key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &ak_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.state_version, 2);
        assert!(key.updated_at.is_some());
    }

    #[tokio::test]
    async fn test_create_binding_rejects_duplicate() {
        let db = connect_transaction_test_database("agent_bind").await;
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
        let db = connect_transaction_test_database("agent_bind").await;
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
        let db = connect_transaction_test_database("agent_bind").await;
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
        let db = connect_transaction_test_database("agent_bind").await;
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
        let db = connect_transaction_test_database("agent_bind").await;
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
        let key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &ak_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.state_version, 3);
    }

    #[tokio::test]
    async fn binding_mutations_revalidate_revoked_org_admin_authority() {
        let db = connect_transaction_test_database("agent_bind_revoked_admin").await;
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .unwrap();
        let membership = test_membership(&org_id, &actor_id, OrgRole::Admin, None);
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(&membership)
            .await
            .unwrap();
        let (api_key_id, service_id, credential_id) = seed_fixtures(&db, &org_id).await;
        let binding = create_binding_with_scope_authorization(
            &db,
            &org_id,
            Some(&actor_id),
            &api_key_id,
            &service_id,
            &credential_id,
        )
        .await
        .expect("active admin creates binding");

        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .update_one(
                doc! { "_id": &membership.id },
                doc! { "$set": { "revoked_at": bson::DateTime::now() } },
            )
            .await
            .unwrap();

        let create_error = create_binding_with_scope_authorization(
            &db,
            &org_id,
            Some(&actor_id),
            &api_key_id,
            &service_id,
            &credential_id,
        )
        .await
        .expect_err("revoked admin cannot create bindings");
        assert!(matches!(create_error, AppError::OrgRoleInsufficient(_)));

        let delete_error = delete_binding_with_scope_authorization(
            &db,
            &org_id,
            Some(&actor_id),
            &api_key_id,
            &binding.id,
        )
        .await
        .expect_err("revoked admin cannot delete bindings");
        assert!(matches!(delete_error, AppError::OrgRoleInsufficient(_)));
        assert!(
            get_binding(&db, &org_id, &api_key_id, &binding.id)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn concurrent_rotation_and_binding_create_never_lose_a_committed_binding() {
        let db = connect_transaction_test_database("agent_bind_rotate_create").await;
        crate::db::ensure_indexes(&db).await.unwrap();
        let user_id = Uuid::new_v4().to_string();
        let (predecessor_id, service_id, credential_id) = seed_fixtures(&db, &user_id).await;
        let successor_id = Uuid::new_v4().to_string();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let binding_hook = key_mutations::TransactionCollisionHook::new(barrier.clone());
        let rotation_hook = key_mutations::TransactionCollisionHook::new(barrier);

        let (create_result, rotation_result) = tokio::join!(
            create_binding_with_collision_hook(
                &db,
                &user_id,
                &predecessor_id,
                &service_id,
                &credential_id,
                binding_hook.clone(),
            ),
            key_service::rotate_api_key_with_scope_authorization_and_id_with_collision_hook(
                &db,
                &user_id,
                Some(&user_id),
                &predecessor_id,
                &successor_id,
                rotation_hook.clone(),
            ),
        );

        assert!(
            binding_hook.attempts() > 1 || rotation_hook.attempts() > 1,
            "forced overlapping snapshots must make MongoDB retry one transaction"
        );

        assert!(matches!(
            rotation_result.unwrap(),
            ApiKeyRotationOutcome::Created(_)
        ));
        let successor_bindings: Vec<AgentServiceBinding> = db
            .collection::<AgentServiceBinding>(AGENT_BINDINGS)
            .find(doc! { "api_key_id": &successor_id })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        match create_result {
            Ok(binding) => {
                assert_eq!(successor_bindings.len(), 1);
                assert_eq!(
                    successor_bindings[0].user_service_id,
                    binding.user_service_id
                );
                assert_eq!(
                    successor_bindings[0].user_api_key_id,
                    binding.user_api_key_id
                );
            }
            Err(AppError::NotFound(_)) | Err(AppError::Conflict(_)) => {
                assert!(successor_bindings.is_empty());
            }
            Err(error) => panic!("unexpected binding-create result: {error:?}"),
        }
    }

    #[tokio::test]
    async fn concurrent_rotation_and_binding_delete_never_leave_a_stale_successor_clone() {
        let db = connect_transaction_test_database("agent_bind_rotate_delete").await;
        crate::db::ensure_indexes(&db).await.unwrap();
        let user_id = Uuid::new_v4().to_string();
        let (predecessor_id, service_id, credential_id) = seed_fixtures(&db, &user_id).await;
        let binding = create_binding(&db, &user_id, &predecessor_id, &service_id, &credential_id)
            .await
            .unwrap();
        let successor_id = Uuid::new_v4().to_string();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let binding_hook = key_mutations::TransactionCollisionHook::new(barrier.clone());
        let rotation_hook = key_mutations::TransactionCollisionHook::new(barrier);

        let (delete_result, rotation_result) = tokio::join!(
            delete_binding_with_collision_hook(
                &db,
                &user_id,
                &predecessor_id,
                &binding.id,
                binding_hook.clone(),
            ),
            key_service::rotate_api_key_with_scope_authorization_and_id_with_collision_hook(
                &db,
                &user_id,
                Some(&user_id),
                &predecessor_id,
                &successor_id,
                rotation_hook.clone(),
            ),
        );

        assert!(
            binding_hook.attempts() > 1 || rotation_hook.attempts() > 1,
            "forced overlapping snapshots must make MongoDB retry one transaction"
        );

        assert!(matches!(
            rotation_result.unwrap(),
            ApiKeyRotationOutcome::Created(_)
        ));
        let successor_bindings: Vec<AgentServiceBinding> = db
            .collection::<AgentServiceBinding>(AGENT_BINDINGS)
            .find(doc! { "api_key_id": &successor_id })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        match delete_result {
            Ok(()) => assert!(successor_bindings.is_empty()),
            Err(AppError::NotFound(_)) | Err(AppError::Conflict(_)) => {
                assert_eq!(successor_bindings.len(), 1);
                assert_eq!(successor_bindings[0].user_service_id, service_id);
            }
            Err(error) => panic!("unexpected binding-delete result: {error:?}"),
        }
    }

    #[tokio::test]
    async fn test_cleanup_bindings_for_user_service() {
        let db = connect_transaction_test_database("agent_bind").await;
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
        let key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &ak_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.state_version, 3);

        let zero = cleanup_bindings_for_user_service(&db, &user_id, &us_id)
            .await
            .unwrap();
        assert_eq!(zero, 0);
    }

    #[tokio::test]
    async fn test_cleanup_bindings_for_credential_versions_the_key() {
        let db = connect_transaction_test_database("agent_bind_cleanup_credential").await;
        let user_id = Uuid::new_v4().to_string();
        let (ak_id, us_id, uak_id) = seed_fixtures(&db, &user_id).await;
        create_binding(&db, &user_id, &ak_id, &us_id, &uak_id)
            .await
            .unwrap();

        let removed = cleanup_bindings_for_credential(&db, &user_id, &uak_id)
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let bindings = list_bindings(&db, &user_id, &ak_id).await.unwrap();
        assert!(bindings.is_empty());
        let key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &ak_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.state_version, 3);
    }
}
