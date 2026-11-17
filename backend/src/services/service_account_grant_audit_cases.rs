use super::*;
use crate::{
    models::audit_log::AuditLog,
    services::{
        audit_service::{
            AuditActor,
            required::{self, TextField},
        },
        service_account_grant_audit as events,
    },
};

async fn parity(action: Action) {
    let f = fixture().await;
    let previous = reads::get_grant(&f.state.db, &f.sa.id)
        .await
        .unwrap()
        .unwrap();
    let response = request(&f, action).await;
    assert!(response.status().is_success());
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let sa = accounts::get_service_account(&f.state.db, &f.sa.id)
        .await
        .unwrap();
    let grant = reads::get_grant(&f.state.db, &f.sa.id).await.unwrap();
    // Exact former handler expressions; mutations use the real handlers above.
    let data = match action {
        Action::CurationIssue => {
            serde_json::json!({"target_sa_id":f.sa.id,"grant_id":sa.curation_grant.as_ref().map(|g|&g.id)})
        }
        Action::CurationRevoke => serde_json::json!({"target_sa_id":f.sa.id}),
        Action::KeyIssue => {
            let g = grant.unwrap();
            let expiry: Option<DateTime<Utc>> =
                serde_json::from_value(response["expires_at"].clone()).unwrap();
            serde_json::json!({"target_sa_id":f.sa.id,"owner_id":g.owner_id,"targets":g.targets,"expires_at":expiry})
        }
        Action::KeyRevoke => {
            serde_json::json!({"target_sa_id":f.sa.id,"previous_grant":Some(key_reads::GrantResponse::from(previous))})
        }
    };
    let written = audit_service::notify_on_audit_write_for_user(action.event(), &f.actor);
    audit_service::log_for_user(f.state.db.clone(), &auth(&f), action.event(), Some(data));
    let old_id = tokio::time::timeout(std::time::Duration::from_secs(5), written)
        .await
        .unwrap()
        .unwrap();
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let logs = f.state.db.collection::<AuditLog>("audit_log");
    let old = logs.find_one(doc! {"_id":&old_id}).await.unwrap().unwrap();
    let new = logs
        .find_one(doc! {"_id":{"$ne":old_id},"event_type":action.event()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new.event_type, old.event_type);
    assert_eq!(new.event_data, old.event_data);
    assert_eq!(new.user_id, old.user_id);
    assert_eq!(new.ip_address, old.ip_address);
    assert_eq!(new.user_agent, old.user_agent);
    assert_eq!(new.api_key_id, old.api_key_id);
    assert_eq!(new.api_key_name, old.api_key_name);
    let data = new.event_data.unwrap().to_string();
    for secret in [
        &f.secret,
        &f.sa.client_secret_hash,
        &f.sa.secret_prefix,
        "do-not-copy",
    ] {
        assert!(!data.contains(secret));
    }
    f.state.db.drop().await.unwrap();
}
macro_rules! parity_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            parity(Action::$action).await;
        }
    };
}
parity_test!(curation_granted_metadata_matches_legacy, CurationIssue);
parity_test!(curation_revoked_metadata_matches_legacy, CurationRevoke);
parity_test!(key_read_granted_metadata_matches_legacy, KeyIssue);
parity_test!(key_read_revoked_metadata_matches_legacy, KeyRevoke);

#[tokio::test]
async fn repeated_revocations_keep_response_without_new_occurrence() {
    for action in [Action::CurationRevoke, Action::KeyRevoke] {
        let f = fixture().await;
        assert!(request(&f, action).await.status().is_success());
        assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
        let second = request(&f, action).await;
        assert!(second.status().is_success());
        assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
        let sa = accounts::get_service_account(&f.state.db, &f.sa.id)
            .await
            .unwrap();
        assert_eq!(sa.purpose, ServiceAccountPurpose::Curation);
        assert!(sa.platform_protected);
        if matches!(action, Action::CurationRevoke) {
            let body = axum::body::to_bytes(second.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["updated_at"], sa.updated_at.to_rfc3339());
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn prepared_curation_writes_keep_exact_fences_and_errors() {
    let f = fixture().await;
    let db = &f.state.db;
    let actor = AuditActor::from_auth_user(&auth(&f));
    let prepared = prepare_issue(db, &f.sa.id, &f.actor, input(&f), Some(&actor))
        .await
        .unwrap();
    db.collection::<ServiceAccount>(ACCOUNTS)
        .update_one(
            doc! {"_id":&f.sa.id},
            doc! {"$set":{"allowed_scopes":"user-services:read"}},
        )
        .await
        .unwrap();
    let before = rows(db, ACCOUNTS).await;
    assert!(matches!(prepared.apply().await, Err(AppError::Conflict(_))));
    assert_eq!(rows(db, ACCOUNTS).await, before);
    assert!(rows(db, INTENTS).await.is_empty());
    let prepared = prepare_revoke(db, &f.sa.id, Some(&actor)).await.unwrap();
    db.collection::<ServiceAccount>(ACCOUNTS)
        .delete_one(doc! {"_id":&f.sa.id})
        .await
        .unwrap();
    assert!(matches!(
        prepared.apply().await,
        Err(AppError::ServiceAccountNotFound(_))
    ));
    assert!(rows(db, INTENTS).await.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn unchanged_key_read_replacement_has_no_occurrence() {
    let f = fixture().await;
    let db = &f.state.db;
    let actor = AuditActor::from_auth_user(&auth(&f));
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    reads::publish_grant_in_session(db, &mut tx, &f.previous, Some(&actor))
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    assert!(rows(db, INTENTS).await.is_empty());
    assert_eq!(
        bson::to_document(&reads::get_grant(db, &f.sa.id).await.unwrap().unwrap()).unwrap(),
        bson::to_document(&f.previous).unwrap()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn competing_revokes_record_each_grant_once() {
    for action in [Action::CurationRevoke, Action::KeyRevoke] {
        let f = fixture().await;
        let (a, b) = tokio::join!(request(&f, action), request(&f, action));
        assert!(a.status().is_success());
        assert!(b.status().is_success());
        assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
        security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
            .await
            .unwrap();
        assert_eq!(rows(&f.state.db, "audit_log").await.len(), 1);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn oversized_actor_strings_do_not_refuse_grant() {
    let f = fixture().await;
    let mut actor = auth(&f);
    let raw = "🦀".repeat(20_000);
    actor.ip_address = Some(raw.clone());
    actor.user_agent = Some(raw.clone());
    actor.api_key_name = Some(raw.clone());
    let _ = curation::issue_curation_grant(
        State(f.state.clone()),
        actor,
        Path(f.sa.id.clone()),
        Json(input(&f)),
    )
    .await
    .unwrap();
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let row = f
        .state
        .db
        .collection::<AuditLog>("audit_log")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    for (value, field) in [
        (row.ip_address, TextField::ForwardedIp),
        (row.user_agent, TextField::UserAgent),
        (row.api_key_name, TextField::KeyName),
    ] {
        let value = value.unwrap();
        assert_eq!(value, required::text(field, &raw));
        assert!(value.ends_with("...[truncated]"));
    }
    assert_eq!(
        accounts::get_service_account(&f.state.db, &f.sa.id)
            .await
            .unwrap()
            .curation_grant
            .unwrap()
            .max_writes,
        100
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_key_read_targets_commit_while_audit_prefix_and_digest_are_bounded() {
    use sha2::{Digest, Sha256};
    let f = fixture().await;
    let db = &f.state.db;
    let mut ids = vec![f.connection.clone()];
    let existing = db
        .collection::<bson::Document>("user_services")
        .find_one(doc! {"_id":&f.connection})
        .await
        .unwrap()
        .unwrap();
    for _ in 1..100 {
        let id = Uuid::new_v4().to_string();
        let mut row = existing.clone();
        row.insert("_id", &id);
        db.collection::<bson::Document>("user_services")
            .insert_one(row)
            .await
            .unwrap();
        ids.push(id);
    }
    let _ = key_reads::issue_grant(
        State(f.state.clone()),
        auth(&f),
        Path(f.sa.id.clone()),
        Json(key_reads::GrantRequest {
            user_service_ids: ids.clone(),
            expires_at: None,
        }),
    )
    .await
    .unwrap();
    let grant = reads::get_grant(db, &f.sa.id).await.unwrap().unwrap();
    assert_eq!(
        grant
            .targets
            .iter()
            .map(|t| t.user_service_id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    let row = db
        .collection::<AuditLog>("audit_log")
        .find_one(doc! {"event_type":"admin.sa.key_read_granted"})
        .await
        .unwrap()
        .unwrap();
    let data = row.event_data.unwrap();
    let kept = data["targets"].as_array().unwrap();
    assert!(!kept.is_empty() && kept.len() < 100);
    assert_eq!(data["targets_count"], 100);
    assert_eq!(data["targets_truncated"], true);
    let complete = grant
        .targets
        .iter()
        .flat_map(|t| [t.user_service_id.as_str(), t.owner_id.as_str()])
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        data["targets_sha256"],
        hex::encode(Sha256::digest(complete.as_bytes()))
    );
    assert_eq!(
        *kept,
        serde_json::to_value(&grant.targets[..kept.len()])
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    );
    let array_bytes = bson::to_vec(&doc! {"targets":bson::to_bson(kept).unwrap()})
        .unwrap()
        .len()
        - 14;
    assert!(array_bytes <= 8192);
    db.drop().await.unwrap();
}

#[test]
fn grant_occurrence_maxima_and_caps() {
    let raw = "x".repeat(64 * 1024);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    let grant = ServiceAccountKeyReadGrant {
        service_account_id: raw.clone(),
        owner_id: raw.clone(),
        issued_by: raw.clone(),
        issued_at: DateTime::<Utc>::MAX_UTC,
        expires_at: Some(DateTime::<Utc>::MAX_UTC),
        targets: (0..100)
            .map(|_| KeyReadTarget {
                user_service_id: raw.clone(),
                owner_id: raw.clone(),
            })
            .collect(),
    };
    let mut pairs = grant.clone();
    pairs.targets[14].owner_id = "y".repeat(83);
    let mut ids = grant.clone();
    ids.targets[30].user_service_id = "z".repeat(238);
    for intent in [
        events::curation(&actor, &raw, Some(&raw)).unwrap(),
        events::curation(&actor, &raw, None).unwrap(),
        events::key_granted(&actor, &pairs).unwrap(),
        events::key_revoked(&actor, &raw, &ids).unwrap(),
    ] {
        let crate::models::security_effect_intent::SecurityEffect::Audit { event } = intent.effect
        else {
            panic!("audit")
        };
        let size = bson::to_vec(&event).unwrap().len();
        assert!(size < 16 * 1024);
        println!(
            "GRANT_MAX {} bytes={size} margin={}",
            event.event_type,
            16384 - size
        );
        let data = event.event_data.as_ref().unwrap();
        assert_eq!(
            data["target_sa_id"],
            required::text(TextField::Identifier, &raw)
        );
        let list = if event.event_type == "admin.sa.key_read_granted" {
            Some(&data["targets"])
        } else if event.event_type == "admin.sa.key_read_revoked" {
            Some(&data["previous_grant"]["user_service_ids"])
        } else {
            None
        };
        if let Some(list) = list {
            assert_eq!(
                bson::to_vec(&doc! {"x":bson::to_bson(list).unwrap()})
                    .unwrap()
                    .len()
                    - 8,
                8192
            );
        }
    }
}
