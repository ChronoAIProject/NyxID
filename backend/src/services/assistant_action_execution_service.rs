use chrono::{DateTime, Utc};
use mongodb::bson::{doc, to_bson};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::assistant_action_receipt::{
    AssistantActionReceipt, AssistantActionReceiptStatus,
    COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::key_service::{self, ApiKeyRotationOutcome, CreatedApiKey};
use crate::services::org_service;

const KEY_CREATE_ACTION: &str = "key.create";
const KEY_ROTATE_ACTION: &str = "key.rotate";
const MAX_ACTION_REQUEST_ID_LEN: usize = 256;
const MAX_SERVICE_IDS: usize = 64;

fn utc_now_at_bson_precision() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(Utc::now().timestamp_millis())
        .expect("current UTC timestamp must fit BSON precision")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCreateActionRequest {
    pub action_request_id: String,
    pub name: String,
    pub platform: String,
    pub allowed_service_ids: Vec<String>,
}

#[derive(Debug)]
pub enum KeyCreateActionResult {
    Created(Box<CreatedApiKey>),
    Replayed { key_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRotateActionRequest {
    pub action_request_id: String,
    pub key_id: String,
}

#[derive(Debug)]
pub enum KeyRotateActionResult {
    Created {
        created: Box<CreatedApiKey>,
        requested_at: DateTime<Utc>,
    },
    Replayed {
        key_id: String,
        requested_at: DateTime<Utc>,
    },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyCreateFingerprint<'a> {
    action: &'static str,
    name: &'a str,
    platform: &'a str,
    allowed_service_ids: &'a [String],
    scopes: &'static str,
    allow_all_services: bool,
    allow_all_nodes: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyRotateFingerprint<'a> {
    action: &'static str,
    key_id: &'a str,
}

fn normalize_action_request_id(value: String) -> AppResult<String> {
    let value = value.trim().to_string();
    if value.is_empty()
        || value.len() > MAX_ACTION_REQUEST_ID_LEN
        || value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, '/' | '\\' | '?' | '#')
        })
    {
        return Err(AppError::ValidationError(
            "actionRequestId must be a valid control identity of at most 256 characters"
                .to_string(),
        ));
    }
    Ok(value)
}

fn normalize_request(request: KeyCreateActionRequest) -> AppResult<KeyCreateActionRequest> {
    let action_request_id = normalize_action_request_id(request.action_request_id)?;

    let name = request.name.trim().to_string();
    let platform = request.platform.trim().to_string();
    let mut allowed_service_ids = request
        .allowed_service_ids
        .into_iter()
        .map(|value| value.trim().to_string())
        .collect::<Vec<_>>();
    if allowed_service_ids.is_empty() {
        return Err(AppError::ValidationError(
            "key.create requires at least one exact allowed service id".to_string(),
        ));
    }
    if allowed_service_ids.len() > MAX_SERVICE_IDS
        || allowed_service_ids.iter().any(String::is_empty)
    {
        return Err(AppError::ValidationError(
            "allowedServiceIds must contain 1 to 64 non-empty ids".to_string(),
        ));
    }
    allowed_service_ids.sort();
    if allowed_service_ids
        .windows(2)
        .any(|pair| pair[0] == pair[1])
    {
        return Err(AppError::ValidationError(
            "allowedServiceIds must not contain duplicates".to_string(),
        ));
    }

    Ok(KeyCreateActionRequest {
        action_request_id,
        name,
        platform,
        allowed_service_ids,
    })
}

fn normalize_rotation_request(
    request: KeyRotateActionRequest,
) -> AppResult<KeyRotateActionRequest> {
    let action_request_id = normalize_action_request_id(request.action_request_id)?;
    let key_id = Uuid::parse_str(request.key_id.trim())
        .map_err(|_| AppError::ValidationError("keyId must be a UUID".to_string()))?
        .to_string();
    Ok(KeyRotateActionRequest {
        action_request_id,
        key_id,
    })
}

fn request_fingerprint(request: &KeyCreateActionRequest) -> AppResult<String> {
    let canonical = serde_json::to_vec(&KeyCreateFingerprint {
        action: KEY_CREATE_ACTION,
        name: &request.name,
        platform: &request.platform,
        allowed_service_ids: &request.allowed_service_ids,
        scopes: "proxy",
        allow_all_services: false,
        allow_all_nodes: false,
    })
    .map_err(|error| AppError::Internal(format!("failed to fingerprint action: {error}")))?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

fn rotation_request_fingerprint(request: &KeyRotateActionRequest) -> AppResult<String> {
    let canonical = serde_json::to_vec(&KeyRotateFingerprint {
        action: KEY_ROTATE_ACTION,
        key_id: &request.key_id,
    })
    .map_err(|error| AppError::Internal(format!("failed to fingerprint action: {error}")))?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

async fn validate_personal_service_ids(
    db: &mongodb::Database,
    user_id: &str,
    allowed_service_ids: &[String],
) -> AppResult<()> {
    let matching = db
        .collection::<UserService>(USER_SERVICES)
        .count_documents(doc! {
            "_id": { "$in": allowed_service_ids },
            "user_id": user_id,
            "is_active": true,
        })
        .await?;
    if matching != allowed_service_ids.len() as u64 {
        return Err(AppError::ValidationError(
            "allowedServiceIds must identify active services owned by this account".to_string(),
        ));
    }
    Ok(())
}

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

async fn find_receipt(
    db: &mongodb::Database,
    user_id: &str,
    action_request_id: &str,
) -> AppResult<Option<AssistantActionReceipt>> {
    Ok(db
        .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .find_one(doc! {
            "user_id": user_id,
            "action": KEY_CREATE_ACTION,
            "action_request_id": action_request_id,
        })
        .await?)
}

async fn find_rotation_receipt(
    db: &mongodb::Database,
    actor_user_id: &str,
    action_request_id: &str,
) -> AppResult<Option<AssistantActionReceipt>> {
    Ok(db
        .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .find_one(doc! {
            "user_id": actor_user_id,
            "action": KEY_ROTATE_ACTION,
            "action_request_id": action_request_id,
        })
        .await?)
}

async fn reserve_receipt(
    db: &mongodb::Database,
    user_id: &str,
    request: &KeyCreateActionRequest,
    fingerprint: &str,
) -> AppResult<AssistantActionReceipt> {
    if let Some(existing) = find_receipt(db, user_id, &request.action_request_id).await? {
        return Ok(existing);
    }

    let now = utc_now_at_bson_precision();
    let receipt = AssistantActionReceipt {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        action: KEY_CREATE_ACTION.to_string(),
        action_request_id: request.action_request_id.clone(),
        request_fingerprint: fingerprint.to_string(),
        resource_id: Uuid::new_v4().to_string(),
        resource_state_version: None,
        resource_access_revision: None,
        resource_material_fingerprint: None,
        status: AssistantActionReceiptStatus::Pending,
        created_at: now,
        completed_at: None,
    };
    match db
        .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .insert_one(&receipt)
        .await
    {
        Ok(_) => Ok(receipt),
        Err(error) if duplicate_key(&error) => {
            find_receipt(db, user_id, &request.action_request_id)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict("assistant action receipt reservation raced".to_string())
                })
        }
        Err(error) => Err(error.into()),
    }
}

async fn reserve_rotation_receipt(
    db: &mongodb::Database,
    actor_user_id: &str,
    request: &KeyRotateActionRequest,
    fingerprint: &str,
) -> AppResult<AssistantActionReceipt> {
    if let Some(existing) =
        find_rotation_receipt(db, actor_user_id, &request.action_request_id).await?
    {
        return Ok(existing);
    }

    let receipt = AssistantActionReceipt {
        id: Uuid::new_v4().to_string(),
        user_id: actor_user_id.to_string(),
        action: KEY_ROTATE_ACTION.to_string(),
        action_request_id: request.action_request_id.clone(),
        request_fingerprint: fingerprint.to_string(),
        resource_id: Uuid::new_v4().to_string(),
        resource_state_version: None,
        resource_access_revision: None,
        resource_material_fingerprint: None,
        status: AssistantActionReceiptStatus::Pending,
        created_at: utc_now_at_bson_precision(),
        completed_at: None,
    };
    match db
        .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .insert_one(&receipt)
        .await
    {
        Ok(_) => Ok(receipt),
        Err(error) if duplicate_key(&error) => {
            find_rotation_receipt(db, actor_user_id, &request.action_request_id)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict("assistant action receipt reservation raced".to_string())
                })
        }
        Err(error) => Err(error.into()),
    }
}

async fn recover_reserved_key(
    db: &mongodb::Database,
    user_id: &str,
    receipt: &AssistantActionReceipt,
) -> AppResult<Option<String>> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": &receipt.resource_id, "user_id": user_id })
        .await?;
    match key {
        Some(key) if key.is_active => Ok(Some(key.id)),
        Some(_) => Err(AppError::Conflict(
            "the key created for this action is no longer active".to_string(),
        )),
        None => Ok(None),
    }
}

async fn mark_completed(db: &mongodb::Database, receipt: &AssistantActionReceipt) -> AppResult<()> {
    db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
        .update_one(
            doc! { "_id": &receipt.id },
            doc! { "$set": {
                "status": to_bson(&AssistantActionReceiptStatus::Completed)
                    .map_err(|error| AppError::Internal(error.to_string()))?,
                "completed_at": mongodb::bson::DateTime::from_chrono(Utc::now()),
            } },
        )
        .await?;
    Ok(())
}

async fn create_reserved_key(
    db: &mongodb::Database,
    user_id: &str,
    request: &KeyCreateActionRequest,
    receipt: &AssistantActionReceipt,
) -> AppResult<KeyCreateActionResult> {
    let no_nodes: Vec<String> = Vec::new();
    let created = key_service::create_api_key_with_scope_authorization_and_id(
        db,
        user_id,
        None,
        &receipt.resource_id,
        &request.name,
        "proxy",
        None,
        Some("Created from a NyxID assistant action"),
        Some(&request.allowed_service_ids),
        Some(&no_nodes),
        Some(false),
        Some(false),
        None,
        None,
        Some(&request.platform),
        None,
        None,
    )
    .await;

    match created {
        Ok(created) => {
            mark_completed(db, receipt).await?;
            Ok(KeyCreateActionResult::Created(Box::new(created)))
        }
        Err(AppError::DatabaseError(error)) if duplicate_key(&error) => {
            let Some(key_id) = recover_reserved_key(db, user_id, receipt).await? else {
                return Err(AppError::DatabaseError(error));
            };
            mark_completed(db, receipt).await?;
            Ok(KeyCreateActionResult::Replayed { key_id })
        }
        Err(error) => Err(error),
    }
}

pub async fn create_key(
    db: &mongodb::Database,
    user_id: &str,
    request: KeyCreateActionRequest,
) -> AppResult<KeyCreateActionResult> {
    let request = normalize_request(request)?;
    let fingerprint = request_fingerprint(&request)?;
    if let Some(receipt) = find_receipt(db, user_id, &request.action_request_id).await? {
        if receipt.request_fingerprint != fingerprint {
            return Err(AppError::Conflict(
                "actionRequestId was already used with different key parameters".to_string(),
            ));
        }
        if let Some(key_id) = recover_reserved_key(db, user_id, &receipt).await? {
            if receipt.status != AssistantActionReceiptStatus::Completed {
                mark_completed(db, &receipt).await?;
            }
            return Ok(KeyCreateActionResult::Replayed { key_id });
        }

        validate_personal_service_ids(db, user_id, &request.allowed_service_ids).await?;
        return create_reserved_key(db, user_id, &request, &receipt).await;
    }

    validate_personal_service_ids(db, user_id, &request.allowed_service_ids).await?;
    let receipt = reserve_receipt(db, user_id, &request, &fingerprint).await?;
    if receipt.request_fingerprint != fingerprint {
        return Err(AppError::Conflict(
            "actionRequestId was already used with different key parameters".to_string(),
        ));
    }
    if let Some(key_id) = recover_reserved_key(db, user_id, &receipt).await? {
        if receipt.status != AssistantActionReceiptStatus::Completed {
            mark_completed(db, &receipt).await?;
        }
        return Ok(KeyCreateActionResult::Replayed { key_id });
    }

    create_reserved_key(db, user_id, &request, &receipt).await
}

async fn resolve_rotation_owner(
    db: &mongodb::Database,
    actor_user_id: &str,
    predecessor_id: &str,
) -> AppResult<String> {
    let predecessor = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": predecessor_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;
    let access = org_service::resolve_owner_access(db, actor_user_id, &predecessor.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to rotate this API key".to_string(),
        ));
    }
    Ok(predecessor.user_id)
}

async fn rotate_reserved_key(
    db: &mongodb::Database,
    actor_user_id: &str,
    owner_user_id: &str,
    request: &KeyRotateActionRequest,
    receipt: &AssistantActionReceipt,
) -> AppResult<KeyRotateActionResult> {
    let outcome = key_service::rotate_api_key_with_scope_authorization_and_id(
        db,
        owner_user_id,
        Some(actor_user_id),
        &request.key_id,
        &receipt.resource_id,
    )
    .await?;
    mark_completed(db, receipt).await?;
    Ok(match outcome {
        ApiKeyRotationOutcome::Created(created) => KeyRotateActionResult::Created {
            created: Box::new(created),
            requested_at: receipt.created_at,
        },
        ApiKeyRotationOutcome::AlreadyCommitted(successor) => KeyRotateActionResult::Replayed {
            key_id: successor.id,
            requested_at: receipt.created_at,
        },
    })
}

pub async fn rotate_key(
    db: &mongodb::Database,
    actor_user_id: &str,
    request: KeyRotateActionRequest,
) -> AppResult<KeyRotateActionResult> {
    let request = normalize_rotation_request(request)?;
    let fingerprint = rotation_request_fingerprint(&request)?;

    if let Some(receipt) =
        find_rotation_receipt(db, actor_user_id, &request.action_request_id).await?
    {
        if receipt.request_fingerprint != fingerprint {
            return Err(AppError::Conflict(
                "actionRequestId was already used with a different rotation predecessor"
                    .to_string(),
            ));
        }
        let owner_user_id = resolve_rotation_owner(db, actor_user_id, &request.key_id).await?;
        return rotate_reserved_key(db, actor_user_id, &owner_user_id, &request, &receipt).await;
    }

    let owner_user_id = resolve_rotation_owner(db, actor_user_id, &request.key_id).await?;
    let receipt = reserve_rotation_receipt(db, actor_user_id, &request, &fingerprint).await?;
    if receipt.request_fingerprint != fingerprint {
        return Err(AppError::Conflict(
            "actionRequestId was already used with a different rotation predecessor".to_string(),
        ));
    }
    rotate_reserved_key(db, actor_user_id, &owner_user_id, &request, &receipt).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use mongodb::{IndexModel, options::IndexOptions};

    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::test_utils::{connect_test_database, test_membership, test_user, test_user_service};

    fn request(ids: &[&str]) -> KeyCreateActionRequest {
        KeyCreateActionRequest {
            action_request_id: "action-alpha".to_string(),
            name: "  coding agent  ".to_string(),
            platform: "codex".to_string(),
            allowed_service_ids: ids.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    #[test]
    fn fingerprint_is_set_order_independent_and_pins_least_scope() {
        let left = normalize_request(request(&["svc-b", "svc-a"])).expect("normalize");
        let right = normalize_request(request(&["svc-a", "svc-b"])).expect("normalize");
        assert_eq!(
            request_fingerprint(&left).unwrap(),
            request_fingerprint(&right).unwrap()
        );
        assert_eq!(left.name, "coding agent");
    }

    #[test]
    fn rejects_empty_and_duplicate_service_sets() {
        assert!(matches!(
            normalize_request(request(&[])),
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            normalize_request(request(&["svc-a", "svc-a"])),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn rotation_fingerprint_pins_exact_predecessor_and_control_identity() {
        let predecessor_id = Uuid::new_v4().to_string();
        let normalized = normalize_rotation_request(KeyRotateActionRequest {
            action_request_id: "  action-rotate-alpha  ".to_string(),
            key_id: predecessor_id.clone(),
        })
        .expect("normalize exact rotation");
        assert_eq!(normalized.action_request_id, "action-rotate-alpha");
        assert_eq!(normalized.key_id, predecessor_id);

        let fingerprint = rotation_request_fingerprint(&normalized).expect("fingerprint");
        let other = normalize_rotation_request(KeyRotateActionRequest {
            action_request_id: normalized.action_request_id.clone(),
            key_id: Uuid::new_v4().to_string(),
        })
        .expect("normalize other predecessor");
        assert_ne!(
            fingerprint,
            rotation_request_fingerprint(&other).expect("other fingerprint")
        );

        for request in [
            KeyRotateActionRequest {
                action_request_id: "invalid/control".to_string(),
                key_id: Uuid::new_v4().to_string(),
            },
            KeyRotateActionRequest {
                action_request_id: "action-valid".to_string(),
                key_id: "not-a-uuid".to_string(),
            },
        ] {
            assert!(matches!(
                normalize_rotation_request(request),
                Err(AppError::ValidationError(_))
            ));
        }
    }

    async fn prepare_database(prefix: &str) -> Option<(mongodb::Database, String, String)> {
        let db = connect_test_database(prefix).await?;
        db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "action": 1, "action_request_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create receipt uniqueness index");
        let user_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .expect("insert key owner");
        let service_id = Uuid::new_v4().to_string();
        let service = test_user_service(
            &service_id,
            &user_id,
            "api-github",
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service)
            .await
            .expect("insert scoped user service");
        Some((db, user_id, service_id))
    }

    #[tokio::test]
    async fn org_visible_services_are_rejected_before_receipt_reservation() {
        let Some((db, user_id, _)) = prepare_database("assistant_key_create_org_owner").await
        else {
            return;
        };

        for (index, role) in [OrgRole::Member, OrgRole::Admin].into_iter().enumerate() {
            let org_id = Uuid::new_v4().to_string();
            let org_service_id = Uuid::new_v4().to_string();
            db.collection::<User>(USERS)
                .insert_one(test_user(&org_id, UserType::Org))
                .await
                .expect("insert org owner");
            db.collection::<UserService>(USER_SERVICES)
                .insert_one(test_user_service(
                    &org_service_id,
                    &org_id,
                    &format!("org-service-{index}"),
                    &Uuid::new_v4().to_string(),
                    None,
                    None,
                ))
                .await
                .expect("insert org service");
            db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                .insert_one(test_membership(
                    &org_id,
                    &user_id,
                    role,
                    Some(vec![org_service_id.clone()]),
                ))
                .await
                .expect("insert visible org membership");

            let mut org_request = exact_request(&org_service_id);
            org_request.action_request_id = format!("action-org-{index}");
            assert!(matches!(
                create_key(&db, &user_id, org_request).await,
                Err(AppError::ValidationError(_))
            ));
        }

        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! { "user_id": &user_id })
                .await
                .expect("count keys"),
            0
        );
        assert_eq!(
            db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! { "user_id": &user_id })
                .await
                .expect("count receipts"),
            0
        );
    }

    fn exact_request(service_id: &str) -> KeyCreateActionRequest {
        KeyCreateActionRequest {
            action_request_id: "action-exactly-once".to_string(),
            name: "coding-agent".to_string(),
            platform: "codex".to_string(),
            allowed_service_ids: vec![service_id.to_string()],
        }
    }

    #[tokio::test]
    async fn concurrent_reserved_effect_creates_once_and_recovers_duplicate_insert() {
        let Some((db, user_id, service_id)) = prepare_database("assistant_key_create_once").await
        else {
            return;
        };

        let request = normalize_request(exact_request(&service_id)).expect("normalize request");
        let fingerprint = request_fingerprint(&request).expect("fingerprint request");
        let receipt = reserve_receipt(&db, &user_id, &request, &fingerprint)
            .await
            .expect("reserve action receipt");

        // Both executions deliberately skip recovery and attempt the same reserved UUID.
        // The unique key insert selects the one caller allowed to receive key material.
        let first = create_reserved_key(&db, &user_id, &request, &receipt);
        let second = create_reserved_key(&db, &user_id, &request, &receipt);
        let (first, second) = tokio::join!(first, second);
        let first = first.expect("first execution");
        let second = second.expect("second execution");

        let created_count = [&first, &second]
            .into_iter()
            .filter(|result| matches!(result, KeyCreateActionResult::Created(_)))
            .count();
        let replayed_count = [&first, &second]
            .into_iter()
            .filter(|result| matches!(result, KeyCreateActionResult::Replayed { .. }))
            .count();
        assert_eq!(created_count, 1);
        assert_eq!(replayed_count, 1);

        let keys = db
            .collection::<ApiKey>(API_KEYS)
            .find(doc! { "user_id": &user_id })
            .await
            .expect("list created keys")
            .try_collect::<Vec<_>>()
            .await
            .expect("collect created keys");
        assert_eq!(keys.len(), 1);
        let key = &keys[0];
        assert_eq!(key.scopes, "proxy");
        assert_eq!(key.allowed_service_ids, vec![service_id]);
        assert!(key.allowed_node_ids.is_empty());
        assert!(!key.allow_all_services);
        assert!(!key.allow_all_nodes);

        let receipts = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find(doc! { "user_id": &user_id })
            .await
            .expect("list receipts")
            .try_collect::<Vec<_>>()
            .await
            .expect("collect receipts");
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].resource_id, key.id);
        assert_eq!(receipts[0].status, AssistantActionReceiptStatus::Completed);
    }

    #[tokio::test]
    async fn action_identity_reuse_with_different_parameters_fails_closed() {
        let Some((db, user_id, service_id)) =
            prepare_database("assistant_key_create_conflict").await
        else {
            return;
        };
        create_key(&db, &user_id, exact_request(&service_id))
            .await
            .expect("initial execution");

        let mut changed = exact_request(&service_id);
        changed.name = "different-agent".to_string();
        assert!(matches!(
            create_key(&db, &user_id, changed).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! { "user_id": &user_id })
                .await
                .expect("count keys"),
            1
        );
    }

    #[tokio::test]
    async fn exact_replay_survives_service_deactivation() {
        let Some((db, user_id, service_id)) =
            prepare_database("assistant_key_create_deactivated_replay").await
        else {
            return;
        };
        let created = create_key(&db, &user_id, exact_request(&service_id))
            .await
            .expect("initial execution");
        let KeyCreateActionResult::Created(created) = created else {
            panic!("initial execution must create the key");
        };

        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service_id, "user_id": &user_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("deactivate service");

        let replayed = create_key(&db, &user_id, exact_request(&service_id))
            .await
            .expect("durable exact replay");
        assert!(matches!(
            replayed,
            KeyCreateActionResult::Replayed { key_id } if key_id == created.id
        ));
        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! { "user_id": &user_id })
                .await
                .expect("count keys"),
            1
        );
    }
}
