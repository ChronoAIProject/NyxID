use super::super::protocol;
use super::*;
use crate::{
    AppState,
    models::{
        channel_adapter::*,
        credits::Credits,
        downstream_service::{DownstreamService, test_helpers::dummy_service},
        managed_comms::*,
        user::{User, UserType},
    },
    services::{audit_service::AuditActor, feature_flag_service as flags, transaction_service},
    test_utils::{connect_transaction_test_database, test_app_state, test_auth_user, test_user},
};
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn rate(service: &str) -> RateSnapshot {
    RateSnapshot {
        service_id: service.into(),
        service_slug: "managed-rental".into(),
        metric: crate::models::service_billing::BillingMetric::PhoneNumberDays,
        rate: "0.000000000001".parse().unwrap(),
        lago_metric_code: "platform_svc_managed_rental_pk".into(),
        revision: "one".into(),
    }
}
struct Fixture {
    state: AppState,
    owner: String,
    admin: AuditActor,
    line: ManagedPhoneLine,
    claim: ManagedLineClaim,
    credential: CredentialBinding,
    service: DownstreamService,
    account: ProviderAccount,
    period: ManagedLinePeriod,
}
async fn fixture() -> Fixture {
    let db = connect_transaction_test_database("managed_m1").await;
    indexes::ensure_indexes(&db).await.unwrap();
    super::super::session::ensure_indexes(&db).await.unwrap();
    let owner = id();
    let admin_id = id();
    let role = id();
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let mut admin = test_user(&admin_id, UserType::Person);
    admin.role_ids.push(role.clone());
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(admin)
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::role::COLLECTION_NAME).insert_one(doc!{"_id":role,"name":"Admin","slug":"admin","is_system":true,"client_id":bson::Bson::Null,"permissions":[],"is_default":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    // Startup seeds both platform system roles; admin authorization resolves both.
    db.collection::<bson::Document>(crate::models::role::COLLECTION_NAME).insert_one(doc!{"_id":id(),"name":"Operator","slug":"operator","is_system":true,"client_id":bson::Bson::Null,"permissions":[],"is_default":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let state = test_app_state(db);
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let admin = AuditActor::from_auth_user(&test_auth_user(&admin_id));
    let now = Utc::now();
    let mut service = dummy_service();
    service.id = id();
    service.slug = "managed-canonical".into();
    service.created_by = admin_id;
    service.visibility = "private".into();
    service.service_category = "internal".into();
    service.owner_user_id = None;
    service.managed_comms_role = Some(ManagedCommsRole::Credential);
    service.base_url = "https://api.twilio.com".into();
    service.auth_method = "basic".into();
    service.platform_key = None;
    service.provider_config_id = None;
    service.identity_propagation_mode = "none".into();
    service.forward_access_token = false;
    service.inject_delegation_token = false;
    let account = ProviderAccount {
        id: id(),
        catalog_service_id: service.id.clone(),
        provider: "twilio".into(),
        account_id: format!("AC{}", Uuid::new_v4().simple()),
        webhook_public_key: None,
        revision: 1,
        configured_by: admin.user_id.clone(),
        updated_at: now,
        underwriting: None,
        fence: 0,
    };
    service.credential_encrypted = crate::services::catalog_credential_record_service::encrypt(
        &state.encryption_keys,
        &service.id,
        &service.created_by,
        format!("{}:fixture-secret", account.account_id).as_bytes(),
    )
    .await
    .unwrap();
    service.secret_context_versions =
        crate::services::catalog_credential_record_service::initial_versions(
            &state.encryption_keys,
            true,
        );
    let mut price = dummy_service();
    price.id = id();
    price.slug = "managed-price".into();
    price.created_by = admin.user_id.clone();
    price.visibility = "private".into();
    price.service_category = "internal".into();
    price.owner_user_id = None;
    price.managed_comms_role = Some(ManagedCommsRole::Voice);
    price.credential_encrypted.clear();
    price.platform_key = None;
    state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&price)
        .await
        .unwrap();
    state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&service)
        .await
        .unwrap();
    state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .insert_one(&account)
        .await
        .unwrap();
    let service = state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":&service.id})
        .await
        .unwrap()
        .unwrap();
    let line = ManagedPhoneLine {
        standard_voice_options: vec![],
        id: id(),
        provider: "twilio".into(),
        trusted_account_id: account.id.clone(),
        phone_number: "+18005550123".into(),
        country: "US".into(),
        number_resource_id: format!("PN{}", Uuid::new_v4().simple()),
        voice: Some(Capability {
            application_id: format!("AP{}", Uuid::new_v4().simple()),
            billing_service_id: price.id.clone(),
        }),
        sms: None,
        rental_billing_service_id: id(),
        regulatory: RegulatoryEvidence {
            requirement_reference: "fixture".into(),
            registration_status: "approved".into(),
            sms_sender_registration: None,
            verified_by: admin.user_id.clone(),
            verified_at: now,
        },
        supplier_tariffs: vec![],
        provider_channel_limit: None,
        call_slot_limit: 1,
        calls_per_minute: 10,
        sms_per_minute: 10,
        state: LineState::Claimed,
        suspension: None,
        current_claim_id: None,
        next_claim_generation: 2,
        revision: 1,
        fence: 1,
        available_at: None,
        supplier_released_at: None,
        created_at: now,
        updated_at: now,
    };
    let input = eligibility_input(None);
    let eligible = eligibility::save(&state.db, &admin, &owner, input, &[7; 32])
        .await
        .unwrap();
    let owner_actor = AuditActor::from_auth_user(&test_auth_user(&owner));
    eligibility::accept_terms(
        &state.db,
        &owner_actor,
        eligible.revision,
        eligible.required_terms.clone(),
        &[7; 32],
    )
    .await
    .unwrap();
    let claim = ManagedLineClaim {
        standard_voice: None,
        id: id(),
        line_id: line.id.clone(),
        user_id: owner.clone(),
        managed_claim_generation: 1,
        eligibility_revision: 1,
        accepted_terms: eligible.required_terms,
        agent_id: id(),
        voice_tier: VoiceTier::Standard,
        sms_enabled: false,
        caller_confirmations: false,
        auto_renew: false,
        accepted_rental_rate: rate(&line.rental_billing_service_id),
        state: LineState::Claimed,
        suspension: None,
        voice_binding_id: None,
        sms_binding_id: None,
        fence: 1,
        covered_from: now - Duration::hours(1),
        created_at: now,
        release_requested_at: None,
        suspended_at: None,
    };
    let mut line = line;
    line.current_claim_id = Some(claim.id.clone());
    let period = ManagedLinePeriod {
        id: id(),
        claim_id: claim.id.clone(),
        managed_claim_generation: 1,
        period_index: 0,
        covered_from: now - Duration::hours(1),
        covered_until: now + Duration::days(30) - Duration::hours(1),
        quantity: 30,
        rate: rate(&line.rental_billing_service_id),
        billing_request_id: format!("managed-line-rental:{}:0", claim.id),
        usage_row_ids: vec![id()],
        funded: true,
        spend_hold_id: None,
        created_at: now,
    };
    state
        .db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .insert_one(&line)
        .await
        .unwrap();
    state
        .db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .insert_one(&claim)
        .await
        .unwrap();
    state
        .db
        .collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .insert_one(&period)
        .await
        .unwrap();
    for (kind, value) in [
        ("number_id", line.number_resource_id.as_str()),
        ("phone", line.phone_number.as_str()),
        (
            "application",
            line.voice.as_ref().unwrap().application_id.as_str(),
        ),
    ] {
        state
            .db
            .collection::<ResourceClaim>(CLAIMS)
            .insert_one(ResourceClaim {
                id: id(),
                identity_key: super::super::session::identity(&["twilio", kind, value]),
                user_id: custody::sentinel(&line.id),
                provider: "twilio".into(),
                resource_kind: kind.into(),
                resource_id: value.into(),
                revision: 1,
                created_at: now,
            })
            .await
            .unwrap();
    }
    state.db.collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME).insert_one(doc!{"_id":&period.usage_row_ids[0],"billing_request_id":&period.billing_request_id,"billing_owner_id":&owner,"metric":"phone_number_days","status":"reserved"}).await.unwrap();
    let credential = CredentialBinding::ManagedLine {
        line_id: line.id.clone(),
        managed_claim_id: claim.id.clone(),
        managed_claim_generation: 1,
    };
    Fixture {
        state,
        owner,
        admin,
        line,
        claim,
        credential,
        service,
        account,
        period,
    }
}
fn eligibility_input(revision: Option<i64>) -> eligibility::EligibilityInput {
    eligibility::EligibilityInput {
        business_legal_name: "Fixture merchant".into(),
        registration_id: "fixture-reg".into(),
        jurisdiction: "SG".into(),
        allowed_use_case: "Restaurant bookings".into(),
        sms_sender_path: "registered".into(),
        sms_registration_status: "approved".into(),
        required_terms: TermsVersions {
            managed_line: "v1".into(),
            elevenlabs_eua: None,
        },
        premium_allowed: false,
        max_lines: 1,
        monthly_spend_limit: None,
        expected_revision: revision,
    }
}
async fn enable(f: &Fixture) {
    crate::services::security_readiness_service::ready_test_replica(&f.state.db).await;
    for flag in [
        flags::NYXBOT_ADAPTERS_FLAG_KEY,
        flags::MANAGED_LINES_FLAG_KEY,
    ] {
        flags::set_platform_override(
            &f.state.db,
            flag,
            &flags::FlagTarget::User(f.owner.clone()),
            true,
            &f.admin.user_id,
        )
        .await
        .unwrap();
    }
}
#[tokio::test]
async fn managed_gate_flags_coverage_terms_revision_and_revocation() {
    let f = fixture().await;
    let db = &f.state.db;
    assert!(admission::read(db, &f.owner, &f.credential).await.is_err());
    enable(&f).await;
    assert!(admission::read(db, &f.owner, &f.credential).await.is_ok());
    db.collection::<bson::Document>(ManagedLinePeriod::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.period.id},
            doc! {"$set":{"covered_until":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert!(admission::read(db, &f.owner, &f.credential).await.is_err());
    db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.period.id}, &f.period)
        .await
        .unwrap();
    let mut input = eligibility_input(Some(1));
    input.allowed_use_case = "Booking updates".into();
    eligibility::save(db, &f.admin, &f.owner, input, &[7; 32])
        .await
        .unwrap();
    assert!(admission::read(db, &f.owner, &f.credential).await.is_err());
    eligibility::apply_revision(db, &f.admin, &f.owner, 2, &[7; 32])
        .await
        .unwrap();
    assert!(admission::read(db, &f.owner, &f.credential).await.is_ok());
    let mut input = eligibility_input(Some(2));
    input.required_terms.managed_line = "v2".into();
    eligibility::save(db, &f.admin, &f.owner, input, &[7; 32])
        .await
        .unwrap();
    assert!(
        eligibility::apply_revision(db, &f.admin, &f.owner, 3, &[7; 32])
            .await
            .is_err()
    );
    let a = AuditActor::from_auth_user(&test_auth_user(&f.owner));
    assert!(
        eligibility::accept_terms(
            db,
            &a,
            2,
            TermsVersions {
                managed_line: "v1".into(),
                elevenlabs_eua: None
            },
            &[7; 32]
        )
        .await
        .is_err()
    );
    eligibility::accept_terms(
        db,
        &a,
        3,
        TermsVersions {
            managed_line: "v2".into(),
            elevenlabs_eua: None,
        },
        &[7; 32],
    )
    .await
    .unwrap();
    eligibility::apply_revision(db, &f.admin, &f.owner, 3, &[7; 32])
        .await
        .unwrap();
    let observed = admission::read(db, &f.owner, &f.credential).await.unwrap();
    eligibility::revoke(&f.state, &f.admin, &f.owner, 3, &[7; 32])
        .await
        .unwrap();
    let context = (db, &observed);
    assert!(
        transaction_service::run_db_only(db, &context, |tx, c| Box::pin(async move {
            admission::fence(c.0, tx, c.1, Utc::now()).await
        }))
        .await
        .is_err()
    );
    assert!(admission::read(db, &f.owner, &f.credential).await.is_err());
    let line = db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.line.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line.suspension, Some(Suspension::EligibilityRevoked));
    assert_audit_metadata(
        db,
        &[
            "managed_eligibility_saved",
            "managed_terms_accepted",
            "managed_eligibility_applied",
            "managed_eligibility_revoked",
        ],
    )
    .await;
}
#[tokio::test]
async fn eligibility_cannot_self_approve_or_approve_org_and_stale_fence_refuses() {
    let f = fixture().await;
    let db = &f.state.db;
    let a = AuditActor::from_auth_user(&test_auth_user(&f.owner));
    assert!(
        eligibility::save(db, &a, &f.owner, eligibility_input(Some(1)), &[7; 32])
            .await
            .is_err()
    );
    let org = id();
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    assert!(
        eligibility::save(db, &f.admin, &org, eligibility_input(None), &[7; 32])
            .await
            .is_err()
    );
    enable(&f).await;
    let old = admission::read(db, &f.owner, &f.credential).await.unwrap();
    db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.line.id},
            doc! {"$inc":{"revision":1_i64,"fence":1_i64}},
        )
        .await
        .unwrap();
    let context = (db, &old);
    assert!(
        transaction_service::run_db_only(db, &context, |tx, c| Box::pin(async move {
            admission::fence(c.0, tx, c.1, Utc::now()).await
        }))
        .await
        .is_err()
    );
}
#[tokio::test]
async fn coverage_requires_the_exact_durable_funding_identity() {
    let f = fixture().await;
    enable(&f).await;
    let rows = f
        .state
        .db
        .collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME);
    for fields in [
        doc! {"billing_owner_id":id()},
        doc! {"billing_request_id":"other-request"},
        doc! {"metric":"voice_seconds"},
        doc! {"status":"cancelled"},
    ] {
        let original = rows
            .find_one(doc! {"_id":&f.period.usage_row_ids[0]})
            .await
            .unwrap()
            .unwrap();
        rows.update_one(
            doc! {"_id":&f.period.usage_row_ids[0]},
            doc! {"$set":fields},
        )
        .await
        .unwrap();
        assert!(
            admission::read(&f.state.db, &f.owner, &f.credential)
                .await
                .is_err()
        );
        rows.replace_one(doc! {"_id":&f.period.usage_row_ids[0]}, original)
            .await
            .unwrap();
    }
    let admitted = admission::read(&f.state.db, &f.owner, &f.credential)
        .await
        .unwrap();
    rows.delete_one(doc! {"_id":&f.period.usage_row_ids[0]})
        .await
        .unwrap();
    assert!(
        admission::read(&f.state.db, &f.owner, &f.credential)
            .await
            .is_err()
    );
    let context = (&f.state.db, &admitted);
    assert!(
        transaction_service::run_db_only(&f.state.db, &context, |tx, c| Box::pin(async move {
            admission::fence(c.0, tx, c.1, Utc::now()).await
        }))
        .await
        .is_err()
    );
}
/// A second claimed, funded line of the same owner on the fixture's trusted account.
async fn second_line(f: &Fixture) -> CredentialBinding {
    let db = &f.state.db;
    let mut claim = f.claim.clone();
    claim.id = id();
    let mut line = f.line.clone();
    line.id = id();
    line.number_resource_id = id();
    line.phone_number = "+18005550177".into();
    if let Some(voice) = line.voice.as_mut() {
        voice.application_id = id();
    }
    line.current_claim_id = Some(claim.id.clone());
    claim.line_id = line.id.clone();
    claim.voice_binding_id = None;
    claim.sms_binding_id = None;
    let mut period = f.period.clone();
    period.id = id();
    period.claim_id = claim.id.clone();
    period.billing_request_id = format!("managed-line-rental:{}:0", claim.id);
    period.usage_row_ids = vec![id()];
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .insert_one(&line)
        .await
        .unwrap();
    db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .insert_one(&claim)
        .await
        .unwrap();
    db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .insert_one(&period)
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME).insert_one(doc!{"_id":&period.usage_row_ids[0],"billing_request_id":&period.billing_request_id,"billing_owner_id":&f.owner,"metric":"phone_number_days","status":"reserved"}).await.unwrap();
    CredentialBinding::ManagedLine {
        line_id: line.id,
        managed_claim_id: claim.id,
        managed_claim_generation: claim.managed_claim_generation,
    }
}
async fn admit(db: &Database, a: &admission::Admission) -> crate::errors::AppResult<()> {
    transaction_service::run_db_only(db, &(db, a), |tx, c| {
        Box::pin(async move { admission::fence(c.0, tx, c.1, Utc::now()).await })
    })
    .await
}
async fn counters(db: &Database, collection: &str, id: &str) -> (i64, i64) {
    let row = db
        .collection::<bson::Document>(collection)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    let read = |field| match row.get(field) {
        Some(bson::Bson::Int64(n)) => *n,
        Some(bson::Bson::Int32(n)) => i64::from(*n),
        _ => 0,
    };
    (read("fence"), read(admission::ADMISSION_TOUCH))
}
#[tokio::test]
async fn concurrent_admissions_on_one_account_retry_against_unchanged_authority() {
    let f = fixture().await;
    enable(&f).await;
    let db = f.state.db.clone();
    let other = second_line(&f).await;
    let first = admission::read(&db, &f.owner, &f.credential).await.unwrap();
    let second = admission::read(&db, &f.owner, &other).await.unwrap();
    assert_eq!(first.account.id, second.account.id);
    assert_ne!(first.line.id, second.line.id);
    let before = (
        counters(&db, ACCOUNTS, &f.account.id).await,
        counters(&db, ManagedPhoneLine::COLLECTION_NAME, &f.line.id).await,
        counters(&db, ManagedLineClaim::COLLECTION_NAME, &f.claim.id).await,
    );
    // Hold one admission's writes uncommitted: the contenders on the other line
    // and on the same line must conflict, retry after it commits and succeed,
    // because nothing changed their authority.
    let mut held = db.client().start_session().await.unwrap();
    held.start_transaction().await.unwrap();
    admission::fence(&db, &mut held, &first, Utc::now())
        .await
        .unwrap();
    let contenders = {
        let (db, first, second) = (db.clone(), first.clone(), second.clone());
        tokio::spawn(async move { tokio::join!(admit(&db, &second), admit(&db, &first)) })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    held.commit_transaction().await.unwrap();
    let (other_line, same_line) = contenders.await.unwrap();
    other_line.unwrap();
    same_line.unwrap();
    // Admissions advance only the conflict counter, never the authority version.
    let after = (
        counters(&db, ACCOUNTS, &f.account.id).await,
        counters(&db, ManagedPhoneLine::COLLECTION_NAME, &f.line.id).await,
        counters(&db, ManagedLineClaim::COLLECTION_NAME, &f.claim.id).await,
    );
    assert_eq!(after.0, (before.0.0, before.0.1 + 3));
    assert_eq!(after.1, (before.1.0, before.1.1 + 2));
    assert_eq!(after.2, (before.2.0, before.2.1 + 2));
    // The original reads stay valid for later admissions too.
    admit(&db, &first).await.unwrap();
    admit(&db, &second).await.unwrap();
}
#[tokio::test]
async fn authority_writes_between_admission_read_and_fence_still_refuse() {
    let f = fixture().await;
    enable(&f).await;
    let db = &f.state.db;
    let other = second_line(&f).await;
    let refused = |result: crate::errors::AppResult<()>| {
        assert!(
            matches!(&result, Err(crate::errors::AppError::Conflict(m)) if m == "Managed admission authority changed"),
            "{:?}",
            result.err()
        );
    };
    // An account authority write (rotation, underwriting, premium or trust
    // change) advances the account fence and refuses every line's older read.
    let first = admission::read(db, &f.owner, &f.credential).await.unwrap();
    let second = admission::read(db, &f.owner, &other).await.unwrap();
    admit(db, &second).await.unwrap();
    db.collection::<bson::Document>(ACCOUNTS)
        .update_one(doc! {"_id":&f.account.id}, doc! {"$inc":{"fence":1_i64}})
        .await
        .unwrap();
    refused(admit(db, &first).await);
    refused(admit(db, &second).await);
    // A real admin suspension committed after the read refuses only that line.
    let first = admission::read(db, &f.owner, &f.credential).await.unwrap();
    let second = admission::read(db, &f.owner, &other).await.unwrap();
    let line = db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.line.id})
        .await
        .unwrap()
        .unwrap();
    lifecycle::suspend(
        &f.state,
        &f.admin,
        &f.line.id,
        lifecycle::LineRevisionInput {
            revision: line.revision,
        },
    )
    .await
    .unwrap();
    assert!(admit(db, &first).await.is_err());
    admit(db, &second).await.unwrap();
}
async fn hold(
    db: &Database,
    b: &ManagedBudget,
    key: &str,
    amount: i64,
    now: chrono::DateTime<Utc>,
) -> crate::errors::AppResult<ManagedBudgetEntry> {
    let context = (db, b, key, amount, now);
    transaction_service::run_db_only(db, &context, |tx, c| {
        Box::pin(
            async move { budgets::hold(c.0, tx, c.1, c.2, Credits::from_whole(c.3), c.4).await },
        )
    })
    .await
}
async fn recognize(
    db: &Database,
    entry: &str,
    amount: i64,
    now: chrono::DateTime<Utc>,
) -> crate::errors::AppResult<()> {
    let context = (db, entry, amount, now);
    transaction_service::run_db_only(db, &context, |tx, c| {
        Box::pin(
            async move { budgets::recognize(c.0, tx, c.1, Credits::from_whole(c.2), c.3).await },
        )
    })
    .await
}
#[tokio::test]
async fn budget_serializes_competing_holds_and_replay_and_recognizes_full_loss() {
    let db = connect_transaction_test_database("managed_budget").await;
    indexes::ensure_indexes(&db).await.unwrap();
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    let b = ManagedBudget {
        id: id(),
        budget_key: id(),
        window: BudgetWindow::RollingThirtyDays,
        limit: Credits::from_whole(100),
        revision: 1,
        fence: 1,
    };
    db.collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME)
        .insert_one(&b)
        .await
        .unwrap();
    let now = Utc::now();
    let (a, c) = tokio::join!(hold(&db, &b, "a", 70, now), hold(&db, &b, "c", 70, now));
    assert_eq!(usize::from(a.is_ok()) + usize::from(c.is_ok()), 1);
    let error = if a.is_err() {
        a.as_ref().err().unwrap()
    } else {
        c.as_ref().err().unwrap()
    };
    assert_eq!(error.error_key(), "managed_budget_exhausted");
    assert_eq!(error.error_code(), 13400);
    let entry = a.or(c).unwrap();
    assert_eq!(
        hold(&db, &b, &entry.effect_key, 70, now).await.unwrap().id,
        entry.id
    );
    assert!(hold(&db, &b, &entry.effect_key, 69, now).await.is_err());
    // Even after the rolling window, unresolved liability still occupies capacity.
    assert!(
        hold(&db, &b, "later", 31, now + Duration::days(31))
            .await
            .is_err()
    );
    recognize(&db, &entry.id, 110, now + Duration::days(31))
        .await
        .unwrap();
    recognize(&db, &entry.id, 110, now + Duration::days(31))
        .await
        .unwrap();
    assert!(
        recognize(&db, &entry.id, 109, now + Duration::days(31))
            .await
            .is_err()
    );
    assert!(
        hold(&db, &b, "loss", 1, now + Duration::days(31))
            .await
            .is_err()
    );
    assert!(
        hold(&db, &b, "expired", 100, now + Duration::days(62))
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn monthly_budget_counts_admission_month_and_preserves_unresolved_prior_holds() {
    let db = connect_transaction_test_database("managed_month").await;
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    indexes::ensure_indexes(&db).await.unwrap();
    let now = "2026-11-30T23:59:00Z".parse().unwrap();
    let mut b = ManagedBudget {
        id: id(),
        budget_key: id(),
        window: BudgetWindow::CalendarMonth {
            year: 2026,
            month: 11,
        },
        limit: Credits::from_whole(100),
        revision: 1,
        fence: 1,
    };
    db.collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME)
        .insert_one(&b)
        .await
        .unwrap();
    let old = hold(&db, &b, "november", 80, now).await.unwrap();
    b.window = BudgetWindow::CalendarMonth {
        year: 2026,
        month: 12,
    };
    b.revision += 1;
    db.collection::<ManagedBudget>(ManagedBudget::COLLECTION_NAME)
        .replace_one(doc! {"_id":&b.id}, &b)
        .await
        .unwrap();
    assert!(
        hold(&db, &b, "december", 21, now + Duration::days(1))
            .await
            .is_err()
    );
    recognize(&db, &old.id, 80, now + Duration::days(1))
        .await
        .unwrap();
    assert!(
        hold(&db, &b, "december", 100, now + Duration::days(1))
            .await
            .is_ok()
    );
}
#[test]
fn managed_models_store_bson_dates_exact_money_and_redact_references() {
    let now = Utc::now();
    let b = ManagedBudgetEntry {
        id: id(),
        budget_id: id(),
        effect_key: "key".into(),
        held: "0.000000000001".parse().unwrap(),
        recognized: None,
        admitted_at: now,
        recognized_at: None,
    };
    let doc = bson::to_document(&b).unwrap();
    assert!(matches!(doc["admitted_at"], bson::Bson::DateTime(_)));
    assert!(matches!(doc["held"], bson::Bson::Decimal128(_)));
    assert_eq!(
        bson::from_document::<ManagedBudgetEntry>(doc).unwrap().held,
        b.held
    );
    let input: custody::RotationInput = serde_json::from_value(
        serde_json::json!({"expected_revision":1,"credential":"do-not-display"}),
    )
    .unwrap();
    assert!(!format!("{input:?}").contains("do-not-display"));
}
#[tokio::test]
async fn cleanup_uses_retained_account_after_revocation_and_policy_deactivation() {
    let f = fixture().await;
    let db = &f.state.db;
    eligibility::revoke(&f.state, &f.admin, &f.owner, 1, &[7; 32])
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.service.id},
            doc! {"$set":{"is_active":false,"proxy_operation_policy":bson::Bson::Null}},
        )
        .await
        .unwrap();
    let admitted = AdmittedReference {
        trusted_account_id: f.account.id.clone(),
        credential_service_id: f.service.id.clone(),
        provider: "twilio".into(),
        account_id: f.account.account_id.clone(),
        line_id: f.line.id.clone(),
        number_resource_id: f.line.number_resource_id.clone(),
        application_id: f.line.voice.as_ref().unwrap().application_id.clone(),
        provider_call_id: Some("CAfixture".into()),
        provider_message_id: None,
        call_leg_id: None,
        call_session_id: None,
    };
    let route = ManagedEffectRoute {
        id: id(),
        trusted_account_id: f.account.id.clone(),
        provider: "twilio".into(),
        provider_effect_id: "CAfixture".into(),
        line_id: f.line.id.clone(),
        managed_claim_id: Some(f.claim.id.clone()),
        managed_claim_generation: Some(1),
        binding_id: None,
        effect_id: id(),
        kind: EffectKind::Call,
        financially_finalized: false,
        admitted,
        covered_from: Utc::now(),
        covered_until: None,
        created_at: Utc::now(),
    };
    db.collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
        .insert_one(&route)
        .await
        .unwrap();
    let target = resolver::cleanup(&f.state, &route.id, resolver::CleanupOperation::Hangup)
        .await
        .unwrap();
    assert!(target.url.as_str().ends_with("/Calls/CAfixture.json"));
    assert_eq!(target.method, "POST");
    assert!(
        matches!(target.body,Some(resolver::CleanupBody::Form(ref fields)) if fields==&[("Status","completed")])
    );
    assert!(target.key.ends_with(":fixture-secret"));
    assert!(
        resolver::cleanup(&f.state, &route.id, resolver::CleanupOperation::ReadMessage)
            .await
            .is_err()
    );
    assert!(
        resolver::cleanup(
            &f.state,
            &route.id,
            resolver::CleanupOperation::ReadDetailRecord
        )
        .await
        .is_err()
    );
    assert!(!format!("{:?}", route.admitted).contains(&f.account.account_id));
}

#[tokio::test]
async fn twilio_registration_proves_exact_account_capability_url_and_empty_fallback() {
    let f = fixture().await;
    let (server, service) = twilio_proof_fixture(&f).await;
    let key = format!("{}:fixture-secret", f.account.account_id);
    assert_eq!(
        custody::prove(
            &f.state,
            &f.line,
            &f.account,
            &service,
            &key,
            Some(&server.uri())
        )
        .await
        .unwrap(),
        None
    );
    // A higher-priority application response cannot smuggle a fallback effect.
    let app = f.line.voice.as_ref().unwrap();
    let app_path = format!(
        "/2010-04-01/Accounts/{}/Applications/{}.json",
        f.account.account_id, app.application_id
    );
    Mock::given(method("GET")).and(path(app_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"account_sid":f.account.account_id,"sid":app.application_id,"voice_url":custody::stable_url(&f.state.config.base_url,&f.line.id,false),"voice_method":"POST","voice_fallback_url":"https://untrusted.invalid"}))).with_priority(1).mount(&server).await;
    assert!(
        custody::prove(
            &f.state,
            &f.line,
            &f.account,
            &service,
            &key,
            Some(&server.uri())
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn custody_and_dependency_fences_block_legacy_and_unsafe_mutations() {
    let f = fixture().await;
    let db = &f.state.db;
    let mut b:Binding=bson::from_document(doc!{"_id":id(),"user_id":&f.owner,"profile":"twilio_relay","contract_version":1,"revision":1_i64,"label":"fixture","agent_id":&f.claim.agent_id,"credential":{"kind":"platform","catalog_service_id":&f.service.id},"account_id":&f.account.account_id,"resource_id":&f.line.number_resource_id,"phone_number":&f.line.phone_number,"application_id":&f.line.voice.as_ref().unwrap().application_id,"voice":"en-US-Standard-A","language":"en-US","enabled":true,"verified_credential_identity":"","verified_at":bson::DateTime::now(),"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
    assert!(custody::refuse_legacy(db, &b).await.is_err());
    b.credential = f.credential.clone();
    assert!(
        super::super::session::prepare(&f.state, &b, "CAfixture", "+18005550124", &b.phone_number)
            .await
            .is_err()
    );
    let context = (db, &f.account.id, &f.service);
    assert!(
        transaction_service::run_db_only(db, &context, |tx, c| Box::pin(async move {
            custody::account_dependencies(c.0, tx, c.1).await
        }))
        .await
        .is_err()
    );
    for fields in [
        doc! {"is_active":false},
        doc! {"base_url":"https://elsewhere.invalid"},
        doc! {"credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2]}},
        doc! {"platform_key":{"enabled":true,"audience":"public"}},
        doc! {"managed_comms_role":bson::Bson::Null},
    ] {
        let context = (db, &f.service, &fields);
        assert!(
            transaction_service::run_db_only(db, &context, |tx, c| Box::pin(async move {
                custody::validate_catalog_update(c.0, tx, c.1, c.2).await
            }))
            .await
            .is_err()
        );
    }
    let fields = doc! {"description":"Updated display text"};
    let context = (db, &f.service, &fields);
    transaction_service::run_db_only(db, &context, |tx, c| {
        Box::pin(async move { custody::validate_catalog_update(c.0, tx, c.1, c.2).await })
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn every_managed_role_refuses_generic_proxy_keys_catalog_connect_and_public_ingress() {
    use crate::models::downstream_service::{
        AnonymousEndpointRule, PlatformKeyAudience, PlatformKeyConfig,
    };
    use crate::services::{
        catalog_service, connect_link_service, platform_key_service as pk, proxy_service,
        unified_key_service,
    };
    let f = fixture().await;
    let db = &f.state.db;
    for role in [
        ManagedCommsRole::Credential,
        ManagedCommsRole::Voice,
        ManagedCommsRole::Sms,
        ManagedCommsRole::Rental,
        ManagedCommsRole::Premium,
    ] {
        let mut s = f.service.clone();
        s.id = id();
        s.slug = format!("managed-{}", id());
        s.managed_comms_role = Some(role);
        s.service_category = "internal".into();
        s.requires_user_credential = false;
        s.platform_key = Some(PlatformKeyConfig {
            enabled: true,
            audience: PlatformKeyAudience::Public,
            ..Default::default()
        });
        s.anonymous_endpoints = vec![AnonymousEndpointRule {
            id: id(),
            enabled: true,
            method: "GET".into(),
            path_pattern: "/status".into(),
            daily_quota: 10,
        }];
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&s)
            .await
            .unwrap();
        assert!(pk::require_ordinary_use(&s).is_err());
        assert!(!pk::has_platform_key(&s));
        assert!(!pk::legacy_master_credential(&s));
        assert!(
            proxy_service::resolve_catalog_platform_target(
                db,
                &f.state.encryption_keys,
                &f.owner,
                s.clone(),
                f.state.platform_user_rate_limit
            )
            .await
            .is_err()
        );
        assert!(
            catalog_service::get_downstream_service_by_slug(db, &s.slug, &f.owner)
                .await
                .is_err()
        );
        assert!(
            unified_key_service::create_platform_key(
                db, &f.owner, &f.owner, &s.slug, "fixture", None, false, None
            )
            .await
            .is_err()
        );
        assert!(
            crate::services::anonymous_endpoint_service::find_matching_enabled_rule(
                db, &s.slug, "GET", "/status"
            )
            .await
            .is_err()
        );
        assert!(
            connect_link_service::catalog_info(db, s.clone())
                .await
                .is_err()
        );
        // Both executable and unavailable MCP discovery must hide stale managed
        // rows, including a formerly public anonymous tool.
        for tools in [
            crate::services::mcp_service::load_user_tools_all(
                db,
                &f.state.node_ws_manager,
                &f.owner,
            )
            .await
            .unwrap(),
            crate::services::mcp_service::load_public_tools(db)
                .await
                .unwrap(),
        ] {
            assert!(tools.iter().all(|tool| tool.service_id != s.id));
        }
        // A stale provider association must not route this row through LLM ingress.
        let provider: crate::models::provider_config::ProviderConfig = bson::from_document(doc! {
            "_id":id(),"slug":"fixture-provider","name":"Fixture","provider_type":"api_key",
            "is_active":true,"created_by":&f.admin.user_id,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).unwrap();
        db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&s.id},
                doc! {"$set":{"slug":format!("llm-{}", s.id),"provider_config_id":&provider.id}},
            )
            .await
            .unwrap();
        assert!(
            crate::services::llm_gateway_service::resolve_llm_service_for_provider(db, &provider)
                .await
                .is_err()
        );
    }
}
#[test]
fn underwriting_uses_supplier_increments_and_requires_all_evidence() {
    let mut p = underwriting_policy();
    assert_eq!(
        custody::worst_case_exposure(&p).unwrap(),
        Credits::from_whole(360)
    );
    p.paid_tier_evidence_reference.clear();
    assert!(custody::worst_case_exposure(&p).is_err());
}
fn underwriting_policy() -> ManagedUnderwriting {
    ManagedUnderwriting {
        acceptance_text: "fixture acceptance".into(),
        timestamp_fixture_reference: Some("fixture".into()),
        detail_record_options_reference: Some("options fixture".into()),
        detail_record_fixture_reference: Some("cdr fixture".into()),
        character_tariff_reference: "fixture character tariff".into(),
        enabled: true,
        accepted_by: id(),
        accepted_at: Utc::now(),
        acceptance_version: "v1".into(),
        paid_tier_evidence_reference: "paid-tier-fixture".into(),
        max_leg_seconds: 1801,
        max_leg_evidence_reference: "written-provider-limit".into(),
        detail_record_fields: vec![
            "call_leg_id".into(),
            "connection_id".into(),
            "answered_at".into(),
            "finished_at".into(),
        ],
        supplier_tariff_revision: 1,
        fx_policy_revision: "fixture-fx".into(),
        components: [CostKind::CarrierInbound, CostKind::Relay, CostKind::Stt]
            .into_iter()
            .map(|kind| ExposureComponent {
                kind,
                credits_per_second: Credits::from_whole(1),
                increment_seconds: 60,
                minimum_seconds: 60,
            })
            .collect(),
        rolling_budget: Credits::from_whole(1000),
        revision: 1,
    }
}

#[tokio::test]
async fn telnyx_registration_fences_total_provider_channel_capacity() {
    let f = fixture().await;
    let db = &f.state.db;
    // This test exercises the registration transaction after controlled provider
    // proof; the separate proof fixture checks provider response identity.
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.line.id})
        .await
        .unwrap();
    let mut account = f.account.clone();
    account.provider = "telnyx".into();
    let mut policy = underwriting_policy();
    policy.rolling_budget = Credits::from_whole(720);
    account.underwriting = Some(policy);
    db.collection::<ProviderAccount>(ACCOUNTS)
        .replace_one(doc! {"_id":&account.id}, &account)
        .await
        .unwrap();
    let mut rental = dummy_service();
    rental.id = f.line.rental_billing_service_id.clone();
    rental.slug = "managed-rental".into();
    rental.created_by = f.admin.user_id.clone();
    rental.visibility = "private".into();
    rental.service_category = "internal".into();
    rental.owner_user_id = None;
    rental.credential_encrypted.clear();
    rental.platform_key = None;
    rental.provider_config_id = None;
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(rental)
        .await
        .unwrap();
    let mut line = f.line.clone();
    line.id = id();
    line.provider = "telnyx".into();
    line.number_resource_id = id();
    line.phone_number = "+18005550129".into();
    line.voice.as_mut().unwrap().application_id = id();
    line.provider_channel_limit = Some(2);
    line.state = LineState::Available;
    line.current_claim_id = None;
    custody::persist(&f.state, &f.admin, &line, &account, &f.service)
        .await
        .unwrap();
    let account = db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&account.id})
        .await
        .unwrap()
        .unwrap();
    let mut additional = line.clone();
    additional.id = id();
    additional.number_resource_id = id();
    additional.phone_number = "+18005550128".into();
    additional.voice.as_mut().unwrap().application_id = id();
    assert!(
        custody::persist(&f.state, &f.admin, &additional, &account, &f.service)
            .await
            .is_err()
    );
    assert!(
        db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
            .find_one(doc! {"_id":&additional.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(db.collection::<ResourceClaim>(CLAIMS)
        .find_one(doc! {"identity_key":super::super::session::identity(&["telnyx","number_id",&additional.number_resource_id])}).await.unwrap().is_none());
}

async fn twilio_proof_fixture(f: &Fixture) -> (MockServer, DownstreamService) {
    use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};
    let server = MockServer::start().await;
    let number_path = format!(
        "/2010-04-01/Accounts/{}/IncomingPhoneNumbers/{}.json",
        f.account.account_id, f.line.number_resource_id
    );
    let cap = f.line.voice.as_ref().unwrap();
    let app_path = format!(
        "/2010-04-01/Accounts/{}/Applications/{}.json",
        f.account.account_id, cap.application_id
    );
    let mut service = f.service.clone();
    service.proxy_operation_policy = Some(ProxyOperationPolicy {
        rules: [&number_path, &app_path]
            .into_iter()
            .map(|p| ProxyOperationRule {
                method: "GET".into(),
                path_template: p.clone(),
                ..Default::default()
            })
            .collect(),
    });
    Mock::given(method("GET")).and(path(number_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"account_sid":f.account.account_id,"sid":f.line.number_resource_id,"phone_number":f.line.phone_number,"voice_application_sid":cap.application_id,"capabilities":{"voice":true},"voice_fallback_url":""}))).mount(&server).await;
    Mock::given(method("GET")).and(path(app_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"account_sid":f.account.account_id,"sid":cap.application_id,"voice_url":custody::stable_url(&f.state.config.base_url,&f.line.id,false),"voice_method":"POST","voice_fallback_url":""}))).mount(&server).await;
    (server, service)
}
#[tokio::test]
async fn verified_rotation_preserves_canonical_account_and_rejects_changed_account_proof() {
    let f = fixture().await;
    let db = &f.state.db;
    let (server, service) = twilio_proof_fixture(&f).await;
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&service.id}, &service)
        .await
        .unwrap();
    insert_rental(&f).await;
    // The original creator is demoted; a different live Admin performs recovery.
    let acting_id = id();
    let mut acting = test_user(&acting_id, UserType::Person);
    let original = db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.admin.user_id})
        .await
        .unwrap()
        .unwrap();
    acting.role_ids = original.role_ids;
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(acting)
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.admin.user_id}, doc! {"$set":{"role_ids":[]}})
        .await
        .unwrap();
    let recovery_admin = AuditActor::from_auth_user(&test_auth_user(&acting_id));
    let successor = format!("{}:successor-secret", f.account.account_id);
    custody::rotate_with_origin(
        &f.state,
        &recovery_admin,
        &f.account.id,
        custody::RotationInput {
            expected_revision: 1,
            credential: zeroize::Zeroizing::new(successor.clone()),
        },
        Some(&server.uri()),
    )
    .await
    .unwrap();
    let saved = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":&service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::services::catalog_credential_record_service::text(&f.state.encryption_keys, &saved)
            .await
            .unwrap()
            .as_str(),
        successor
    );
    let account = db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&f.account.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.catalog_service_id, f.account.catalog_service_id);
    assert_eq!(account.account_id, f.account.account_id);
    // Rotation rechecks dedicated use inside its commit transaction, even when
    // all resource proofs succeed with the same canonical account.
    let reference = id();
    db.collection::<bson::Document>("user_endpoints").insert_one(doc!{"_id":&reference,"catalog_service_id":&service.id,"deleted_at":bson::DateTime::now()}).await.unwrap();
    assert!(
        custody::rotate_with_origin(
            &f.state,
            &recovery_admin,
            &f.account.id,
            custody::RotationInput {
                expected_revision: 1,
                credential: zeroize::Zeroizing::new(successor.clone())
            },
            Some(&server.uri())
        )
        .await
        .is_err()
    );
    db.collection::<bson::Document>("user_endpoints")
        .delete_one(doc! {"_id":reference})
        .await
        .unwrap();
    let bad = format!("AC{}:other-account-secret", Uuid::new_v4().simple());
    assert!(
        custody::rotate_with_origin(
            &f.state,
            &recovery_admin,
            &f.account.id,
            custody::RotationInput {
                expected_revision: 1,
                credential: zeroize::Zeroizing::new(bad)
            },
            Some(&server.uri())
        )
        .await
        .is_err()
    );
    let still = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":&service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.credential_encrypted, saved.credential_encrypted);
    assert_audit_metadata(db, &["managed_credential_rotated"]).await;
}
#[tokio::test]
async fn registration_serializes_custody_and_refuses_legacy_rows_without_claims() {
    let f = fixture().await;
    let db = &f.state.db;
    let mut rental = dummy_service();
    rental.id = f.line.rental_billing_service_id.clone();
    rental.slug = "managed-rental".into();
    rental.created_by = f.admin.user_id.clone();
    rental.visibility = "private".into();
    rental.service_category = "internal".into();
    rental.owner_user_id = None;
    rental.credential_encrypted.clear();
    rental.platform_key = None;
    rental.provider_config_id = None;
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&rental)
        .await
        .unwrap();
    let mut line = f.line.clone();
    line.id = id();
    line.number_resource_id = id();
    line.phone_number = "+18005550129".into();
    line.voice.as_mut().unwrap().application_id = id();
    line.state = LineState::Available;
    line.current_claim_id = None;
    line.next_claim_generation = 1;
    let mut other = line.clone();
    other.id = id();
    other.number_resource_id = id();
    other.phone_number = "+18005550128".into();
    let (a, b) = tokio::join!(
        custody::persist(&f.state, &f.admin, &line, &f.account, &f.service),
        custody::persist(&f.state, &f.admin, &other, &f.account, &f.service)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
            .count_documents(doc! {"_id":{"$in":[&line.id,&other.id]}})
            .await
            .unwrap(),
        1
    );
    let mut unused = line.clone();
    unused.id = id();
    unused.number_resource_id = id();
    unused.phone_number = "+18005550127".into();
    unused.voice.as_mut().unwrap().application_id = id();
    db.collection::<bson::Document>(BINDINGS)
        .insert_one(
            doc! {"_id":id(),"profile":"twilio_relay","resource_id":&unused.number_resource_id},
        )
        .await
        .unwrap();
    let account = db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&f.account.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        custody::persist(&f.state, &f.admin, &unused, &account, &f.service)
            .await
            .is_err()
    );
    assert!(
        db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
            .find_one(doc! {"_id":&unused.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(db.collection::<ResourceClaim>(CLAIMS).find_one(doc!{"identity_key":super::super::session::identity(&["twilio","number_id",&unused.number_resource_id])}).await.unwrap().is_none());
    assert_audit_metadata(db, &["managed_line_registered"]).await;
}

#[tokio::test]
async fn telnyx_registration_requires_trusted_account_exact_url_and_non_null_capacity() {
    use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};
    let f = fixture().await;
    let server = MockServer::start().await;
    let mut account = f.account.clone();
    account.provider = "telnyx".into();
    account.account_id = id();
    let mut line = f.line.clone();
    line.provider = "telnyx".into();
    let mut service = f.service.clone();
    service.auth_method = "bearer".into();
    service.base_url = "https://api.telnyx.com".into();
    let number_path = format!("/v2/phone_numbers/{}", line.number_resource_id);
    let app = line.voice.as_ref().unwrap();
    let app_path = format!("/v2/call_control_applications/{}", app.application_id);
    let voice_path = format!("/v2/phone_numbers/{}/voice", line.number_resource_id);
    let account_path = "/v2/user_profile".to_string();
    service.proxy_operation_policy = Some(ProxyOperationPolicy {
        rules: [&number_path, &app_path, &voice_path, &account_path]
            .into_iter()
            .map(|p| ProxyOperationRule {
                method: "GET".into(),
                path_template: p.clone(),
                ..Default::default()
            })
            .collect(),
    });
    let number = serde_json::json!({"data":{"id":line.number_resource_id,"phone_number":line.phone_number,"status":"active","connection_id":app.application_id}});
    for (account_id, limit, url, valid) in [
        (
            account.account_id.as_str(),
            Some(2),
            custody::stable_url(&f.state.config.base_url, &line.id, false),
            true,
        ),
        (
            account.account_id.as_str(),
            None,
            custody::stable_url(&f.state.config.base_url, &line.id, false),
            false,
        ),
        (
            "different-account",
            Some(2),
            custody::stable_url(&f.state.config.base_url, &line.id, false),
            false,
        ),
        (
            account.account_id.as_str(),
            Some(2),
            "https://untrusted.invalid".into(),
            false,
        ),
    ] {
        for fallback in [
            serde_json::Value::Null,
            serde_json::json!(""),
            serde_json::json!(custody::stable_url(
                &f.state.config.base_url,
                &line.id,
                false
            )),
            serde_json::json!("https://untrusted.invalid/failover"),
        ] {
            let valid = valid && fallback != "https://untrusted.invalid/failover";
            server.reset().await;
            Mock::given(method("GET"))
                .and(path(&account_path))
                .and(wiremock::matchers::header(
                    "authorization",
                    "Bearer fixture-key",
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"data":{"id":42,"user_id":account_id}})),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(&number_path))
                .respond_with(ResponseTemplate::new(200).set_body_json(number.clone()))
                .mount(&server)
                .await;
            Mock::given(method("GET")).and(path(&app_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":{"id":app.application_id,"webhook_event_url":url,"webhook_event_failover_url":fallback,"inbound":{"channel_limit":limit}}}))).mount(&server).await;
            Mock::given(method("GET")).and(path(&voice_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":{"id":line.number_resource_id,"phone_number":line.phone_number,"connection_id":app.application_id}}))).mount(&server).await;
            assert_eq!(
                custody::prove(
                    &f.state,
                    &line,
                    &account,
                    &service,
                    "fixture-key",
                    Some(&server.uri())
                )
                .await
                .is_ok(),
                valid
            );
        }
    }
}

#[tokio::test]
async fn telnyx_sms_registration_requires_number_features_and_exact_messaging_profile() {
    use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};
    let f = fixture().await;
    let server = MockServer::start().await;
    let mut account = f.account.clone();
    account.provider = "telnyx".into();
    account.account_id = id();
    let mut line = f.line.clone();
    line.provider = "telnyx".into();
    line.sms = line.voice.take();
    let mut service = f.service.clone();
    service.auth_method = "bearer".into();
    service.base_url = "https://api.telnyx.com".into();
    let number_path = format!("/v2/phone_numbers/{}", line.number_resource_id);
    let cap = line.sms.as_ref().unwrap();
    let app_path = format!("/v2/messaging_profiles/{}", cap.application_id);
    let messaging_path = format!("{number_path}/messaging");
    let account_path = "/v2/user_profile".to_string();
    service.proxy_operation_policy = Some(ProxyOperationPolicy {
        rules: [&number_path, &app_path, &messaging_path, &account_path]
            .into_iter()
            .map(|p| ProxyOperationRule {
                method: "GET".into(),
                path_template: p.clone(),
                ..Default::default()
            })
            .collect(),
    });
    for (features, profile, status, valid) in [
        (
            serde_json::json!({"sms":{"domestic_two_way":true}}),
            cap.application_id.as_str(),
            "active",
            true,
        ),
        (
            serde_json::Value::Null,
            cap.application_id.as_str(),
            "active",
            false,
        ),
        (
            serde_json::json!({"sms":{"domestic_two_way":false,"international_inbound":true}}),
            cap.application_id.as_str(),
            "active",
            false,
        ),
        (
            serde_json::json!({"sms":{"domestic_two_way":true}}),
            "different-profile",
            "active",
            false,
        ),
        (
            serde_json::json!({"sms":{"domestic_two_way":true}}),
            cap.application_id.as_str(),
            "provision-pending",
            false,
        ),
    ] {
        for fallback in [
            serde_json::Value::Null,
            serde_json::json!(""),
            serde_json::json!(custody::stable_url(
                &f.state.config.base_url,
                &line.id,
                true
            )),
            serde_json::json!("https://untrusted.invalid/failover"),
        ] {
            let valid = valid && fallback != "https://untrusted.invalid/failover";
            server.reset().await;
            Mock::given(method("GET"))
                .and(path(&account_path))
                .and(wiremock::matchers::header(
                    "authorization",
                    "Bearer fixture-key",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"data":{"id":42,"user_id":account.account_id}}),
                ))
                .mount(&server)
                .await;
            Mock::given(method("GET")).and(path(&number_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":{"id":line.number_resource_id,"phone_number":line.phone_number,"status":status,"messaging_profile_id":cap.application_id}}))).mount(&server).await;
            Mock::given(method("GET")).and(path(&app_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":{"id":cap.application_id,"webhook_failover_url":fallback,"webhook_url":custody::stable_url(&f.state.config.base_url,&line.id,true)}}))).mount(&server).await;
            Mock::given(method("GET")).and(path(&messaging_path)).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":{"id":line.number_resource_id,"phone_number":line.phone_number,"messaging_profile_id":profile,"features":features}}))).mount(&server).await;
            assert_eq!(
                custody::prove(
                    &f.state,
                    &line,
                    &account,
                    &service,
                    "fixture-key",
                    Some(&server.uri())
                )
                .await
                .is_ok(),
                valid
            );
        }
    }
}

/// Persisted audit payloads may contain identifiers and state metadata only.
async fn assert_audit_metadata(db: &Database, required: &[&str]) {
    use futures::TryStreamExt;
    let rows: Vec<crate::models::audit_log::AuditLog> = db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":{"$regex":"^managed_"}})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for event in required {
        assert!(
            rows.iter().any(|row| row.event_type == *event),
            "Missing {event}"
        );
    }
    for row in rows {
        let keys: &[&str] = match row.event_type.as_str() {
            "managed_eligibility_saved" => {
                &["eligibility_id", "revision", "owner_id", "terms_changed"]
            }
            "managed_terms_accepted" => &["revision"],
            "managed_eligibility_applied" => &["eligibility_id", "revision"],
            "managed_eligibility_revoked" => &["owner_id", "revision"],
            "managed_line_registered" => &["line_id", "trusted_account_id", "has_voice", "has_sms"],
            "managed_credential_rotated" => {
                &["trusted_account_id", "catalog_service_id", "proved_lines"]
            }
            _ => panic!("Unexpected managed audit event"),
        };
        let data = row.event_data.as_ref().unwrap().as_object().unwrap();
        assert_eq!(data.len(), keys.len());
        assert!(keys.iter().all(|key| data.contains_key(*key)));
        assert!(row.seq.is_some_and(|seq| seq > 0));
        assert_eq!(row.prev_hash.as_ref().unwrap().len(), 64);
        assert_eq!(row.entry_hash.as_ref().unwrap().len(), 64);
    }
}

async fn insert_rental(f: &Fixture) {
    let mut rental = dummy_service();
    rental.id = f.line.rental_billing_service_id.clone();
    rental.slug = "managed-rental".into();
    rental.created_by = f.admin.user_id.clone();
    rental.owner_user_id = None;
    rental.visibility = "private".into();
    rental.service_category = "internal".into();
    rental.credential_encrypted.clear();
    rental.platform_key = None;
    rental.provider_config_id = None;
    f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(rental)
        .await
        .unwrap();
}

#[tokio::test]
async fn designation_refuses_public_or_referenced_catalog_rows_including_tombstones() {
    let f = fixture().await;
    insert_rental(&f).await;
    let db = &f.state.db;
    let services =
        db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME);
    let mut line = f.line.clone();
    line.id = id();
    line.number_resource_id = id();
    line.phone_number = "+18005550192".into();
    line.voice.as_mut().unwrap().application_id = id();
    line.current_claim_id = None;
    line.state = LineState::Available;
    for service in [
        &f.service.id,
        &f.line.voice.as_ref().unwrap().billing_service_id,
        &f.line.rental_billing_service_id,
    ] {
        services
            .update_one(doc! {"_id":service}, doc! {"$set":{"visibility":"public"}})
            .await
            .unwrap();
        let reference = id();
        db.collection::<bson::Document>("user_services").insert_one(doc!{"_id":&reference,"catalog_service_id":service,"enabled":false,"deleted_at":bson::DateTime::now()}).await.unwrap();
        assert!(
            custody::persist(&f.state, &f.admin, &line, &f.account, &f.service)
                .await
                .is_err()
        );
        services
            .update_one(doc! {"_id":service}, doc! {"$set":{"visibility":"private"}})
            .await
            .unwrap();
        // A private row with disabled/tombstoned references is still in use.
        assert!(
            custody::persist(&f.state, &f.admin, &line, &f.account, &f.service)
                .await
                .is_err()
        );
        db.collection::<bson::Document>("user_services")
            .delete_one(doc! {"_id":reference})
            .await
            .unwrap();
    }
    // Cover the independent legacy/credential/binding collections as well.
    for (collection, field) in [
        (
            crate::models::user_endpoint::COLLECTION_NAME,
            "catalog_service_id",
        ),
        (
            crate::models::user_service_connection::COLLECTION_NAME,
            "service_id",
        ),
        (
            crate::models::node_service_binding::COLLECTION_NAME,
            "service_id",
        ),
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            "grants.platform_service_ids",
        ),
        (
            crate::models::api_key::COLLECTION_NAME,
            "allowed_platform_service_ids",
        ),
        (
            crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME,
            "catalog_service_id",
        ),
    ] {
        let mut row = doc! {"_id":id(),"enabled":false,"deleted_at":bson::DateTime::now()};
        if field == "grants.platform_service_ids" {
            row.insert("grants", doc! {"platform_service_ids":[&f.service.id]});
        } else if field == "allowed_platform_service_ids" {
            row.insert(field, vec![&f.service.id]);
        } else {
            row.insert(field, &f.service.id);
        }
        db.collection::<bson::Document>(collection)
            .insert_one(&row)
            .await
            .unwrap();
        assert!(
            custody::persist(&f.state, &f.admin, &line, &f.account, &f.service)
                .await
                .is_err(),
            "{collection}"
        );
        db.collection::<bson::Document>(collection)
            .delete_one(doc! {"_id":row.get_str("_id").unwrap()})
            .await
            .unwrap();
    }
    custody::persist(&f.state, &f.admin, &line, &f.account, &f.service)
        .await
        .unwrap();
    assert!(
        db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
            .find_one(doc! {"_id":line.id})
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn new_managed_admissions_and_configuration_require_readiness() {
    use crate::errors::AppError;
    let db = connect_transaction_test_database("managed_unready_writes").await;
    let state = test_app_state(db.clone());
    let actor = AuditActor::from_auth_user(&test_auth_user(&id()));
    let owner = id();
    let fixture = fixture().await;
    let terms = eligibility_input(None).required_terms;
    let assert_unready = |result: Result<(), AppError>| {
        assert!(matches!(
            result,
            Err(AppError::SecurityReadinessUnavailable)
        ))
    };
    assert_unready(
        eligibility::save(&db, &actor, &owner, eligibility_input(None), &[7; 32])
            .await
            .map(|_| ()),
    );
    assert_unready(eligibility::accept_terms(&db, &actor, 1, terms, &[7; 32]).await);
    assert_unready(eligibility::apply_revision(&db, &actor, &owner, 1, &[7; 32]).await);
    assert_unready(eligibility::revoke(&state, &actor, &owner, 1, &[7; 32]).await);
    assert_unready(
        custody::persist(
            &state,
            &actor,
            &fixture.line,
            &fixture.account,
            &fixture.service,
        )
        .await,
    );
    assert_unready(eligibility::require_admin(&db, &actor).await);
    let registration = custody::RegistrationInput {
        standard_voice_options: vec![],
        line_id: id(),
        trusted_account_id: id(),
        phone_number: "+18005550191".into(),
        country: "US".into(),
        number_resource_id: id(),
        voice: None,
        sms: None,
        rental_billing_service_id: id(),
        regulatory: custody::RegulatoryInput {
            requirement_reference: "fixture".into(),
            registration_status: "approved".into(),
            sms_sender_registration: None,
        },
        supplier_tariffs: vec![],
        call_slot_limit: 1,
        calls_per_minute: 1,
        sms_per_minute: 1,
    };
    assert_unready(
        custody::register(&state, &actor, registration)
            .await
            .map(|_| ()),
    );
    assert_unready(
        custody::rotate(
            &state,
            &actor,
            &id(),
            custody::RotationInput {
                expected_revision: 1,
                credential: zeroize::Zeroizing::new("fixture".into()),
            },
        )
        .await,
    );
    let budget = ManagedBudget {
        id: id(),
        budget_key: id(),
        window: BudgetWindow::RollingThirtyDays,
        limit: Credits::from_whole(1),
        revision: 1,
        fence: 1,
    };
    assert_unready(
        hold(&db, &budget, "effect", 1, Utc::now())
            .await
            .map(|_| ()),
    );

    for collection in [
        ManagedCommsEligibility::COLLECTION_NAME,
        ManagedPhoneLine::COLLECTION_NAME,
        ManagedBudgetEntry::COLLECTION_NAME,
        "audit_logs",
    ] {
        assert_eq!(
            db.collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn apply_revision_updates_suspended_claims_without_resuming_them() {
    let f = fixture().await;
    let db = &f.state.db;
    db.collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.claim.id},
            doc! {"$set":{"state":"suspended","suspension":"unpaid"}},
        )
        .await
        .unwrap();
    eligibility::save(db, &f.admin, &f.owner, eligibility_input(Some(1)), &[7; 32])
        .await
        .unwrap();
    eligibility::apply_revision(db, &f.admin, &f.owner, 2, &[7; 32])
        .await
        .unwrap();
    let claim = db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.claim.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim.eligibility_revision, 2);
    assert_eq!(claim.state, LineState::Suspended);
    assert_eq!(claim.accepted_terms, f.claim.accepted_terms);
}

fn admitted(
    f: &Fixture,
    b: &Binding,
    call: Option<&str>,
    message: Option<&str>,
) -> AdmittedReference {
    AdmittedReference {
        trusted_account_id: f.account.id.clone(),
        credential_service_id: f.service.id.clone(),
        provider: f.account.provider.clone(),
        account_id: f.account.account_id.clone(),
        line_id: f.line.id.clone(),
        number_resource_id: b.resource_id.clone(),
        application_id: b.application_id.clone(),
        provider_call_id: call.map(Into::into),
        provider_message_id: message.map(Into::into),
        call_leg_id: None,
        call_session_id: None,
    }
}
async fn managed_binding(f: &Fixture, profile: Profile) -> Binding {
    let now = Utc::now();
    let b = Binding {
        id: id(),
        user_id: f.owner.clone(),
        profile,
        contract_version: 1,
        revision: 1,
        label: "managed".into(),
        agent_id: f.claim.agent_id.clone(),
        credential: f.credential.clone(),
        account_id: f.account.account_id.clone(),
        resource_id: f.line.number_resource_id.clone(),
        phone_number: f.line.phone_number.clone(),
        application_id: f.line.voice.as_ref().unwrap().application_id.clone(),
        webhook_public_key: f.account.webhook_public_key.clone(),
        voice: super::voice::default_option(super::super::profiles::provider(profile)).voice,
        language: "en-US".into(),
        enabled: true,
        verified_credential_identity: super::super::session::identity(&[
            "managed-canonical",
            &f.account.id,
            &f.service.id,
        ]),
        trusted_account_revision: Some(1),
        verified_at: now,
        created_at: now,
        updated_at: now,
    };
    f.state
        .db
        .collection::<Binding>(BINDINGS)
        .insert_one(&b)
        .await
        .unwrap();
    b
}
async fn admitted_session(f: &Fixture, b: &Binding) -> CallSession {
    let now = bson::DateTime::now();
    let call = format!("CA{}", Uuid::new_v4().simple());
    let reference = admitted(f, b, Some(&call), None);
    let mut row:CallSession=bson::from_document(doc!{
        "_id":id(),"identity_key":super::super::session::identity(&[super::super::profiles::provider(b.profile),&b.account_id,&call]),"user_id":&b.user_id,"binding_id":&b.id,"binding_revision":1_i64,
        "managed_claim_id":&f.claim.id,"managed_claim_generation":1_i64,"admitted_reference":bson::to_bson(&reference).unwrap(),"profile":bson::to_bson(&b.profile).unwrap(),"contract_version":1,"credential_identity":&b.verified_credential_identity,"provider_call_id":call,
        "from":"+18005550999","to":&b.phone_number,"conversation_id":format!("nyxa-{}",Uuid::new_v4().simple()),"state":"active","claim_id":id(),"generation":1_i64,"next_utterance":0_i64,"billing_request_id":id(),"observed_seconds":0_i64,"billing_finalized":false,"delivery":"generated_not_confirmed","created_at":now,"connected_at":now,"lease_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10)),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(30))
    }).unwrap();
    row.billing_request_id = format!("managed-call:{}:base", row.id);
    f.state
        .db
        .collection::<CallSession>(SESSIONS)
        .insert_one(&row)
        .await
        .unwrap();
    // This authority-only fixture carries a pre-settled prefix. M4's effects
    // fixtures exercise real reservation/ledger settlement and missing rows.
    use crate::models::{
        service_billing::BillingMetric,
        usage_meter::{BillingLayer, CredentialClass, UsageFunding, UsageMeterRow, UsageStatus},
    };
    let mut snapshot = rate(&f.line.voice.as_ref().unwrap().billing_service_id);
    snapshot.metric = BillingMetric::VoiceSeconds;
    let context = crate::services::billing::BillingRouteContext::new(
        crate::services::billing::BillingIngress::ChannelOutbound,
        row.billing_request_id.clone(),
        f.owner.clone(),
        f.owner.clone(),
        None,
        None,
        Some(snapshot.service_id.clone()),
        Some(snapshot.service_slug.clone()),
        crate::services::billing::NodeIntent::Direct,
        "basic".into(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::VoiceSeconds,
        None,
        false,
    );
    let mut usage = crate::services::billing::meter::test_reserved_row(
        &context,
        BillingLayer::Platform,
        BillingMetric::VoiceSeconds,
        snapshot.lago_metric_code.clone(),
        None,
        None,
    );
    usage.forwarded = true;
    usage.released = true;
    usage.status = UsageStatus::Finalized;
    usage.quantity = Some(12);
    usage.funding = Some(UsageFunding {
        settlement_rate: Some(snapshot.rate),
        settled: true,
        ..Default::default()
    });
    let admission = ManagedCallAdmission {
        id: id(),
        session_id: row.id.clone(),
        managed_claim_id: f.claim.id.clone(),
        managed_claim_generation: 1,
        admitted: reference,
        base: MeterAdmission {
            billing_request_id: row.billing_request_id.clone(),
            usage_row_ids: vec![usage.id.clone()],
            rate: snapshot,
            reserved_quantity: 1800,
        },
        premium: None,
        underwriting_snapshot: None,
        underwriting_hold_id: None,
        spend_hold_id: None,
        slot_held: true,
        financially_finalized: false,
        answer_dispatch_anchor: Some(Utc::now()),
        hangup_deadline: Some(Utc::now() + Duration::minutes(29)),
        created_at: Utc::now(),
    };
    f.state
        .db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .insert_one(usage)
        .await
        .unwrap();
    f.state
        .db
        .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
        .insert_one(admission)
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).insert_one(doc!{"_id":&row.conversation_id,"user_id":&f.owner,"title":"managed fixture","model":"fixture","credential_api_key_id":id(),"message_count":0_i64,"created_at":now,"updated_at":now,"active_turn":{"turn_id":id(),"started_at":now,"heartbeat_at":now}}).await.unwrap();
    row
}
fn signed_twilio(f: &Fixture, b: &Binding, body: &str) -> axum::http::HeaderMap {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(super::super::coordinator::webhook_url(&f.state, b).as_bytes());
    for (key, value) in protocol::form(body.as_bytes()).unwrap() {
        mac.update(key.as_bytes());
        mac.update(value.as_bytes());
    }
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "x-twilio-signature",
        base64::engine::general_purpose::STANDARD
            .encode(mac.finalize().into_bytes())
            .parse()
            .unwrap(),
    );
    headers
}
async fn make_managed_unready(db: &Database) {
    let replicas =
        db.collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME);
    let mut stale = replicas.find_one(doc! {}).await.unwrap().unwrap();
    stale.insert("_id", id());
    stale.insert("generation_id", "unupgraded-fixture-generation");
    stale.insert("capabilities", Vec::<String>::new());
    stale.insert("observed_at", bson::DateTime::now());
    replicas.insert_one(stale).await.unwrap();
    assert!(super::require_ready(db).await.is_err());
}
async fn revoke_lapse_release_and_flag_off(f: &Fixture) {
    let db = &f.state.db;
    eligibility::revoke(&f.state, &f.admin, &f.owner, 1, &[7; 32])
        .await
        .unwrap();
    db.collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.claim.id}, doc! {"$set":{"state":"cooldown"}})
        .await
        .unwrap();
    db.collection::<bson::Document>(ManagedLinePeriod::COLLECTION_NAME).update_one(doc!{"_id":&f.period.id},doc!{"$set":{"covered_until":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    flags::set_platform_override(
        db,
        flags::MANAGED_LINES_FLAG_KEY,
        &flags::FlagTarget::User(f.owner.clone()),
        false,
        &f.admin.user_id,
    )
    .await
    .unwrap();
    assert!(admission::read(db, &f.owner, &f.credential).await.is_err());
}
#[tokio::test]
async fn signed_terminal_callback_survives_revocation_lapse_release_and_flag_off() {
    let f = fixture().await;
    enable(&f).await;
    let b = managed_binding(&f, Profile::TwilioRelay).await;
    let s = admitted_session(&f, &b).await;
    // No provider call is necessary for signed terminal evidence; revocation's
    // immediate cleanup uses a failing loopback fixture and leaves recovery due.
    let server = MockServer::start().await;
    super::super::profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), server.uri());
    revoke_lapse_release_and_flag_off(&f).await;
    make_managed_unready(&f.state.db).await;
    let body = format!(
        "AccountSid={}&CallSid={}&CallStatus=completed&CallDuration=12&From={}&To={}",
        b.account_id,
        s.provider_call_id,
        urlencoding::encode(&s.from),
        urlencoding::encode(&s.to)
    );
    let headers = signed_twilio(&f, &b, &body);
    super::super::coordinator::ingress(f.state.clone(), &b.id, &headers, body.as_bytes())
        .await
        .unwrap();
    let saved = super::super::session::load(&f.state, &s.id).await.unwrap();
    assert_eq!(saved.state, SessionState::Closed);
    assert_eq!(saved.observed_seconds, 12);
    assert!(saved.billing_finalized);
    // A signed new call still requires the full gate and creates no session.
    let body = format!(
        "AccountSid={}&CallSid=CAnew&CallStatus=ringing&Direction=inbound&From={}&To={}",
        b.account_id,
        urlencoding::encode(&s.from),
        urlencoding::encode(&s.to)
    );
    let headers = signed_twilio(&f, &b, &body);
    assert!(
        super::super::coordinator::ingress(f.state.clone(), &b.id, &headers, body.as_bytes())
            .await
            .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<CallSession>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn signed_sms_receipt_survives_revocation_lapse_release_and_flag_off() {
    let f = fixture().await;
    enable(&f).await;
    let b = managed_binding(&f, Profile::TwilioSms).await;
    let message = format!("SM{}", Uuid::new_v4().simple());
    let send_id = id();
    let reference = admitted(&f, &b, None, Some(&message));
    f.state.db.collection::<bson::Document>(EVENTS).insert_one(doc!{"_id":&send_id,"identity_key":id(),"user_id":&f.owner,"binding_id":&b.id,"binding_revision":1_i64,"managed_claim_id":&f.claim.id,"managed_claim_generation":1_i64,"admitted_reference":bson::to_bson(&reference).unwrap(),"event_kind":"sms_send","status":"accepted","provider_message_id":&message,"delivery":"accepted","observed_parts":1_i64,"billing_finalized":true,"from":&b.phone_number,"to":"+18005550999","created_at":bson::DateTime::now()}).await.unwrap();
    revoke_lapse_release_and_flag_off(&f).await;
    make_managed_unready(&f.state.db).await;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("AccountSid", b.account_id.as_str()),
            ("MessageSid", message.as_str()),
            ("MessageStatus", "delivered"),
            ("From", b.phone_number.as_str()),
            ("To", "+18005550999"),
            ("NumSegments", "1"),
        ])
        .finish();
    let headers = signed_twilio(&f, &b, &body);
    super::super::coordinator::ingress(f.state.clone(), &b.id, &headers, body.as_bytes())
        .await
        .unwrap();
    let receipt = f
        .state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"event_kind":"sms_receipt"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.delivery, "delivered");
    assert_eq!(
        receipt.managed_claim_id.as_deref(),
        Some(f.claim.id.as_str())
    );
    assert!(receipt.admitted_reference.is_some());
    let mut bad = headers;
    bad.insert("x-twilio-signature", "invalid".parse().unwrap());
    assert!(
        super::super::coordinator::ingress(f.state.clone(), &b.id, &bad, body.as_bytes())
            .await
            .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<EventReceipt>(EVENTS)
            .count_documents(doc! {"event_kind":"sms_receipt"})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn revocation_requests_live_session_termination_and_sweep_completes_cleanup() {
    let f = fixture().await;
    enable(&f).await;
    let b = managed_binding(&f, Profile::TwilioRelay).await;
    let s = admitted_session(&f, &b).await;
    let server = MockServer::start().await;
    super::super::profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), server.uri());
    // Immediate shutdown is uncertain. The durable expired lease must remain.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    eligibility::revoke(&f.state, &f.admin, &f.owner, 1, &[7; 32])
        .await
        .unwrap();
    let pending = super::super::session::load(&f.state, &s.id).await.unwrap();
    let conversation = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id":&s.conversation_id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        conversation
            .get_document("active_turn")
            .unwrap()
            .get_bool("stop_requested")
            .unwrap()
    );
    assert!(pending.generation > s.generation);
    assert!(pending.lease_expires_at <= Utc::now());
    assert_eq!(pending.state, SessionState::Uncertain);
    assert!(
        super::super::session::expired(&f.state)
            .await
            .unwrap()
            .iter()
            .any(|row| row.id == s.id)
    );
    server.reset().await;
    make_managed_unready(&f.state.db).await;
    assert!(super::require_ready(&f.state.db).await.is_err());
    Mock::given(method("POST"))
        .and(wiremock::matchers::header(
            "content-type",
            "application/x-www-form-urlencoded",
        ))
        .and(wiremock::matchers::body_string("Status=completed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"sid":s.provider_call_id,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":s.from,"to":s.to,"status":"completed","duration":"12","end_time":Utc::now().to_rfc2822()}))).expect(1).mount(&server).await;
    super::super::coordinator::recover_once(&f.state)
        .await
        .unwrap();
    let saved = super::super::session::load(&f.state, &s.id).await.unwrap();
    assert_eq!(saved.state, SessionState::Closed);
    assert!(saved.billing_finalized);
    assert_eq!(saved.observed_seconds, 12);
    server.verify().await;
}

#[test]
fn cleanup_targets_type_twilio_form_and_telnyx_json_and_exact_reads() {
    let mut reference = AdmittedReference {
        trusted_account_id: id(),
        credential_service_id: id(),
        provider: "twilio".into(),
        account_id: format!("AC{}", Uuid::new_v4().simple()),
        line_id: id(),
        number_resource_id: id(),
        application_id: id(),
        provider_call_id: Some("CAfixture".into()),
        provider_message_id: None,
        call_leg_id: None,
        call_session_id: None,
    };
    let target = resolver::cleanup_request(&reference, resolver::CleanupOperation::Hangup).unwrap();
    assert_eq!(target.method, "POST");
    assert_eq!(target.url.host_str(), Some("api.twilio.com"));
    assert!(
        matches!(target.body,Some(resolver::CleanupBody::Form(ref fields)) if fields==&[("Status","completed")])
    );
    assert!(
        resolver::cleanup_request(&reference, resolver::CleanupOperation::ReadCall)
            .unwrap()
            .body
            .is_none()
    );
    reference.provider = "telnyx".into();
    reference.account_id = id();
    reference.provider_call_id = Some("v3:fixture".into());
    let target = resolver::cleanup_request(&reference, resolver::CleanupOperation::Hangup).unwrap();
    assert_eq!(target.method, "POST");
    assert_eq!(target.url.host_str(), Some("api.telnyx.com"));
    assert!(
        matches!(target.body,Some(resolver::CleanupBody::Json(ref value)) if value["command_id"].as_str().is_some_and(|id|Uuid::parse_str(id).is_ok()))
    );
    assert!(
        resolver::cleanup_request(&reference, resolver::CleanupOperation::ReadCall)
            .unwrap()
            .body
            .is_none()
    );
}

#[tokio::test]
async fn signed_telnyx_answer_observation_survives_revocation_without_starting_relay() {
    use base64::Engine;
    use ed25519_dalek::Signer;
    let mut f = fixture().await;
    enable(&f).await;
    let signer = ed25519_dalek::SigningKey::from_bytes(&[31; 32]);
    f.account.provider = "telnyx".into();
    f.account.account_id = id();
    f.account.webhook_public_key =
        Some(base64::engine::general_purpose::STANDARD.encode(signer.verifying_key().to_bytes()));
    f.service.base_url = "https://api.telnyx.com".into();
    f.service.auth_method = "bearer".into();
    f.service.credential_encrypted = crate::services::catalog_credential_record_service::encrypt(
        &f.state.encryption_keys,
        &f.service.id,
        &f.service.created_by,
        b"fixture-key",
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .replace_one(doc! {"_id":&f.account.id}, &f.account)
        .await
        .unwrap();
    f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.service.id}, &f.service)
        .await
        .unwrap();
    let b = managed_binding(&f, Profile::TelnyxRelay).await;
    let session = admitted_session(&f, &b).await;
    let server = MockServer::start().await;
    super::super::profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), server.uri());
    revoke_lapse_release_and_flag_off(&f).await;
    make_managed_unready(&f.state.db).await;
    let body=serde_json::json!({"data":{"id":id(),"event_type":"call.answered","occurred_at":Utc::now().to_rfc3339(),"payload":{"call_control_id":session.provider_call_id,"connection_id":b.application_id}}}).to_string();
    let timestamp = Utc::now().timestamp().to_string();
    let signature = base64::engine::general_purpose::STANDARD.encode(
        signer
            .sign(format!("{timestamp}|{body}").as_bytes())
            .to_bytes(),
    );
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("telnyx-timestamp", timestamp.parse().unwrap());
    headers.insert("telnyx-signature-ed25519", signature.parse().unwrap());
    super::super::coordinator::ingress(f.state.clone(), &b.id, &headers, body.as_bytes())
        .await
        .unwrap();
    let saved = super::super::session::load(&f.state, &session.id)
        .await
        .unwrap();
    assert!(saved.answered);
    assert!(!saved.relay_dispatched);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| !request.url.path().ends_with("conversation_relay_start"))
    );
}

#[tokio::test]
async fn byok_twilio_unavailable_connection_keeps_verification_error_before_typed_parsing() {
    let f = fixture().await;
    let mut b = managed_binding(&f, Profile::TwilioRelay).await;
    b.credential = CredentialBinding::Connection {
        user_service_id: id(),
    };
    f.state
        .db
        .collection::<Binding>(BINDINGS)
        .replace_one(doc! {"_id":&b.id}, &b)
        .await
        .unwrap();
    // Missing fields would fail typed parsing; the unavailable auth mapping wins.
    let error = super::super::coordinator::ingress(
        f.state.clone(),
        &b.id,
        &axum::http::HeaderMap::new(),
        b"CallSid=forged",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error,crate::errors::AppError::ChannelWebhookVerificationFailed(ref message) if message=="Provider authentication unavailable")
    );
}
#[tokio::test]
async fn byok_unsigned_or_forged_body_is_verified_before_typed_parsing() {
    let f = fixture().await;
    let mut b = managed_binding(&f, Profile::TwilioRelay).await;
    b.credential = CredentialBinding::Platform {
        catalog_service_id: f.service.id.clone(),
    };
    // This ordinary internal row has an explicit public platform key and can
    // resolve its credential, but receives no valid provider signature.
    f.state
        .db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.service.id},
            doc! {"$unset":{"managed_comms_role":""},"$set":{"visibility":"public","platform_key":bson::to_bson(&crate::models::downstream_service::PlatformKeyConfig {
                enabled:true,
                audience:crate::models::downstream_service::PlatformKeyAudience::Public,
                ..Default::default()
            }).unwrap()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Binding>(BINDINGS)
        .replace_one(doc! {"_id":&b.id}, &b)
        .await
        .unwrap();
    super::super::binding::resolve(&f.state, &b.user_id, &b.credential, b.profile)
        .await
        .expect("ordinary platform fixture credential resolves before signature verification");
    for signature in [None, Some("forged")] {
        let mut headers = axum::http::HeaderMap::new();
        if let Some(s) = signature {
            headers.insert("x-twilio-signature", s.parse().unwrap());
        }
        let error = super::super::coordinator::ingress(
            f.state.clone(),
            &b.id,
            &headers,
            b"CallSid=invalid",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                error,
                crate::errors::AppError::ChannelWebhookVerificationFailed(_)
            ),
            "{error:?}"
        );
        assert!(!error.to_string().contains("Call resource"));
        assert!(
            !error
                .to_string()
                .contains("Provider authentication unavailable")
        );
    }
}
#[tokio::test]
async fn managed_socket_rejects_different_call_account_and_completed_call() {
    let f = fixture().await;
    let b = managed_binding(&f, Profile::TwilioRelay).await;
    let s = admitted_session(&f, &b).await;
    let valid = serde_json::json!({"sid":s.provider_call_id,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":s.from,"to":s.to,"direction":"inbound","status":"in-progress"});
    assert!(super::super::carrier::validate_managed_socket_call(&b, &s, &valid).is_ok());
    for (field, value) in [
        (
            "sid",
            serde_json::json!(format!("CA{}", Uuid::new_v4().simple())),
        ),
        (
            "account_sid",
            serde_json::json!(format!("AC{}", Uuid::new_v4().simple())),
        ),
        ("status", serde_json::json!("completed")),
        ("phone_number_sid", serde_json::json!("another-number")),
        ("from", serde_json::json!("+18005550998")),
        ("to", serde_json::json!("+18005550998")),
        ("direction", serde_json::json!("outbound-api")),
    ] {
        let mut v = valid.clone();
        v[field] = value;
        assert!(
            super::super::carrier::validate_managed_socket_call(&b, &s, &v).is_err(),
            "{field}"
        );
    }
    let mut telnyx = b.clone();
    telnyx.profile = Profile::TelnyxRelay;
    let v = serde_json::json!({"data":{"call_control_id":s.provider_call_id,"account_id":b.account_id,"connection_id":b.application_id,"phone_number_id":b.resource_id,"from":s.from,"to":s.to,"direction":"incoming","is_alive":true}});
    assert!(super::super::carrier::validate_managed_socket_call(&telnyx, &s, &v).is_ok());
    for field in [
        "call_control_id",
        "account_id",
        "phone_number_id",
        "connection_id",
        "to",
        "from",
    ] {
        let mut bad = v.clone();
        bad["data"][field] = serde_json::json!("wrong");
        assert!(super::super::carrier::validate_managed_socket_call(&telnyx, &s, &bad).is_err());
    }
    let mut dead = v;
    dead["data"]["is_alive"] = serde_json::json!(false);
    assert!(super::super::carrier::validate_managed_socket_call(&telnyx, &s, &dead).is_err());
}

#[tokio::test]
async fn managed_socket_provider_read_refuses_wrong_identity_and_terminal_state() {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256};
    let f = fixture().await;
    let b = managed_binding(&f, Profile::TwilioRelay).await;
    let mut s = admitted_session(&f, &b).await;
    let raw = "a".repeat(64);
    s.state = SessionState::Prepared;
    s.claim_id = None;
    s.socket_ticket_hash = Some(hex::encode(Sha256::digest(raw.as_bytes())));
    f.state
        .db
        .collection::<CallSession>(SESSIONS)
        .replace_one(doc! {"_id":&s.id}, &s)
        .await
        .unwrap();
    let socket_url = format!(
        "{}/socket/{}?ticket={raw}",
        super::super::coordinator::webhook_url(&f.state, &b),
        s.id
    )
    .replacen("https://", "wss://", 1)
    .replacen("http://", "ws://", 1);
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(socket_url.as_bytes());
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "x-twilio-signature",
        base64::engine::general_purpose::STANDARD
            .encode(mac.finalize().into_bytes())
            .parse()
            .unwrap(),
    );
    let server = MockServer::start().await;
    super::super::profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), server.uri());
    let valid = serde_json::json!({"sid":s.provider_call_id,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":s.from,"to":s.to,"direction":"inbound","status":"in-progress"});
    for (field, value) in [
        ("sid", "other-call"),
        ("account_sid", "other-account"),
        ("status", "completed"),
    ] {
        let mut bad = valid.clone();
        bad[field] = serde_json::json!(value);
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(bad))
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            super::super::coordinator::authenticate_socket(&f.state, &b, &s, &headers, &raw)
                .await
                .is_err(),
            "{field}"
        );
    }
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(valid))
        .expect(1)
        .mount(&server)
        .await;
    super::super::coordinator::authenticate_socket(&f.state, &b, &s, &headers, &raw)
        .await
        .unwrap();
}

#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
