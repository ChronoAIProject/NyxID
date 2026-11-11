use super::*;
use crate::{
    crypto::token::{agent_key_display_prefix, hash_token},
    models::{
        api_key::COLLECTION_NAME as API_KEYS, api_key_credential::CredentialRevokedReason,
        machine_external_grant::COLLECTION_NAME as GRANTS,
    },
    services::{
        api_key_credential_service as credentials, api_key_mutation_service as mutations,
        key_service,
    },
    test_utils::connect_transaction_test_database,
};
use chrono::Duration;
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

struct Fixture {
    db: Database,
    owner: String,
    parent: String,
    grant: String,
    secret: Zeroizing<String>,
}

async fn fixture(name: &str) -> Fixture {
    let db = connect_transaction_test_database(name).await;
    let owner = Uuid::new_v4().to_string();
    let parent = Uuid::new_v4().to_string();
    let grant = Uuid::new_v4().to_string();
    let secret = credentials::generate_secret();
    db.collection::<Document>(API_KEYS).insert_one(doc! {
        "_id": &parent, "user_id": &owner, "name": "external source fixture",
        "key_prefix": agent_key_display_prefix(&secret).unwrap(), "key_hash": hash_token(&secret),
        "scopes": "proxy", "is_active": true, "created_at": bson::DateTime::now(),
        "state_version": 1_i64, "updated_at": bson::DateTime::now(),
        "machine_external_fence": 1_i64, "machine_external_grants_present": true,
    }).await.unwrap();
    db.collection::<Document>(GRANTS).insert_one(doc! {
        "_id": &grant, "user_id": &owner,
        "binding": {"kind": "api_key", "api_key_id": &parent, "api_key_state_version_at_approval": 1_i64},
        "status": "active", "revision": 1_i64, "admission_fence": 0_i64,
    }).await.unwrap();
    Fixture {
        db,
        owner,
        parent,
        grant,
        secret,
    }
}

async fn row(db: &Database, collection: &str, id: &str) -> Document {
    db.collection::<Document>(collection)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

async fn revocations(db: &Database) -> Vec<Document> {
    use futures::TryStreamExt;
    db.collection::<Document>(crate::models::machine_access::OUTBOX)
        .find(doc! {"kind": "external"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

fn cancellation_selector(row: &Document) -> SourceSelector {
    bson::from_bson(row.get("selector").unwrap().clone()).unwrap()
}

async fn child(f: &Fixture, expires_at: Option<DateTime<Utc>>) -> (String, Zeroizing<String>) {
    let id = Uuid::new_v4().to_string();
    let secret = credentials::generate_secret();
    f.db.collection::<Document>(CREDENTIALS).insert_one(doc! {
        "_id": &id, "api_key_id": &f.parent, "user_id": &f.owner,
        "secret_hash": hash_token(&secret), "secret_prefix": agent_key_display_prefix(&secret).unwrap(),
        "label": "test child", "login_request_id": Uuid::new_v4().to_string(), "is_active": true,
        "expires_at": expires_at.map(bson::DateTime::from_chrono), "created_at": bson::DateTime::now(),
    }).await.unwrap();
    (id, secret)
}

#[tokio::test]
async fn external_source_repeated_authentication_and_labels_preserve_grant() {
    let f = fixture("extkey_use").await;
    for _ in 0..4 {
        key_service::validate_api_key(&f.db, &f.secret)
            .await
            .unwrap();
    }
    mutations::update_one(
        &f.db,
        doc! {"_id": &f.parent},
        doc! {"$set": {
            "name": "renamed", "description": "display only", "platform": "codex",
        }},
        None,
    )
    .await
    .unwrap();
    let parent = row(&f.db, API_KEYS, &f.parent).await;
    assert_eq!(parent.get_i64("state_version").unwrap(), 6);
    assert_eq!(source_fence(&parent).unwrap(), 1);
    let grant = row(&f.db, GRANTS, &f.grant).await;
    assert_eq!(grant.get_str("status").unwrap(), "active");
    assert_eq!(grant.get_i64("revision").unwrap(), 1);
    assert!(revocations(&f.db).await.is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_reviewed_child_issuance_preserves_grant_until_parent_security_changes() {
    use crate::models::{
        api_key::ApiKey,
        api_key_credential::ApiKeyCredential,
        login_client_context::LoginClientContext,
        user::{COLLECTION_NAME as USERS, UserType},
    };
    use crate::services::auth_agent_key_login_service::{self as login, Selection};

    let f = fixture("extkey_reviewed_child").await;
    f.db.collection(USERS)
        .insert_one(crate::test_utils::test_user(&f.owner, UserType::Person))
        .await
        .unwrap();
    let parent =
        f.db.collection::<ApiKey>(API_KEYS)
            .find_one(doc! {"_id": &f.parent})
            .await
            .unwrap()
            .unwrap();
    let reviewed = login::key_summary(&f.db, &parent, false).await.unwrap();
    let before_parent = row(&f.db, API_KEYS, &f.parent).await;
    let before_grant = row(&f.db, GRANTS, &f.grant).await;
    let child_id = Uuid::new_v4().to_string();
    let request_id = Uuid::new_v4().to_string();
    let secret = credentials::generate_secret();
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    login::issue_selected(
        &f.db,
        &f.owner,
        &request_id,
        &LoginClientContext::default(),
        Some("reviewed-child"),
        &Selection::Existing {
            api_key_id: f.parent.clone(),
            permission_snapshot: Some(reviewed.permission_snapshot),
        },
        Some(Utc::now() + Duration::minutes(5)),
        Utc::now(),
        &f.parent,
        &child_id,
        &secret,
        &mut session,
    )
    .await
    .unwrap();

    let pending_parent =
        f.db.collection::<Document>(API_KEYS)
            .find_one(doc! {"_id": &f.parent})
            .session(&mut session)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        pending_parent.get_i64("state_version").unwrap(),
        parent.state_version + 1
    );
    assert_eq!(
        source_fence(&pending_parent).unwrap(),
        source_fence(&before_parent).unwrap()
    );
    assert!(
        pending_parent
            .get_bool("machine_external_grants_present")
            .unwrap()
    );
    let pending_grant =
        f.db.collection::<Document>(GRANTS)
            .find_one(doc! {"_id": &f.grant})
            .session(&mut session)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        pending_grant, before_grant,
        "reviewing unchanged parent authority must preserve the exact active grant"
    );
    assert_eq!(
        f.db.collection::<Document>(crate::models::machine_access::OUTBOX)
            .count_documents(doc! {"kind": "external"})
            .session(&mut session)
            .await
            .unwrap(),
        0
    );
    assert_eq!(row(&f.db, API_KEYS, &f.parent).await, before_parent);
    assert!(
        f.db.collection::<Document>(CREDENTIALS)
            .find_one(doc! {"_id": &child_id})
            .await
            .unwrap()
            .is_none()
    );
    session.commit_transaction().await.unwrap();

    let committed_parent = row(&f.db, API_KEYS, &f.parent).await;
    assert_eq!(
        committed_parent.get_i64("state_version").unwrap(),
        parent.state_version + 1
    );
    assert_eq!(
        source_fence(&committed_parent).unwrap(),
        source_fence(&before_parent).unwrap()
    );
    assert!(
        committed_parent
            .get_bool("machine_external_grants_present")
            .unwrap()
    );
    assert_eq!(row(&f.db, GRANTS, &f.grant).await, before_grant);
    assert!(revocations(&f.db).await.is_empty());
    let issued =
        f.db.collection::<ApiKeyCredential>(CREDENTIALS)
            .find_one(doc! {"_id": &child_id})
            .await
            .unwrap()
            .unwrap();
    assert!(issued.is_active);
    assert_eq!(issued.api_key_id, f.parent);
    assert_eq!(issued.user_id, f.owner);
    assert_eq!(issued.login_request_id, request_id);
    assert_eq!(issued.secret_hash, hash_token(&secret));

    mutations::update_one(
        &f.db,
        doc! {"_id": &f.parent, "state_version": parent.state_version + 1},
        doc! {"$set": {"scopes": "read"}},
        None,
    )
    .await
    .unwrap();
    let changed_parent = row(&f.db, API_KEYS, &f.parent).await;
    assert_eq!(
        changed_parent.get_i64("state_version").unwrap(),
        parent.state_version + 2
    );
    assert_eq!(
        source_fence(&changed_parent).unwrap(),
        source_fence(&before_parent).unwrap() + 1
    );
    assert!(
        changed_parent
            .get_bool("machine_external_grants_present")
            .unwrap()
    );
    let revoked = row(&f.db, GRANTS, &f.grant).await;
    assert_eq!(revoked.get_str("status").unwrap(), "revoked");
    assert_eq!(
        revoked.get_i64("revision").unwrap(),
        before_grant.get_i64("revision").unwrap() + 1
    );
    assert!(
        revoked.get_i64("admission_fence").unwrap()
            > before_grant.get_i64("admission_fence").unwrap()
    );
    let queued = revocations(&f.db).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(
        cancellation_selector(&queued[0]),
        SourceSelector::Grant {
            user_id: f.owner.clone(),
            grant_id: f.grant.clone(),
            through_revision: 1,
        }
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_ordinary_auth_and_ungranted_security_edits_do_not_query_machine() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = fixture("extkey_no_reads").await;
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let observed = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "extkey_monitor",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && ["find", "aggregate", "count", "findAndModify", "update"]
                    .iter()
                    .any(|key| {
                        event
                            .command
                            .get_str(*key)
                            .is_ok_and(|name| name.starts_with("machine_"))
                    })
            {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await;
    let monitored = observed.client().database(f.db.name());
    key_service::validate_api_key(&monitored, &f.secret)
        .await
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    mutations::update_one(
        &monitored,
        doc! {"_id": &f.parent},
        doc! {"$set": {"name":"display name", "description":"display only", "platform":"codex"}},
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        reads.load(Ordering::SeqCst),
        0,
        "benign changes must not query Machine even when a grant is present"
    );
    // A different newly created source has never had a Machine grant.
    let mut virgin = row(&f.db, API_KEYS, &f.parent).await;
    let virgin_id = Uuid::new_v4().to_string();
    virgin.insert("_id", &virgin_id);
    virgin.insert("key_hash", hash_token(&credentials::generate_secret()));
    virgin.remove("machine_external_grants_present");
    virgin.remove("machine_external_fence");
    f.db.collection::<Document>(API_KEYS)
        .insert_one(virgin)
        .await
        .unwrap();
    mutations::update_one(
        &monitored,
        doc! {"_id": &virgin_id},
        doc! {"$set": {"allow_all_nodes": false}},
        None,
    )
    .await
    .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    observed.drop().await.unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_security_mutation_and_grant_invalidation_commit_or_abort_together() {
    let f = fixture("extkey_atomic").await;
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    mutations::update_one(
        &f.db,
        doc! {"_id": &f.parent, "state_version": 1_i64},
        doc! {"$set": {"scopes": "read"}},
        Some(&mut session),
    )
    .await
    .unwrap();
    let pending =
        f.db.collection::<Document>(crate::models::machine_access::OUTBOX)
            .find_one(doc! {"kind": "external"})
            .session(&mut session)
            .await
            .unwrap()
            .expect("source mutation must enqueue before commit");
    assert_eq!(
        cancellation_selector(&pending),
        SourceSelector::Grant {
            user_id: f.owner.clone(),
            grant_id: f.grant.clone(),
            through_revision: 1,
        }
    );
    assert!(revocations(&f.db).await.is_empty());
    // Neither side is observable before the transaction commits.
    assert_eq!(
        row(&f.db, API_KEYS, &f.parent)
            .await
            .get_str("scopes")
            .unwrap(),
        "proxy"
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_str("status")
            .unwrap(),
        "active"
    );
    session.abort_transaction().await.unwrap();
    assert!(revocations(&f.db).await.is_empty());
    assert_eq!(
        source_fence(&row(&f.db, API_KEYS, &f.parent).await).unwrap(),
        1
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_i64("revision")
            .unwrap(),
        1
    );

    let stale = mutations::update_one(
        &f.db,
        doc! {"_id": &f.parent, "state_version": 99_i64},
        doc! {"$set": {"scopes": "read"}},
        None,
    )
    .await
    .unwrap();
    assert_eq!(stale.matched_count, 0);
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_str("status")
            .unwrap(),
        "active"
    );
    mutations::update_one(
        &f.db,
        doc! {"_id": &f.parent, "state_version": 1_i64},
        doc! {"$set": {"scopes": "read"}},
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        source_fence(&row(&f.db, API_KEYS, &f.parent).await).unwrap(),
        2
    );
    let grant = row(&f.db, GRANTS, &f.grant).await;
    assert_eq!(grant.get_str("status").unwrap(), "revoked");
    assert_eq!(grant.get_i64("revision").unwrap(), 2);
    let queued = revocations(&f.db).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(
        cancellation_selector(&queued[0]),
        SourceSelector::Grant {
            user_id: f.owner.clone(),
            grant_id: f.grant.clone(),
            through_revision: 1,
        }
    );
    // Later security changes are monotonic source fences, but do not rewrite
    // historical grant revocation or make a revoked grant active again.
    mutations::update_one(
        &f.db,
        doc! {"_id": &f.parent},
        doc! {"$set": {"allow_all_nodes": false}},
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        source_fence(&row(&f.db, API_KEYS, &f.parent).await).unwrap(),
        3
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_i64("revision")
            .unwrap(),
        2
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_individual_child_revoke_is_precise_atomic_and_idempotent() {
    let f = fixture("extkey_child").await;
    let (first, first_secret) = child(&f, None).await;
    let (sibling, sibling_secret) = child(&f, None).await;
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let revoked = credentials::revoke_in_session(
        &f.db,
        &first,
        CredentialRevokedReason::Logout,
        &mut session,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        revoked._source_invalidation.selector
            == SourceSelector::Child {
                user_id: f.owner.clone(),
                api_key_id: f.parent.clone(),
                api_key_credential_id: first.clone(),
            }
    );
    assert_eq!(revoked._source_invalidation.source_fence, 1);
    assert_eq!(
        f.db.collection::<Document>(crate::models::machine_access::OUTBOX)
            .count_documents(doc! {"kind": "external"})
            .session(&mut session)
            .await
            .unwrap(),
        1,
        "child receipt must already be durable in the caller's transaction"
    );
    assert!(revocations(&f.db).await.is_empty());
    assert!(
        row(&f.db, CREDENTIALS, &first)
            .await
            .get_bool("is_active")
            .unwrap()
    );
    session.abort_transaction().await.unwrap();
    assert!(revocations(&f.db).await.is_empty());
    assert!(
        key_service::validate_api_key(&f.db, &first_secret)
            .await
            .is_ok()
    );
    credentials::revoke(&f.db, &first, CredentialRevokedReason::Logout)
        .await
        .unwrap();
    let queued = revocations(&f.db).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(
        cancellation_selector(&queued[0]),
        SourceSelector::Child {
            user_id: f.owner.clone(),
            api_key_id: f.parent.clone(),
            api_key_credential_id: first.clone(),
        }
    );
    credentials::revoke(&f.db, &first, CredentialRevokedReason::Logout)
        .await
        .unwrap();
    assert_eq!(
        source_fence(&row(&f.db, CREDENTIALS, &first).await).unwrap(),
        1
    );
    assert!(
        key_service::validate_api_key(&f.db, &first_secret)
            .await
            .is_err()
    );
    assert!(
        key_service::validate_api_key(&f.db, &sibling_secret)
            .await
            .is_ok()
    );
    assert!(
        row(&f.db, CREDENTIALS, &sibling)
            .await
            .get_bool("is_active")
            .unwrap()
    );
    assert_eq!(
        source_fence(&row(&f.db, API_KEYS, &f.parent).await).unwrap(),
        1
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_str("status")
            .unwrap(),
        "active"
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_i64("revision")
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_child_admission_rejects_wrong_owner_parent_and_expiry_immediately() {
    let f = fixture("extkey_live").await;
    let (active, _) = child(&f, Some(Utc::now() + Duration::hours(1))).await;
    let (expired, _) = child(&f, Some(Utc::now() - Duration::seconds(1))).await;
    for (owner, parent, id) in [
        ("wrong-owner", f.parent.as_str(), active.as_str()),
        (f.owner.as_str(), "wrong-parent", active.as_str()),
        (f.owner.as_str(), f.parent.as_str(), expired.as_str()),
    ] {
        let mut session = f.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            fence_child_in_session(&f.db, owner, parent, id, &mut session).await,
            Err(AppError::MachinePermissionRevoked)
        ));
        session.abort_transaction().await.unwrap();
    }
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let result = fence_child_in_session(&f.db, &f.owner, &f.parent, &active, &mut session)
        .await
        .unwrap();
    assert_eq!(result._source_fence, 1);
    assert!(result.expires_at.is_some());
    session.commit_transaction().await.unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_stale_child_admission_conflicts_with_committed_revoke() {
    let f = fixture("extkey_race").await;
    let (child_id, _) = child(&f, None).await;
    let mut stale = f.db.client().start_session().await.unwrap();
    stale.start_transaction().await.unwrap();
    f.db.collection::<Document>(CREDENTIALS)
        .find_one(doc! {"_id": &child_id})
        .session(&mut stale)
        .await
        .unwrap()
        .unwrap();
    credentials::revoke(&f.db, &child_id, CredentialRevokedReason::WebRevoke)
        .await
        .unwrap();
    let result = fence_child_in_session(&f.db, &f.owner, &f.parent, &child_id, &mut stale).await;
    assert!(matches!(result, Err(AppError::DatabaseError(ref error))
        if error.contains_label("TransientTransactionError")));
    let _ = stale.abort_transaction().await;
    let mut fresh = f.db.client().start_session().await.unwrap();
    fresh.start_transaction().await.unwrap();
    assert!(matches!(
        fence_child_in_session(&f.db, &f.owner, &f.parent, &child_id, &mut fresh).await,
        Err(AppError::MachinePermissionRevoked)
    ));
    fresh.abort_transaction().await.unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_rotation_revokes_parent_and_children_without_successor_inheritance() {
    let f = fixture("extkey_rotate").await;
    let (child_id, child_secret) = child(&f, None).await;
    let successor = key_service::rotate_api_key(
        &f.db,
        &Arc::new(crate::test_utils::test_encryption_keys()),
        &f.owner,
        &f.parent,
    )
    .await
    .unwrap();
    assert_ne!(successor.id, f.parent);
    assert!(
        !row(&f.db, API_KEYS, &f.parent)
            .await
            .get_bool("is_active")
            .unwrap()
    );
    assert!(
        !row(&f.db, CREDENTIALS, &child_id)
            .await
            .get_bool("is_active")
            .unwrap()
    );
    assert!(
        key_service::validate_api_key(&f.db, &f.secret)
            .await
            .is_err()
    );
    assert!(
        key_service::validate_api_key(&f.db, &child_secret)
            .await
            .is_err()
    );
    assert!(
        key_service::validate_api_key(&f.db, &successor.full_key)
            .await
            .is_ok()
    );
    let grant = row(&f.db, GRANTS, &f.grant).await;
    assert_eq!(grant.get_str("status").unwrap(), "revoked");
    assert_eq!(grant.get_i64("revision").unwrap(), 2);
    assert_eq!(
        f.db.collection::<Document>(GRANTS)
            .count_documents(doc! {
                "binding.api_key_id": &successor.id,
            })
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_soft_delete_revokes_parent_grant_and_children() {
    let f = fixture("extkey_delete").await;
    let (child_id, _) = child(&f, None).await;
    key_service::delete_api_key_with_expected_state_version(&f.db, &f.owner, &f.parent, Some(1))
        .await
        .unwrap();
    assert!(
        !row(&f.db, API_KEYS, &f.parent)
            .await
            .get_bool("is_active")
            .unwrap()
    );
    assert!(
        !row(&f.db, CREDENTIALS, &child_id)
            .await
            .get_bool("is_active")
            .unwrap()
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_str("status")
            .unwrap(),
        "revoked"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_abandoned_provisioning_delete_fences_visible_grant_atomically() {
    let f = fixture("extkey_cleanup").await;
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert_eq!(
        mutations::delete_one(&f.db, &f.owner, &f.parent, Some(&mut session))
            .await
            .unwrap()
            .deleted_count,
        1
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_str("status")
            .unwrap(),
        "active"
    );
    session.abort_transaction().await.unwrap();
    assert!(
        key_service::validate_api_key(&f.db, &f.secret)
            .await
            .is_ok()
    );
    assert_eq!(
        mutations::delete_one(&f.db, "wrong-owner", &f.parent, None)
            .await
            .unwrap()
            .deleted_count,
        0
    );
    assert_eq!(
        mutations::delete_one(&f.db, &f.owner, &f.parent, None)
            .await
            .unwrap()
            .deleted_count,
        1
    );
    assert!(
        f.db.collection::<Document>(API_KEYS)
            .find_one(doc! {"_id": &f.parent})
            .await
            .unwrap()
            .is_none()
    );
    let grant = row(&f.db, GRANTS, &f.grant).await;
    assert_eq!(grant.get_str("status").unwrap(), "revoked");
    assert_eq!(grant.get_i64("revision").unwrap(), 2);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_machine_approval_consumes_current_or_legacy_cas_once() {
    for expected in [0_i64, 1_i64] {
        let f = fixture("extkey_approval").await;
        f.db.collection::<Document>(GRANTS)
            .delete_one(doc! {"_id": &f.grant})
            .await
            .unwrap();
        if expected == 0 {
            f.db.collection::<Document>(API_KEYS).update_one(doc! {"_id": &f.parent},
                doc! {"$unset": {"state_version": "", "machine_external_fence": "", "machine_external_grants_present": ""}})
                .await.unwrap();
        }
        let mut session = f.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        mutations::approve_machine_grant_source_in_session(
            &f.db,
            &f.owner,
            &f.parent,
            expected,
            &mut session,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let parent = row(&f.db, API_KEYS, &f.parent).await;
        assert_eq!(parent.get_i64("state_version").unwrap(), expected + 1);
        assert_eq!(source_fence(&parent).unwrap(), expected + 1);
        assert!(parent.get_bool("machine_external_grants_present").unwrap());
        session.start_transaction().await.unwrap();
        assert!(matches!(
            mutations::approve_machine_grant_source_in_session(
                &f.db,
                &f.owner,
                &f.parent,
                expected,
                &mut session
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        session.abort_transaction().await.unwrap();
        assert_eq!(
            row(&f.db, API_KEYS, &f.parent)
                .await
                .get_i64("state_version")
                .unwrap(),
            expected + 1
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn external_source_outbox_failure_aborts_actual_key_security_entrypoint() {
    let f = fixture("extkey_outbox_failure").await;
    f.db.create_collection(crate::models::machine_access::OUTBOX)
        .validator(doc! {"test_only_reject_receipts": {"$exists": true}})
        .await
        .unwrap();
    let result = mutations::update_one(
        &f.db,
        doc! {"_id": &f.parent},
        doc! {"$set": {"is_active": false}},
        None,
    )
    .await;
    assert!(
        result.is_err(),
        "outbox durability failure must reject the source mutation"
    );
    let parent = row(&f.db, API_KEYS, &f.parent).await;
    assert!(parent.get_bool("is_active").unwrap());
    assert_eq!(parent.get_i64("state_version").unwrap(), 1);
    assert_eq!(source_fence(&parent).unwrap(), 1);
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_str("status")
            .unwrap(),
        "active"
    );
    assert!(revocations(&f.db).await.is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_source_revoke_children_enqueues_each_exact_child_in_supplied_transaction() {
    let f = fixture("extkey_children").await;
    let (a, _) = child(&f, None).await;
    let (b, _) = child(&f, None).await;
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let children = credentials::revoke_children(
        &f.db,
        &f.parent,
        CredentialRevokedReason::ParentRevoked,
        Some(&mut session),
    )
    .await
    .unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(
        f.db.collection::<Document>(crate::models::machine_access::OUTBOX)
            .count_documents(doc! {"kind": "external"})
            .session(&mut session)
            .await
            .unwrap(),
        2
    );
    session.abort_transaction().await.unwrap();
    assert!(revocations(&f.db).await.is_empty());
    assert!(
        row(&f.db, CREDENTIALS, &a)
            .await
            .get_bool("is_active")
            .unwrap()
    );
    assert_eq!(
        credentials::revoke_children(
            &f.db,
            &f.parent,
            CredentialRevokedReason::ParentRevoked,
            None
        )
        .await
        .unwrap()
        .len(),
        2
    );
    let rows = revocations(&f.db).await;
    assert_eq!(rows.len(), 2);
    let expected = [a, b].map(|id| SourceSelector::Child {
        user_id: f.owner.clone(),
        api_key_id: f.parent.clone(),
        api_key_credential_id: id,
    });
    assert!(
        rows.iter()
            .all(|row| expected.contains(&cancellation_selector(row)))
    );
    assert_eq!(
        row(&f.db, GRANTS, &f.grant)
            .await
            .get_i64("revision")
            .unwrap(),
        1
    );
    assert!(
        credentials::revoke_children(
            &f.db,
            &f.parent,
            CredentialRevokedReason::ParentRevoked,
            None
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert_eq!(revocations(&f.db).await.len(), 2);
    f.db.drop().await.unwrap();
}
