use super::*;
use crate::services::feature_flag_service::{self as flags, FlagTarget};
use axum::{body::to_bytes, response::IntoResponse};

const LEGACY_FLEET_EXCLUSION: &str = "Billing provider unavailable: CMA paid admission is disabled";
const COHORT_EXCLUSION: &str =
    "Billing provider unavailable: Sandbox owner is outside the billing rollout";

async fn assert_excluded(error: AppError, expected_message: &str) {
    let response = error.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::PAYMENT_REQUIRED);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    // Pin the complete envelope consumed by CMA #1322 at 2d31bf36b. Neither
    // an ordinary provider outage nor a malformed refusal permits free compute.
    assert_eq!(
        body,
        serde_json::json!({
            "error": "billing_provider_unavailable",
            "error_code": 11302,
            "message": expected_message,
        })
    );
}

#[tokio::test]
async fn excluded_quotes_and_new_windows_do_not_read_paid_billing_dependencies() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let observed = Arc::new(Mutex::new(Vec::new()));
    let recorded = observed.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "cma_exclusion_before_dependencies",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                for kind in ["find", "aggregate", "insert", "update", "findAndModify"] {
                    if let Ok(collection) = event.command.get_str(kind) {
                        recorded.lock().unwrap().push(collection.to_owned());
                    }
                }
            }
        }),
    )
    .await;
    let owner = Uuid::new_v4().to_string();
    let producer = ProducerAuthority {
        id: Uuid::new_v4().to_string(),
        token_jti: Uuid::new_v4().to_string(),
        credential_generation: 0,
        service_id: Uuid::new_v4().to_string(),
        settings_revision: 1,
    };
    let mut billing = BillingService::new(
        db.clone(),
        Arc::new(AppConfig {
            billing_enabled: false,
            billing_fail_closed: true,
            ..crate::test_utils::test_app_config()
        }),
    );
    db.collection::<Document>("billing_migrations")
        .delete_many(doc! {})
        .await
        .unwrap();
    let input = ReserveSandboxWindow {
        request_id: Uuid::new_v4().to_string(),
        quote_id: Uuid::new_v4().to_string(),
        owner_id: owner.clone(),
        sandbox_id: format!("sbx_{}", Uuid::new_v4().simple()),
        runtime_generation: 1,
        operation_id: Uuid::new_v4().to_string(),
        previous_window_id: None,
        seconds: 60,
        allocation: allocation(),
    };
    // First missing, then malformed documents: neither can mask default-off
    // cohort exclusion. Also absent: Lago, catalog configuration and user wallet.
    for malformed in [false, true] {
        if malformed {
            let config = Arc::make_mut(&mut billing.config);
            config.billing_enabled = true;
            config.billing_fail_closed = false;
            for collection in [
                "billing_migrations",
                "downstream_services",
                "sandbox_rate_cards",
                WALLETS,
                "billing_rate_cache",
            ] {
                db.collection::<Document>(collection)
                    .insert_one(doc! {
                        "_id": "malformed", "owner_id": &owner, "is_active": true,
                        "effective_from": bson::DateTime::from_millis(0), "prices": false,
                    })
                    .await
                    .unwrap();
            }
        }
        observed.lock().unwrap().clear();
        assert_excluded(
            billing
                .quote_sandbox(&producer, &owner, &input.sandbox_id, allocation())
                .await
                .unwrap_err(),
            COHORT_EXCLUSION,
        )
        .await;
        assert_excluded(
            billing
                .reserve_sandbox_window(&producer, input.clone())
                .await
                .unwrap_err(),
            COHORT_EXCLUSION,
        )
        .await;
        let reads = observed.lock().unwrap().clone();
        assert!(!reads.is_empty(), "cohort resolution must actually run");
        assert!(
            reads.iter().all(|name| [
                "org_memberships",
                "feature_flag_overrides",
                "feature_flag_definitions",
                WINDOW_COLLECTION
            ]
            .contains(&name.as_str())),
            "exclusion may only read rollout and recover existing receipts: {reads:?}"
        );
    }
    for collection in [
        QUOTE_COLLECTION,
        WINDOW_COLLECTION,
        CURSOR_COLLECTION,
        USAGE,
    ] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    // Unable to resolve enrollment is a failure, never an explicit exclusion.
    db.collection::<Document>("feature_flag_overrides")
        .insert_one(doc! {
            "_id": "broken-flag", "org_user_id": null, "enabled": "invalid",
        })
        .await
        .unwrap();
    let error = billing
        .quote_sandbox(
            &producer,
            &owner,
            &format!("sbx_{}", Uuid::new_v4().simple()),
            allocation(),
        )
        .await
        .unwrap_err();
    assert!(![LEGACY_FLEET_EXCLUSION, COHORT_EXCLUSION].contains(&error.to_string().as_str()));
}

#[tokio::test]
async fn cma_quote_cohort_precedence_is_global_then_org_then_personal() {
    use crate::models::{
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
        user::User,
    };
    let f = fixture("cma_cohort_precedence").await;
    let user = FlagTarget::User(f.owner.clone());
    flags::clear_platform_override(&f.db, flags::CMA_SANDBOX_BILLING_FLAG_KEY, &user)
        .await
        .unwrap();
    let org = Uuid::new_v4().to_string();
    f.db.collection::<User>("users")
        .insert_one(crate::test_utils::test_user(&org, UserType::Org))
        .await
        .unwrap();
    f.db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            &f.owner,
            OrgRole::Viewer,
            None,
        ))
        .await
        .unwrap();
    flags::set_platform_override(
        &f.db,
        flags::CMA_SANDBOX_BILLING_FLAG_KEY,
        &FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    flags::set_platform_org_override(
        &f.db,
        &org,
        flags::CMA_SANDBOX_BILLING_FLAG_KEY,
        false,
        &f.owner,
    )
    .await
    .unwrap();
    let sandbox = format!("sbx_{}", Uuid::new_v4().simple());
    assert_excluded(
        f.billing
            .quote_sandbox(&f.producer, &f.owner, &sandbox, allocation())
            .await
            .unwrap_err(),
        COHORT_EXCLUSION,
    )
    .await;
    flags::set_platform_override(
        &f.db,
        flags::CMA_SANDBOX_BILLING_FLAG_KEY,
        &user,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let input = request(&f, 60).await;
    let admitted = f
        .billing
        .reserve_sandbox_window(&f.producer, input)
        .await
        .unwrap();
    assert_eq!(admitted.components.len(), 3);
    assert!(wallet(&f).await.reserved_credits > Credits::ZERO);

    // An organization cohort alone admits its member's personal wallet even
    // when the global baseline is off and there is no personal override.
    flags::clear_platform_override(&f.db, flags::CMA_SANDBOX_BILLING_FLAG_KEY, &user)
        .await
        .unwrap();
    flags::set_platform_override(
        &f.db,
        flags::CMA_SANDBOX_BILLING_FLAG_KEY,
        &FlagTarget::Global,
        false,
        &f.owner,
    )
    .await
    .unwrap();
    flags::set_platform_org_override(
        &f.db,
        &org,
        flags::CMA_SANDBOX_BILLING_FLAG_KEY,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let held = wallet(&f).await.reserved_credits;
    let input = request(&f, 60).await;
    let admitted = f
        .billing
        .reserve_sandbox_window(&f.producer, input)
        .await
        .unwrap();
    assert_eq!(admitted.owner_id, f.owner);
    assert_eq!(admitted.components.len(), 3);
    assert!(wallet(&f).await.reserved_credits > held);
}

#[tokio::test]
async fn enrolled_billing_failures_never_use_the_unbilled_exclusion_contract() {
    let mut f = fixture("cma_enrolled_fail_closed").await;
    let sandbox = format!("sbx_{}", Uuid::new_v4().simple());
    for (enabled, fail_closed) in [(false, false), (true, true)] {
        let config = Arc::make_mut(&mut f.billing.config);
        config.billing_enabled = enabled;
        config.billing_fail_closed = fail_closed;
        let error = f
            .billing
            .quote_sandbox(&f.producer, &f.owner, &sandbox, allocation())
            .await
            .unwrap_err();
        assert!(matches!(&error, AppError::BillingProviderUnavailable(_)));
        assert!(![LEGACY_FLEET_EXCLUSION, COHORT_EXCLUSION].contains(&error.to_string().as_str()));
    }
    let config = Arc::make_mut(&mut f.billing.config);
    config.billing_enabled = true;
    config.billing_fail_closed = false;
    flags::set_platform_override(
        &f.db,
        flags::BILLING_FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        false,
        &f.owner,
    )
    .await
    .unwrap();
    let error = f
        .billing
        .quote_sandbox(&f.producer, &f.owner, &sandbox, allocation())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Billing provider unavailable: Sandbox owner is not eligible for paid billing"
    );
    flags::set_platform_override(
        &f.db,
        flags::BILLING_FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    f.provider.available.store(false, Ordering::SeqCst);
    let error = f
        .billing
        .quote_sandbox(&f.producer, &f.owner, &sandbox, allocation())
        .await
        .unwrap_err();
    assert!(![LEGACY_FLEET_EXCLUSION, COHORT_EXCLUSION].contains(&error.to_string().as_str()));
    for collection in [QUOTE_COLLECTION, WINDOW_COLLECTION, USAGE] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
}
