use super::super::audit::{Event, Mutation};
use super::*;
use crate::{
    models::audit_log::AuditLog,
    services::audit_service::{
        AuditActor,
        required::{self, TextField},
    },
};

async fn parity(action: Action) {
    let f = fixture().await;
    assert!(request(&f, action).await.status().is_success());
    let id = if matches!(action, Action::Create) {
        &f.created
    } else {
        &f.sa.id
    };
    let sa = get_service_account(&f.state.db, id).await.unwrap();
    // The exact old handler metadata; production mutation and serialization are not duplicated.
    let data = match action {
        Action::Create => {
            serde_json::json!({"target_sa_id":&sa.id,"client_id":&sa.client_id,"name":&sa.name})
        }
        Action::Rotate => serde_json::json!({"target_sa_id":&sa.id,"client_id":&sa.client_id}),
        Action::Revoke => serde_json::json!({"target_sa_id":&sa.id,"revoked_count":2}),
        Action::Update | Action::Delete => serde_json::json!({"target_sa_id":&sa.id}),
    };
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
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
        .find_one(doc! {"_id":{"$ne":&old_id},"event_type":action.event()})
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
        &sa.client_secret_hash,
        &sa.secret_prefix,
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
parity_test!(sa_created_metadata_matches_legacy, Create);
parity_test!(sa_updated_metadata_matches_legacy, Update);
parity_test!(sa_deleted_metadata_matches_legacy, Delete);
parity_test!(sa_rotated_metadata_matches_legacy, Rotate);
parity_test!(sa_revoked_metadata_matches_legacy, Revoke);

#[test]
fn sa_occurrence_maxima_and_caps() {
    let raw = "x".repeat(64 * 1024);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    for event in [
        Event::Created {
            client_id: &raw,
            name: &raw,
        },
        Event::Updated,
        Event::Deleted,
        Event::Rotated { client_id: &raw },
        Event::TokensRevoked {
            count: i64::MAX as u64,
        },
    ] {
        let intent = event.intent(&actor, &raw).unwrap();
        let crate::models::security_effect_intent::SecurityEffect::Audit { event } = intent.effect
        else {
            panic!("audit")
        };
        let size = bson::to_vec(&event).unwrap().len();
        println!(
            "SA_MAX {} bytes={size} margin={}",
            event.event_type,
            16 * 1024 - size
        );
        assert!(size < 16 * 1024);
        let data = event.event_data.as_ref().unwrap();
        assert_eq!(
            data["target_sa_id"],
            required::text(TextField::Identifier, &raw)
        );
        for (name, kind) in [
            ("client_id", TextField::Identifier),
            ("name", TextField::DisplayName),
        ] {
            if let Some(value) = data.get(name) {
                assert_eq!(value, &required::text(kind, &raw));
            }
        }
        assert_eq!(
            event.user_agent.as_deref(),
            Some(required::text(TextField::UserAgent, &raw).as_str())
        );
        assert_eq!(
            event.ip_address.as_deref(),
            Some(required::text(TextField::ForwardedIp, &raw).as_str())
        );
    }
}

#[tokio::test]
async fn sa_failed_management_fence_keeps_existing_errors_and_no_intent() {
    let f = fixture().await;
    let db = &f.state.db;
    db.collection::<bson::Document>(SERVICE_ACCOUNTS)
        .update_one(
            doc! {"_id":&f.sa.id},
            doc! {"$set":{"platform_protected":true}},
        )
        .await
        .unwrap();
    let actor = AuditActor::from_auth_user(&auth(&f));
    let before = rows(db, SERVICE_ACCOUNTS).await;
    for action in [
        Action::Update,
        Action::Delete,
        Action::Rotate,
        Action::Revoke,
    ] {
        let error = match action {
            Action::Update => update_service_account(
                db,
                &f.sa.id,
                Some("Should not change"),
                None,
                None,
                None,
                None,
                None,
                false,
                None,
                Some(&actor),
            )
            .await
            .err()
            .unwrap(),
            Action::Delete => delete_service_account(db, &f.sa.id, false, Some(&actor))
                .await
                .unwrap_err(),
            Action::Rotate => rotate_secret(db, &f.sa.id, false, Some(&actor))
                .await
                .err()
                .unwrap(),
            Action::Revoke => revoke_all_tokens(db, &f.sa.id, false, Some(&actor))
                .await
                .unwrap_err(),
            Action::Create => unreachable!(),
        };
        assert!(
            matches!(error, AppError::Forbidden(_)),
            "{action:?}: {error}"
        );
        assert_eq!(rows(db, SERVICE_ACCOUNTS).await, before);
        assert!(rows(db, INTENTS).await.is_empty());
    }
    let expected = ExpectedAccessState {
        role_ids: vec![],
        allowed_scopes: "changed elsewhere".into(),
        purpose: f.sa.purpose,
        platform_protected: true,
        is_active: true,
        catalog_scope_authorized: false,
    };
    let error = update_service_account(
        db,
        &f.sa.id,
        None,
        None,
        Some("proxy"),
        None,
        None,
        None,
        true,
        Some(&expected),
        Some(&actor),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, AppError::Conflict(_)));
    assert_eq!(rows(db, SERVICE_ACCOUNTS).await, before);
    assert!(rows(db, INTENTS).await.is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn sa_unmodified_source_writes_no_occurrence() {
    let f = fixture().await;
    let actor = AuditActor::from_auth_user(&auth(&f));
    let intent = Event::Updated.intent(&actor, &f.sa.id).unwrap();
    let source = Mutation {
        db: f.state.db.clone(),
        filter: management_filter(&f.sa.id, true),
        update: doc! {"$set":{"name":&f.sa.name}},
        expected_access: false,
    };
    source.apply(Some(&intent)).await.unwrap();
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn sa_cleanup_failure_retains_effect_and_audit_but_revoke_is_atomic() {
    for action in [Action::Delete, Action::Rotate, Action::Revoke] {
        let f = fixture().await;
        let db = &f.state.db;
        let before = rows(db, SERVICE_ACCOUNTS).await;
        db.collection::<bson::Document>(SA_TOKENS)
            .drop()
            .await
            .unwrap();
        db.run_command(doc! {"create":SA_TOKENS,"viewOn":"read_only_token_fixture","pipeline":[]})
            .await
            .unwrap();
        assert_eq!(
            request(&f, action).await.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        if matches!(action, Action::Revoke) {
            assert_eq!(rows(db, SERVICE_ACCOUNTS).await, before);
            assert!(rows(db, INTENTS).await.is_empty());
        } else {
            let sa = get_service_account(db, &f.sa.id).await.unwrap();
            assert_eq!(sa.credential_generation, 1);
            assert_eq!(rows(db, INTENTS).await.len(), 1);
            if matches!(action, Action::Delete) {
                assert!(!sa.is_active);
            } else {
                assert_ne!(sa.client_secret_hash, f.sa.client_secret_hash);
            }
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn sa_empty_revoke_still_audits_effective_generation_change() {
    let f = fixture().await;
    let db = &f.state.db;
    db.collection::<bson::Document>(SA_TOKENS)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(request(&f, Action::Revoke).await.status().is_success());
    assert_eq!(
        get_service_account(db, &f.sa.id)
            .await
            .unwrap()
            .credential_generation,
        1
    );
    let intent = rows(db, INTENTS).await.pop().unwrap();
    assert!(intent.to_string().contains("revoked_count"));
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    let event = db
        .collection::<AuditLog>("audit_log")
        .find_one(doc! {"event_type":Action::Revoke.event()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.event_data.unwrap()["revoked_count"], 0);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn sa_oversized_actor_strings_do_not_reject_rotation() {
    let f = fixture().await;
    let raw = "界".repeat(17 * 1024);
    let mut auth = auth(&f);
    auth.ip_address = Some(raw.clone());
    auth.user_agent = Some(raw.clone());
    auth.api_key_name = Some(raw.clone());
    let response = handlers::rotate_secret(
        State(f.state.clone()),
        auth,
        crate::telemetry::TelemetryContext::default(),
        HeaderMap::new(),
        Path(f.sa.id.clone()),
    )
    .await;
    assert!(response.is_ok());
    assert_eq!(
        get_service_account(&f.state.db, &f.sa.id)
            .await
            .unwrap()
            .credential_generation,
        1
    );
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let event = f
        .state
        .db
        .collection::<AuditLog>("audit_log")
        .find_one(doc! {"event_type":Action::Rotate.event()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event.ip_address,
        Some(required::text(TextField::ForwardedIp, &raw))
    );
    assert_eq!(
        event.user_agent,
        Some(required::text(TextField::UserAgent, &raw))
    );
    assert_eq!(
        event.api_key_name,
        Some(required::text(TextField::KeyName, &raw))
    );
    assert!(event.user_agent.unwrap().ends_with("...[truncated]"));
    f.state.db.drop().await.unwrap();
}
