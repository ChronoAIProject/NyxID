use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::node_pending_credential::{
    COLLECTION_NAME as NODE_PENDING_CREDENTIALS, InjectionMethod, NodePendingCredential,
};
use crate::services::{node_service, url_validation};

pub struct CreatePendingCredentialInput {
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub ttl_secs: i64,
}

pub async fn create_pending_credential(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    input: CreatePendingCredentialInput,
) -> AppResult<NodePendingCredential> {
    validate_service_slug(&input.service_slug)?;
    validate_field_name(&input.field_name, &input.injection_method)?;
    let target_url = clean_optional_string(input.target_url);
    if let Some(url) = target_url.as_deref() {
        url_validation::validate_public_http_url(url, "target_url").await?;
    }
    let label = clean_optional_string(input.label);
    if let Some(label) = label.as_deref()
        && label.len() > 128
    {
        return Err(AppError::ValidationError(
            "label must be 128 characters or fewer".to_string(),
        ));
    }

    let node = node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    let existing = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one(doc! {
            "node_id": node_id,
            "service_slug": &input.service_slug,
            "is_active": true,
        })
        .await?;
    if let Some(existing) = existing {
        return Err(AppError::Conflict(format!(
            "A pending credential already exists for service '{}' on this node (id: {})",
            input.service_slug, existing.id
        )));
    }

    let now = Utc::now();
    let expires_at = now + Duration::seconds(input.ttl_secs.max(1));
    let pending = NodePendingCredential {
        id: Uuid::new_v4().to_string(),
        node_id: node_id.to_string(),
        service_slug: input.service_slug,
        injection_method: input.injection_method,
        field_name: input.field_name,
        target_url,
        label,
        created_by_user_id: actor_user_id.to_string(),
        owner_user_id: node.user_id,
        created_at: now,
        expires_at,
        consumed_at: None,
        declined_at: None,
        is_active: true,
    };

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .insert_one(&pending)
        .await?;

    Ok(pending)
}

pub async fn list_pending_credentials_for_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    include_history: bool,
) -> AppResult<Vec<NodePendingCredential>> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;

    let mut filter = doc! { "node_id": node_id };
    if !include_history {
        filter.insert("is_active", true);
        filter.insert(
            "expires_at",
            doc! { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        );
    }

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn list_pending_credentials_for_node(
    db: &mongodb::Database,
    node_id: &str,
) -> AppResult<Vec<NodePendingCredential>> {
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find(doc! {
            "node_id": node_id,
            "is_active": true,
            "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
}

pub async fn cancel_pending_credential(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;

    // Consume rejects expired pushes because accepting stale setup metadata is
    // correctness-critical. Cancel intentionally remains admin housekeeping:
    // it can deactivate an expired active row so cleanup is idempotent.
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
            },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

pub async fn consume_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    complete_pending_credential_for_node(db, node_id, pending_id, CompletionKind::Consumed).await
}

pub async fn decline_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
) -> AppResult<NodePendingCredential> {
    complete_pending_credential_for_node(db, node_id, pending_id, CompletionKind::Declined).await
}

enum CompletionKind {
    Consumed,
    Declined,
}

async fn complete_pending_credential_for_node(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: &str,
    kind: CompletionKind,
) -> AppResult<NodePendingCredential> {
    let now_chrono = Utc::now();
    let now = bson::DateTime::from_chrono(now_chrono);
    let timestamp_field = match kind {
        CompletionKind::Consumed => "consumed_at",
        CompletionKind::Declined => "declined_at",
    };
    let mut set_doc = doc! { "is_active": false };
    set_doc.insert(timestamp_field, now);

    db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .find_one_and_update(
            doc! {
                "_id": pending_id,
                "node_id": node_id,
                "is_active": true,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now_chrono) },
            },
            doc! { "$set": set_doc },
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Pending credential not found".to_string()))
}

fn validate_service_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty() || slug.len() > 64 {
        return Err(AppError::ValidationError(
            "service_slug must be 1-64 characters".to_string(),
        ));
    }
    let valid = slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && slug
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid {
        return Err(AppError::ValidationError(
            "service_slug must be lowercase alphanumeric with optional hyphens, and cannot start or end with hyphen".to_string(),
        ));
    }
    Ok(())
}

fn validate_field_name(field_name: &str, injection_method: &InjectionMethod) -> AppResult<()> {
    if field_name.is_empty() || field_name.len() > 128 {
        return Err(AppError::ValidationError(
            "field_name must be 1-128 characters".to_string(),
        ));
    }

    match injection_method {
        InjectionMethod::Header => {
            for ch in field_name.chars() {
                if !is_http_token_char(ch) {
                    return Err(disallowed_field_char_error("header", ch));
                }
            }
        }
        InjectionMethod::QueryParam => {
            validate_percent_encoding(field_name, "query-param")?;
            for ch in field_name.chars() {
                if ch == '%' {
                    continue;
                }
                if !is_rfc3986_unreserved(ch) {
                    return Err(disallowed_field_char_error("query-param", ch));
                }
            }
        }
        InjectionMethod::PathPrefix => {
            validate_percent_encoding(field_name, "path-prefix")?;
            for ch in field_name.chars() {
                if ch == '%' {
                    continue;
                }
                if ch.is_control() || ch.is_whitespace() || matches!(ch, '?' | '#') {
                    return Err(disallowed_field_char_error("path-prefix", ch));
                }
                if !ch.is_ascii() {
                    return Err(disallowed_field_char_error("path-prefix", ch));
                }
            }
        }
    }

    Ok(())
}

fn is_http_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
        )
}

fn is_rfc3986_unreserved(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~')
}

fn validate_percent_encoding(value: &str, method: &str) -> AppResult<()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let valid = index + 2 < bytes.len()
                && bytes[index + 1].is_ascii_hexdigit()
                && bytes[index + 2].is_ascii_hexdigit();
            if !valid {
                return Err(AppError::ValidationError(format!(
                    "field_name for {method} contains invalid percent-encoding"
                )));
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn disallowed_field_char_error(method: &str, ch: char) -> AppError {
    let display = match ch {
        ' ' => "space".to_string(),
        '\t' => "tab".to_string(),
        '\n' => "newline".to_string(),
        '\r' => "carriage return".to_string(),
        _ => ch.to_string(),
    };
    AppError::ValidationError(format!(
        "field_name for {method} contains disallowed character '{display}'"
    ))
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::node_service;
    use crate::test_utils::{connect_test_database, test_membership, test_user};

    fn test_node(owner_id: &str, name: &str) -> Node {
        let now = Utc::now();
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn credential_input(service_slug: &str) -> CreatePendingCredentialInput {
        CreatePendingCredentialInput {
            service_slug: service_slug.to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: Some("Production".to_string()),
            ttl_secs: 86_400,
        }
    }

    async fn insert_users(db: &mongodb::Database, users: Vec<User>) {
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert users");
    }

    async fn insert_membership(db: &mongodb::Database, membership: OrgMembership) {
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(membership)
            .await
            .expect("insert membership");
    }

    async fn insert_node(db: &mongodb::Database, node: &Node) {
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");
    }

    async fn load_pending(db: &mongodb::Database, pending_id: &str) -> NodePendingCredential {
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": pending_id })
            .await
            .expect("query pending credential")
            .expect("pending credential exists")
    }

    fn assert_invalid_field_name(method: InjectionMethod, field_name: &str, expected: &str) {
        let err = validate_field_name(field_name, &method).expect_err("field name should fail");
        assert!(
            matches!(err, AppError::ValidationError(ref message) if message.contains(expected)),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn validates_header_field_name_as_http_token() {
        validate_field_name("X-API-Key", &InjectionMethod::Header).expect("valid header");
        validate_field_name("X_Custom!#$%&'*+-.^`|~", &InjectionMethod::Header)
            .expect("valid token chars");

        assert_invalid_field_name(InjectionMethod::Header, "X API Key", "space");
        assert_invalid_field_name(InjectionMethod::Header, "X:API-Key", ":");
        assert_invalid_field_name(InjectionMethod::Header, "X,API-Key", ",");
        assert_invalid_field_name(InjectionMethod::Header, "X-ÄPI-Key", "Ä");
    }

    #[test]
    fn validates_query_param_field_name_as_url_safe() {
        validate_field_name("api_key", &InjectionMethod::QueryParam).expect("valid param");
        validate_field_name("api-key.%7E", &InjectionMethod::QueryParam)
            .expect("valid percent-encoded param");

        assert_invalid_field_name(InjectionMethod::QueryParam, "api key", "space");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api&key", "&");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api=key", "=");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api?key", "?");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api#key", "#");
        assert_invalid_field_name(InjectionMethod::QueryParam, "api%key", "percent-encoding");
    }

    #[test]
    fn validates_path_prefix_field_name_as_path_component() {
        validate_field_name("v1/api/%2Ftenant", &InjectionMethod::PathPrefix)
            .expect("valid path prefix");

        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api key", "space");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api?key", "?");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/api#key", "#");
        assert_invalid_field_name(InjectionMethod::PathPrefix, "v1/%key", "percent-encoding");
    }

    #[tokio::test]
    async fn admin_push_creates_pending_credential_with_acl_fields() {
        let Some(db) = connect_test_database("pending_credential_push").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &admin_id, OrgRole::Admin, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;

        let pending =
            create_pending_credential(&db, &admin_id, &node.id, credential_input("openclaw"))
                .await
                .expect("admin can push");

        assert_eq!(pending.node_id, node.id);
        assert_eq!(pending.service_slug, "openclaw");
        assert_eq!(pending.created_by_user_id, admin_id);
        assert_eq!(pending.owner_user_id, org_id);
        assert!(pending.is_active);

        let listed = list_pending_credentials_for_admin(&db, &admin_id, &node.id, false)
            .await
            .expect("admin can list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, pending.id);
    }

    #[tokio::test]
    async fn member_cannot_push_pending_credential_for_org_node() {
        let Some(db) = connect_test_database("pending_credential_member_denied").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &member_id, OrgRole::Member, None),
        )
        .await;
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;

        let err =
            create_pending_credential(&db, &member_id, &node.id, credential_input("openclaw"))
                .await
                .expect_err("member cannot push");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn push_for_nonexistent_node_returns_not_found() {
        let Some(db) = connect_test_database("pending_credential_missing_node").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;

        let err = create_pending_credential(
            &db,
            &actor_id,
            &Uuid::new_v4().to_string(),
            credential_input("openclaw"),
        )
        .await
        .expect_err("missing node should fail");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn duplicate_pending_slug_returns_conflict_with_existing_id() {
        let Some(db) = connect_test_database("pending_credential_duplicate").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;

        let first =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("first push succeeds");
        let err = create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
            .await
            .expect_err("duplicate push should fail");

        match err {
            AppError::Conflict(message) => {
                assert!(message.contains(&first.id));
            }
            other => panic!("expected conflict, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn push_rejects_internal_target_url() {
        let Some(db) = connect_test_database("pending_credential_internal_url").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let mut input = credential_input("openclaw");
        input.target_url = Some("http://127.0.0.1:8080".to_string());

        let err = create_pending_credential(&db, &actor_id, &node.id, input)
            .await
            .expect_err("internal URL should fail");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn node_consumes_own_pending_credential() {
        let Some(db) = connect_test_database("pending_credential_consume").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let returned = consume_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect("node consumes own pending");
        assert_eq!(returned.id, pending.id);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.consumed_at.is_some());
        assert!(stored.declined_at.is_none());
    }

    #[tokio::test]
    async fn node_cannot_consume_another_nodes_pending_credential() {
        let Some(db) = connect_test_database("pending_credential_wrong_node").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node_a = test_node(&actor_id, "node-a");
        let node_b = test_node(&actor_id, "node-b");
        insert_node(&db, &node_a).await;
        insert_node(&db, &node_b).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node_a.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let err = consume_pending_credential_for_node(&db, &node_b.id, &pending.id)
            .await
            .expect_err("other node cannot consume");
        assert!(matches!(err, AppError::NotFound(_)));

        let stored = load_pending(&db, &pending.id).await;
        assert!(stored.is_active);
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn node_declines_pending_credential() {
        let Some(db) = connect_test_database("pending_credential_decline").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        decline_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect("node declines");

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.declined_at.is_some());
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn admin_cancel_prevents_later_consume() {
        let Some(db) = connect_test_database("pending_credential_cancel").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        cancel_pending_credential(&db, &actor_id, &node.id, &pending.id)
            .await
            .expect("admin cancels");

        let err = consume_pending_credential_for_node(&db, &node.id, &pending.id)
            .await
            .expect_err("canceled row is not consumable");
        assert!(matches!(err, AppError::NotFound(_)));

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert!(stored.consumed_at.is_none());
    }

    #[tokio::test]
    async fn expired_pending_credentials_are_not_listed() {
        let Some(db) = connect_test_database("pending_credential_expired").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let now = Utc::now();
        let expired = NodePendingCredential {
            id: Uuid::new_v4().to_string(),
            node_id: node.id.clone(),
            service_slug: "expired".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: actor_id.clone(),
            owner_user_id: actor_id.clone(),
            created_at: now - Duration::hours(2),
            expires_at: now - Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            is_active: true,
        };
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .insert_one(&expired)
            .await
            .expect("insert expired pending");

        let admin_list = list_pending_credentials_for_admin(&db, &actor_id, &node.id, false)
            .await
            .expect("admin list succeeds");
        let node_list = list_pending_credentials_for_node(&db, &node.id)
            .await
            .expect("node list succeeds");

        assert!(admin_list.is_empty());
        assert!(node_list.is_empty());
    }

    #[tokio::test]
    async fn transfer_deactivates_pending_credentials_for_node() {
        let Some(db) = connect_test_database("pending_credential_transfer").await else {
            eprintln!("skipping pending credential test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        insert_membership(
            &db,
            test_membership(&org_id, &actor_id, OrgRole::Admin, None),
        )
        .await;
        let node = test_node(&actor_id, "personal-node");
        insert_node(&db, &node).await;
        let pending =
            create_pending_credential(&db, &actor_id, &node.id, credential_input("openclaw"))
                .await
                .expect("push succeeds");

        let transfer = node_service::transfer_node_owner(&db, &actor_id, &node.id, &org_id, 10)
            .await
            .expect("transfer succeeds");
        assert_eq!(transfer.deactivated_pending_credentials_count, 1);

        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
    }
}
