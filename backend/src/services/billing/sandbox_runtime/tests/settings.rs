use super::*;
use crate::{
    models::sandbox_billing_settings::COLLECTION_NAME as SETTINGS,
    services::{audit_service::AuditActor, billing::sandbox_settings},
};
use futures::TryStreamExt;

async fn set_producer(
    f: &Fixture,
    producer: Option<&str>,
    revision: i64,
) -> AppResult<crate::models::sandbox_billing_settings::SandboxBillingSettings> {
    sandbox_settings::update(
        &f.db,
        &f.producer.service_id,
        producer,
        revision,
        &AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&f.owner)),
        super::super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY.as_slice(),
    )
    .await
}

#[tokio::test]
async fn producer_settings_are_audited_protected_and_revision_fenced() {
    let f = fixture("sandbox_settings_revision").await;
    let input = request(&f, 60).await;
    let cleared = set_producer(&f, None, 1).await.unwrap();
    assert_eq!(cleared.revision, 2);
    assert!(
        authorize_producer(&f.db, &f.producer.id, WRITE_SCOPE, &f.producer.token_jti)
            .await
            .is_err()
    );
    assert!(matches!(
        f.billing.reserve_sandbox_window(&f.producer, input).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert!(matches!(
        set_producer(&f, Some(&f.producer.id), 1).await,
        Err(AppError::Conflict(_))
    ));
    let saved = set_producer(&f, Some(&f.producer.id), 2).await.unwrap();
    assert_eq!(saved.revision, 3);
    let account =
        f.db.collection::<Document>("service_accounts")
            .find_one(doc! {"_id": &f.producer.id})
            .await
            .unwrap()
            .unwrap();
    assert!(account.get_bool("platform_protected").unwrap());
    let audits: Vec<Document> =
        f.db.collection::<Document>("audit_logs")
            .find(doc! {"event_type":"sandbox_billing_producer_updated"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(audits.len(), 2);
    assert!(
        audits
            .iter()
            .all(|a| a.contains_key("seq") && a.contains_key("entry_hash"))
    );
    let fresh = authorize_producer(&f.db, &f.producer.id, WRITE_SCOPE, &f.producer.token_jti)
        .await
        .unwrap();
    assert_eq!(fresh.settings_revision, 3);
    let quote = f
        .billing
        .quote_sandbox(
            &fresh,
            &f.owner,
            &format!("sbx_{}", Uuid::new_v4().simple()),
            allocation(),
        )
        .await
        .unwrap();
    assert_eq!(quote.service_id, saved.service_id);
}

#[tokio::test]
async fn producer_changes_preserve_unreported_windows_and_existing_fences() {
    let f = fixture("sandbox_settings_liability").await;
    let input = request(&f, 2).await;
    let window = f
        .billing
        .reserve_sandbox_window(&f.producer, input)
        .await
        .unwrap();
    assert!(matches!(
        set_producer(&f, None, 1).await,
        Err(AppError::Conflict(_))
    ));
    for update in [
        doc! {"purpose":"curation"},
        doc! {"allowed_scopes":"proxy"},
        doc! {"credential_generation":1_i64},
        doc! {"is_active":false},
    ] {
        f.db.collection::<Document>("service_accounts")
            .update_one(doc! {"_id":&f.producer.id}, doc! {"$set":update})
            .await
            .unwrap();
        let past = elapsed(&f, window.clone()).await;
        assert!(matches!(
            f.billing
                .finalize_sandbox_window(&f.producer, evidence(&past))
                .await,
            Err(AppError::Forbidden(_))
        ));
        f.db.collection::<Document>("service_accounts").update_one(doc! {"_id":&f.producer.id}, doc! {"$set":{"purpose":"general","allowed_scopes":WRITE_SCOPE,"credential_generation":0_i64,"is_active":true}}).await.unwrap();
    }
    let past = elapsed(&f, window).await;
    f.billing
        .finalize_sandbox_window(&f.producer, evidence(&past))
        .await
        .unwrap();
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    set_producer(&f, None, 1).await.unwrap();
}

#[tokio::test]
async fn producer_settings_fail_closed_and_audit_failure_rolls_back() {
    let f = fixture("sandbox_settings_audit").await;
    f.db.collection::<Document>(SETTINGS)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(
        authorize_producer(&f.db, &f.producer.id, WRITE_SCOPE, &f.producer.token_jti)
            .await
            .is_err()
    );
    for update in [
        doc! {"purpose":"catalog_editor"},
        doc! {"allowed_scopes":"proxy"},
        doc! {"is_active":false},
    ] {
        f.db.collection::<Document>("service_accounts")
            .update_one(doc! {"_id":&f.producer.id}, doc! {"$set":update})
            .await
            .unwrap();
        assert!(matches!(
            set_producer(&f, Some(&f.producer.id), 0).await,
            Err(AppError::ValidationError(_))
        ));
        f.db.collection::<Document>("service_accounts")
            .update_one(
                doc! {"_id":&f.producer.id},
                doc! {"$set":{"purpose":"general","allowed_scopes":WRITE_SCOPE,"is_active":true}},
            )
            .await
            .unwrap();
    }
    f.db.run_command(doc! {"collMod":"audit_logs","validator":{"event_type":{"$ne":"sandbox_billing_producer_updated"}}}).await.unwrap();
    assert!(set_producer(&f, Some(&f.producer.id), 0).await.is_err());
    assert!(
        sandbox_settings::load(&f.db, &f.producer.service_id)
            .await
            .unwrap()
            .is_none()
    );
    let account =
        f.db.collection::<Document>("service_accounts")
            .find_one(doc! {"_id": &f.producer.id})
            .await
            .unwrap()
            .unwrap();
    assert!(!account.get_bool("platform_protected").unwrap_or(false));
}

#[tokio::test]
async fn catalog_resolution_rejects_missing_inactive_or_ambiguous_cma() {
    let f = fixture("sandbox_catalog_resolution").await;
    let service_id = f.producer.service_id.clone();
    f.db.collection::<Document>("downstream_services")
        .update_one(doc! {"_id":&service_id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(sandbox_settings::resolve_service(&f.db).await.is_err());
    f.db.collection::<Document>("downstream_services")
        .update_one(doc! {"_id":&service_id}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    let mut duplicate = sandbox_settings::resolve_service(&f.db).await.unwrap();
    duplicate.id = Uuid::new_v4().to_string();
    f.db.collection::<Document>("downstream_services")
        .drop_indexes()
        .await
        .unwrap();
    f.db.collection::<DownstreamService>("downstream_services")
        .insert_one(duplicate)
        .await
        .unwrap();
    assert!(sandbox_settings::resolve_service(&f.db).await.is_err());
    f.db.collection::<crate::models::sandbox_billing_settings::SandboxBillingSettings>(SETTINGS)
        .insert_one(
            crate::models::sandbox_billing_settings::SandboxBillingSettings {
                service_id: Uuid::new_v4().to_string(),
                authorized_producer_service_account_id: Some(f.producer.id.clone()),
                revision: 1,
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    assert!(
        authorize_producer(&f.db, &f.producer.id, WRITE_SCOPE, &f.producer.token_jti)
            .await
            .is_err()
    );
    f.db.collection::<Document>("downstream_services")
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(sandbox_settings::resolve_service(&f.db).await.is_err());
}

#[tokio::test]
async fn producer_setting_change_and_window_admission_cannot_both_commit() {
    let f = fixture("sandbox_settings_admission_race").await;
    let input = request(&f, 60).await;
    let (changed, admitted) = tokio::join!(
        set_producer(&f, None, 1),
        f.billing.reserve_sandbox_window(&f.producer, input),
    );
    assert!(!(changed.is_ok() && admitted.is_ok()));
    let saved = sandbox_settings::load(&f.db, &f.producer.service_id)
        .await
        .unwrap()
        .unwrap();
    let windows =
        f.db.collection::<Document>(WINDOW_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap();
    if changed.is_ok() {
        assert!(saved.authorized_producer_service_account_id.is_none());
        assert_eq!(windows, 0);
        assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    } else if admitted.is_ok() {
        assert_eq!(
            saved.authorized_producer_service_account_id.as_deref(),
            Some(f.producer.id.as_str())
        );
        assert_eq!(windows, 1);
        assert!(wallet(&f).await.reserved_credits > Credits::ZERO);
    } else {
        // MongoDB may abort both conflicting transactions; neither may leave
        // a changed authority or partially funded window behind.
        assert_eq!(saved.revision, 1);
        assert_eq!(windows, 0);
        assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    }
}
