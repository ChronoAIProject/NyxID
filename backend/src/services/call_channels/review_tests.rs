//! M34 review regressions use real reservations and the replica-set recovery paths.
use super::*;
use crate::models::{
    billing_rate_cache::BillingRateCache,
    billing_wallet::{BillingWallet, CollectionState, PlanKind},
    credits::Credits,
    service_billing::BillingMetric,
    usage_meter::{BillingLayer, CredentialClass, UsageMeterRow},
};
use crate::services::billing::{
    BillingIngress, BillingRouteContext, NodeIntent, meter,
    reservation::{BillingReservation, LayerReservation},
};

async fn reserve_voice(f: &Fixture, s: &CallSession) {
    let now = Utc::now();
    let owner = &f.b.user_id;
    f.state
        .db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"transaction_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let mut ctx = BillingRouteContext::new(
        BillingIngress::ChannelOutbound,
        s.billing_request_id.clone(),
        owner.clone(),
        owner.clone(),
        None,
        None,
        Some(Uuid::new_v4().to_string()),
        Some("twilio".into()),
        NodeIntent::Direct,
        "basic".into(),
        CredentialClass::UserOwned,
        BillingMetric::VoiceSeconds,
        None::<&crate::models::service_billing::ServiceBilling>,
        false,
    )
    .with_platform_metering(true);
    ctx.requested_voice_seconds = 1800;
    ctx.voice_initial_window = true;
    f.state
        .db
        .collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .insert_one(BillingRateCache {
            id: BillingRateCache::cache_id(&ctx.platform_lago_metric_code, None),
            lago_metric_code: ctx.platform_lago_metric_code.clone(),
            model: None,
            credits_per_unit_micros: 2_000_000,
            credits_per_unit_pico: None,
            synced_at: now,
            retired_at: None,
        })
        .await
        .unwrap();
    let wallet = BillingWallet {
        id: Uuid::new_v4().to_string(),
        owner_id: owner.clone(),
        lago_customer_id: owner.clone(),
        lago_wallet_id: Some(Uuid::new_v4().to_string()),
        lago_subscription_id: Some(Uuid::new_v4().to_string()),
        plan_kind: PlanKind::Prepaid,
        snapshot_generation: 0,
        tax_generation: 0,
        tax_guard_token: None,
        balance_credits: Credits::from_whole(5000),
        reserved_credits: Credits::ZERO,
        pending_lago_debits: Credits::ZERO,
        pending_topup_expiry_credits: Credits::ZERO,
        has_payment_instrument: false,
        overdraft_cap_credits: Credits::ZERO,
        suspended: false,
        collection_state: CollectionState::Good,
        topup_expiry_checked_at: None,
        active_topup_expiry: None,
        balance_synced_at: now,
        created_at: now,
        updated_at: now,
    };
    f.state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .insert_one(&wallet)
        .await
        .unwrap();
    let hold = Credits::from_whole(3600);
    crate::services::billing::reservation::try_reserve_prepaid(&f.state.db, owner, hold)
        .await
        .unwrap()
        .unwrap();
    let reservation = BillingReservation {
        owner_id: owner.clone(),
        wallet_id: wallet.id.clone(),
        total_reserved_credits: hold,
        layers: vec![LayerReservation {
            metric: BillingMetric::VoiceSeconds,
            lago_metric_code: ctx.platform_lago_metric_code.clone(),
            layer: BillingLayer::Platform,
            estimated_quantity: 1800,
            credits_per_unit_micros: 2_000_000,
            credits_per_unit_pico: None,
            reserved_credits: hold,
            allowance_reservations: vec![],
            grant_reservations: vec![],
            tax_cache_ttl_secs: 900,
            applied_tax: Some(crate::services::billing::taxes::zero_snapshot()),
        }],
    };
    let meter = meter::open(&f.state.db, &ctx, Some(&reservation))
        .await
        .unwrap();
    meter::mark_forwarded(&f.state.db, &meter).await.unwrap();
}

async fn row(f: &Fixture, s: &CallSession) -> UsageMeterRow {
    f.state
        .db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .find_one(doc! {"billing_request_id":&s.billing_request_id})
        .await
        .unwrap()
        .unwrap()
}
async fn pending_wallet(f: &Fixture) -> BillingWallet {
    f.state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .find_one(doc! {"owner_id":&f.b.user_id})
        .await
        .unwrap()
        .unwrap()
}
async fn age_evidence_window(f: &Fixture, s: &CallSession, hours: i64) {
    f.state.db.collection::<bson::Document>(SESSIONS).update_one(doc! {"_id":&s.id},
        doc! {"$set":{"termination_observed_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(hours))}}).await.unwrap();
}
async fn assert_byok_waiver_and_cleanup(deleted_credential: bool) {
    let f = fixture(Profile::TelnyxRelay, false).await;
    let s = active_session(&f, "v2:missing-byok-evidence==").await;
    reserve_voice(&f, &s).await;
    let server = wiremock::MockServer::start().await;
    profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(f.b.account_id.clone(), server.uri());
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(404))
        .mount(&server)
        .await;
    // An old owner-authored terminal time cannot age the server-owned window.
    let pending = session::terminal(
        &f.state,
        &s,
        Utc::now() - Duration::days(30),
        None,
        "transport_terminal",
    )
    .await
    .unwrap();
    assert!(pending.termination_observed_at.unwrap() > Utc::now() - Duration::minutes(1));
    if deleted_credential {
        let CredentialBinding::Connection { user_service_id } = &f.b.credential else {
            panic!("BYOK required")
        };
        let connection = f
            .state
            .db
            .collection::<crate::models::user_service::UserService>(
                crate::models::user_service::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":user_service_id})
            .await
            .unwrap()
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
            .delete_one(doc! {"_id":connection.api_key_id.unwrap()})
            .await
            .unwrap();
    }
    assert!(
        coordinator::close(&f.state, &f.b, &pending, false)
            .await
            .is_err()
    );
    assert!(!row(&f, &s).await.released);
    assert_eq!(
        pending_wallet(&f).await.reserved_credits,
        Credits::from_whole(3600)
    );
    assert!(
        super::super::management::ensure_idle(&f.state, &f.b.id)
            .await
            .is_err()
    );
    assert!(
        crate::test_utils::accept_and_process_erasure_with_state(
            &f.state,
            &Uuid::new_v4().to_string(),
            &f.b.user_id,
            None
        )
        .await
        .is_err()
    );
    age_evidence_window(&f, &s, 73).await;
    coordinator::recover_once(&f.state).await.unwrap();
    let saved = session::load(&f.state, &s.id).await.unwrap();
    assert!(saved.billing_finalized);
    assert!(!saved.terminated_quantity_pending);
    assert_eq!(
        saved.terminal_evidence.as_deref(),
        Some("provider_evidence_unavailable")
    );
    assert_eq!(
        saved.provider_seconds, None,
        "fee waiver never invents provider quantity"
    );
    assert_eq!(saved.billable_seconds, Some(0));
    assert!(row(&f, &s).await.released);
    assert_eq!(row(&f, &s).await.quantity, Some(0));
    let wallet = pending_wallet(&f).await;
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
    // A late callback/provider result cannot undo a committed fee waiver.
    let replay = session::terminal(&f.state, &s, Utc::now(), Some(77), "late_provider")
        .await
        .unwrap();
    assert_eq!(replay.provider_seconds, None);
    coordinator::finalize_call(&f.state, &replay).await.unwrap();
    coordinator::recover_once(&f.state).await.unwrap();
    let audits = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"channel_adapter_provider_evidence_unavailable"})
            .await
            .unwrap(),
        1
    );
    let audit = audits
        .find_one(doc! {"event_type":"channel_adapter_provider_evidence_unavailable"})
        .await
        .unwrap()
        .unwrap();
    assert!(audit.contains_key("entry_hash"));
    let metadata = audit.get_document("event_data").unwrap();
    assert_eq!(metadata.len(), 3);
    assert_eq!(metadata.get_str("session_id").unwrap(), s.id);
    assert!(!format!("{metadata:?}").contains(&s.from));
    super::super::management::ensure_idle(&f.state, &f.b.id)
        .await
        .unwrap();
    super::super::management::disable(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    super::super::management::remove(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    crate::test_utils::accept_and_process_erasure_with_state(
        &f.state,
        &Uuid::new_v4().to_string(),
        &f.b.user_id,
        None,
    )
    .await
    .unwrap();
    assert!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&f.b.user_id})
            .await
            .unwrap()
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn deleted_byok_credential_waives_after_evidence_window_and_unblocks_removal_erasure() {
    assert_byok_waiver_and_cleanup(true).await;
}
#[tokio::test]
async fn expired_byok_call_record_waives_after_evidence_window_and_unblocks_removal_erasure() {
    assert_byok_waiver_and_cleanup(false).await;
}
#[tokio::test]
async fn byok_provider_evidence_inside_window_wins_and_legacy_clock_starts_at_recovery() {
    let f = fixture(Profile::TelnyxRelay, false).await;
    let s = active_session(&f, "v2:byok-provider-wins==").await;
    reserve_voice(&f, &s).await;
    session::terminal(&f.state, &s, Utc::now(), None, "transport_terminal")
        .await
        .unwrap();
    let server = wiremock::MockServer::start().await;
    profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(f.b.account_id.clone(), server.uri());
    // Legacy rows cannot use an owner-provided ended_at to expire the window.
    f.state.db.collection::<bson::Document>(SESSIONS).update_one(doc! {"_id":&s.id},
        doc! {"$unset":{"termination_observed_at":""},"$set":{"ended_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(30))}}).await.unwrap();
    coordinator::recover_once(&f.state).await.unwrap();
    let legacy = session::load(&f.state, &s.id).await.unwrap();
    assert!(legacy.termination_observed_at.unwrap() > Utc::now() - Duration::minutes(1));
    assert!(legacy.terminated_quantity_pending);
    assert!(!row(&f, &s).await.released);
    age_evidence_window(&f, &s, 71).await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"data":{"call_control_id":s.provider_call_id,"is_alive":false,"call_duration":23}})))
        .mount(&server).await;
    coordinator::recover_once(&f.state).await.unwrap();
    let saved = session::load(&f.state, &s.id).await.unwrap();
    assert_eq!(saved.provider_seconds, Some(23));
    assert_eq!(
        saved.terminal_evidence.as_deref(),
        Some("authenticated_provider_status")
    );
    assert!(saved.billing_finalized);
    assert_eq!(row(&f, &s).await.quantity, Some(23));
    assert_eq!(pending_wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        pending_wallet(&f).await.pending_lago_debits,
        Credits::from_whole(46)
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"channel_adapter_provider_evidence_unavailable"})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn human_admin_resolves_byok_evidence_once_without_readiness_or_credentials() {
    use crate::services::{audit_service::AuditActor, call_channels::managed::calls};
    let f = fixture(Profile::TelnyxRelay, false).await;
    let s = active_session(&f, "v2:byok-admin-evidence==").await;
    reserve_voice(&f, &s).await;
    session::terminal(&f.state, &s, Utc::now(), None, "transport_terminal")
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    let ordinary = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&f.b.user_id));
    assert!(
        calls::resolve_evidence(&f.state, &ordinary, &s.id, 31, "private provider reference")
            .await
            .is_err()
    );
    let id = Uuid::new_v4().to_string();
    let role = Uuid::new_v4().to_string();
    let mut admin = test_user(&id, UserType::Person);
    admin.role_ids.push(role.clone());
    f.state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(admin)
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::role::COLLECTION_NAME).insert_one(doc! {"_id":role,"name":"Admin","slug":"admin","is_system":true,"client_id":bson::Bson::Null,"permissions":[],"is_default":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let actor = AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&id));
    let mut key_actor = actor.clone();
    key_actor.api_key_id = Some(Uuid::new_v4().to_string());
    assert!(
        calls::resolve_evidence(
            &f.state,
            &key_actor,
            &s.id,
            31,
            "private provider reference"
        )
        .await
        .is_err()
    );
    calls::resolve_evidence(&f.state, &actor, &s.id, 31, "private provider reference")
        .await
        .unwrap();
    calls::resolve_evidence(&f.state, &actor, &s.id, 31, "private provider reference")
        .await
        .unwrap();
    assert!(
        calls::resolve_evidence(&f.state, &actor, &s.id, 32, "private provider reference")
            .await
            .is_err()
    );
    assert!(
        calls::resolve_evidence(&f.state, &actor, &s.id, 31, "changed reference")
            .await
            .is_err()
    );
    let saved = session::load(&f.state, &s.id).await.unwrap();
    assert!(saved.billing_finalized);
    assert_eq!(saved.provider_seconds, Some(31));
    assert_eq!(
        pending_wallet(&f).await.pending_lago_debits,
        Credits::from_whole(62)
    );
    let audits = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"channel_adapter_call_evidence_resolved"})
            .await
            .unwrap(),
        1
    );
    let audit = audits
        .find_one(doc! {"event_type":"channel_adapter_call_evidence_resolved"})
        .await
        .unwrap()
        .unwrap();
    assert!(audit.contains_key("entry_hash"));
    assert!(!format!("{audit:?}").contains("private provider reference"));
    f.state.db.drop().await.unwrap();
}
