//! M3/M4 transactions use real mc0 funding and controlled provider evidence.
use super::*;
use crate::errors::AppError;
use crate::models::service_billing::BillingMetric;
use crate::services::call_channels::{
    binding, coordinator,
    managed::{calls, ingress, sms, underwriting},
    profiles, session, sms_delivery,
};
use axum::http::HeaderMap;
use chrono::Timelike;
use futures::TryStreamExt;
use serde_json::json;

fn occurrence() -> ingress::EventIdentity {
    ingress::EventIdentity {
        occurred_at: Utc::now(),
        occurrence_verified: true,
        deliverable: true,
        call_leg_id: None,
        call_session_id: None,
    }
}
async fn setup_call() -> (LifecycleFixture, Binding) {
    let t = setup().await;
    // Lifecycle-only fixtures need just the specialist identity. Live effect
    // dispatch loads the typed agent and must exercise its real validation.
    let now = bson::DateTime::now();
    t.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.claim.agent_id},
            doc! {"$set":{"name":"fixture-specialist","created_by":"user","model":"fixture","created_at":now,"updated_at":now}},
        )
        .await
        .unwrap();
    let r = claim(&t).await;
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(
            doc! {},
            doc! {"$set":{"balance_credits":Credits::from_whole(10000)}},
        )
        .await
        .unwrap();
    let c = saved_claim(&t, r.claim_id.as_deref().unwrap()).await;
    let b = binding::load(&t.f.state, c.voice_binding_id.as_deref().unwrap())
        .await
        .unwrap();
    profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), t.server.uri());
    binding::live(&t.f.state, &b).await.unwrap();
    (t, b)
}
async fn prepare(t: &LifecycleFixture, b: &Binding) -> CallSession {
    calls::prepare(
        &t.f.state,
        b,
        &format!("CA{}", Uuid::new_v4().simple()),
        "+18005550999",
        &b.phone_number,
        Some(&occurrence()),
    )
    .await
    .unwrap()
    .0
}
fn sign(t: &LifecycleFixture, sms: bool, body: &str) -> HeaderMap {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(ingress::url(&t.f.state, &t.f.line.id, sms).as_bytes());
    for (k, v) in protocol::form(body.as_bytes()).unwrap() {
        mac.update(k.as_bytes());
        mac.update(v.as_bytes());
    }
    let mut h = HeaderMap::new();
    h.insert(
        "x-twilio-signature",
        base64::engine::general_purpose::STANDARD
            .encode(mac.finalize().into_bytes())
            .parse()
            .unwrap(),
    );
    h
}
fn call_body(b: &Binding, call: &str, time: chrono::DateTime<Utc>) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("AccountSid", b.account_id.as_str()),
            ("CallSid", call),
            ("CallStatus", "ringing"),
            ("Direction", "inbound"),
            ("From", "+18005550999"),
            ("To", b.phone_number.as_str()),
            ("Timestamp", &time.to_rfc3339()),
        ])
        .finish()
}
#[tokio::test]
async fn atomic_call_receipt_duplicate_slots_and_required_rates() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let r = calls::receipt(&t.f.state, &s.id).await.unwrap();
    assert_eq!(
        r.base.billing_request_id,
        format!("managed-call:{}:base", s.id)
    );
    assert_eq!(r.base.reserved_quantity, 1800);
    assert!(r.premium.is_none());
    let duplicate = calls::prepare(
        &t.f.state,
        &b,
        &s.provider_call_id,
        &s.from,
        &s.to,
        Some(&occurrence()),
    )
    .await
    .unwrap();
    assert!(!duplicate.1);
    assert_eq!(duplicate.0.id, s.id);
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    assert!(
        calls::prepare(
            &t.f.state,
            &b,
            "CAsecond",
            "+18005550999",
            &b.phone_number,
            Some(&occurrence())
        )
        .await
        .is_err()
    );
    t.f.state
        .db
        .collection::<UsageMeterRow>(USAGE)
        .delete_one(doc! {"_id":&r.base.usage_row_ids[0]})
        .await
        .unwrap();
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(10), "fixture-provider")
        .await
        .unwrap();
    assert!(calls::finalize(&t.f.state, &ended).await.is_err());
    assert!(
        !session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .billing_finalized
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
}
#[tokio::test]
async fn two_replica_admission_race_and_line_rate_are_atomic() {
    let (t, b) = setup_call().await;
    let peer = separate_client_state(&t, &format!("managed-call-peer-{}", id())).await;
    let a = occurrence();
    let z = occurrence();
    let (x, y) = tokio::join!(
        calls::prepare(
            &t.f.state,
            &b,
            "CAone",
            "+18005550999",
            &b.phone_number,
            Some(&a)
        ),
        calls::prepare(
            &peer,
            &b,
            "CAtwo",
            "+18005550999",
            &b.phone_number,
            Some(&z)
        )
    );
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    let s = if let Ok((s, _)) = x { s } else { y.unwrap().0 };
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(20), "fixture")
        .await
        .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    t.f.state
        .db
        .collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"calls_per_minute":1_i64},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    assert!(
        calls::prepare(
            &peer,
            &b,
            "CAratelimited",
            "+18005550999",
            &b.phone_number,
            Some(&occurrence())
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn concurrent_calls_on_one_claim_fill_slots_then_refuse_by_capacity() {
    let (t, b) = setup_call().await;
    t.f.state
        .db
        .collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"call_slot_limit":2_i64}},
        )
        .await
        .unwrap();
    let peer = separate_client_state(&t, &format!("managed-slot-peer-{}", id())).await;
    let (one, two, three) = (occurrence(), occurrence(), occurrence());
    // Every admission writes the same claim, line and account: losers retry
    // against unchanged authority, and only the exact slot rule refuses.
    let (x, y, z) = tokio::join!(
        calls::prepare(
            &t.f.state,
            &b,
            "CAslotone",
            "+18005550999",
            &b.phone_number,
            Some(&one)
        ),
        calls::prepare(
            &peer,
            &b,
            "CAslottwo",
            "+18005550999",
            &b.phone_number,
            Some(&two)
        ),
        calls::prepare(
            &t.f.state,
            &b,
            "CAslotthree",
            "+18005550999",
            &b.phone_number,
            Some(&three)
        )
    );
    let results = [x.map(|r| r.0.id), y.map(|r| r.0.id), z.map(|r| r.0.id)];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2);
    for result in &results {
        if let Err(error) = result {
            assert!(
                matches!(error, AppError::Conflict(m) if m == "Managed line call capacity is full"),
                "{error:?}"
            );
        }
    }
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
            .count_documents(doc! {"admitted.line_id":&t.f.line.id,"slot_held":true})
            .await
            .unwrap(),
        2
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(3600));
}
#[tokio::test]
async fn managed_call_unknown_commit_replays_same_provider_financial_identity() {
    let _guard = COMMIT_FAULT.lock().await;
    let (t, b) = setup_call().await;
    let app = format!("managed-call-unknown-{}", id());
    let peer = separate_client_state(&t, &app).await;
    uncertain_commit(&t, &app).await;
    let event = occurrence();
    let first = calls::prepare(
        &peer,
        &b,
        "CAunknown",
        "+18005550999",
        &b.phone_number,
        Some(&event),
    )
    .await
    .unwrap();
    let again = calls::prepare(
        &peer,
        &b,
        "CAunknown",
        "+18005550999",
        &b.phone_number,
        Some(&event),
    )
    .await
    .unwrap();
    assert_eq!(first.0.id, again.0.id);
    assert!(!again.1);
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
}
#[tokio::test]
async fn stable_twilio_authentication_reject_first_and_unseen_old_quarantine() {
    let (t, b) = setup_call().await;
    let old = call_body(&b, "CAold", Utc::now() - Duration::days(1));
    let headers = sign(&t, false, &old);
    let output = ingress::receive(
        t.f.state.clone(),
        &t.f.line.id,
        false,
        &headers,
        old.as_bytes(),
    )
    .await
    .unwrap();
    assert_eq!(output, "<Response><Reject/></Response>");
    let route =
        t.f.state
            .db
            .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
            .find_one(doc! {"provider_effect_id":"CAold"})
            .await
            .unwrap()
            .unwrap();
    assert!(route.binding_id.is_none());
    assert!(route.managed_claim_id.is_none());
    let forged = super::super::signed_twilio(&t.f, &b, &old);
    assert!(
        ingress::receive(
            t.f.state.clone(),
            &t.f.line.id,
            false,
            &forged,
            old.as_bytes()
        )
        .await
        .is_err()
    );
    flags::set_platform_override(
        &t.f.state.db,
        flags::MANAGED_LINES_FLAG_KEY,
        &flags::FlagTarget::User(t.f.owner.clone()),
        false,
        &t.f.admin.user_id,
    )
    .await
    .unwrap();
    let body = call_body(&b, "CAoff", Utc::now());
    let headers = sign(&t, false, &body);
    assert_eq!(
        ingress::receive(
            t.f.state.clone(),
            &t.f.line.id,
            false,
            &headers,
            body.as_bytes()
        )
        .await
        .unwrap(),
        "<Response><Reject/></Response>"
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<CallSession>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn retained_terminal_routes_original_claim_without_readiness_and_after_erasure() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    t.f.state
        .db
        .collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"current_claim_id":id(),"state":"claimed"},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    super::super::make_managed_unready(&t.f.state.db).await;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("AccountSid", b.account_id.as_str()),
            ("CallSid", s.provider_call_id.as_str()),
            ("CallStatus", "completed"),
            ("CallDuration", "1901"),
            ("From", s.from.as_str()),
            ("To", s.to.as_str()),
        ])
        .finish();
    let h = sign(&t, false, &body);
    ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, body.as_bytes())
        .await
        .unwrap();
    let ended = session::load(&t.f.state, &s.id).await.unwrap();
    assert_eq!(ended.provider_seconds, Some(1901));
    assert_eq!(ended.billable_seconds, Some(1800));
    assert_eq!(ended.absorbed_seconds, Some(101));
    assert!(ended.billing_finalized);
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(1830)
    );
    t.f.state
        .db
        .collection::<CallSession>(SESSIONS)
        .delete_one(doc! {"_id":&s.id})
        .await
        .unwrap();
    t.f.state
        .db
        .collection::<Binding>(BINDINGS)
        .delete_one(doc! {"_id":&b.id})
        .await
        .unwrap();
    ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, body.as_bytes())
        .await
        .unwrap();
    assert_eq!(
        t.f.state
            .db
            .collection::<CallSession>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
async fn price_service(
    t: &LifecycleFixture,
    role: ManagedCommsRole,
    metric: BillingMetric,
) -> String {
    let mut service = dummy_service();
    service.id = id();
    service.slug = format!("managed-test-{}", id());
    service.managed_comms_role = Some(role);
    service.credential_encrypted.clear();
    service.visibility = "private".into();
    service.service_category = "internal".into();
    service.platform_key = None;
    let code = format!("managed_test_{}", id());
    let lane = LanePricing {
        metric,
        credits_per_unit: "1".into(),
        lago_metric_code: code.clone(),
        sync_status: PricingSyncStatus::Synced,
        sync_error: None,
        components: vec![],
        operations: vec![],
        model_prices: vec![],
    };
    service.billing = Some(ServiceBilling {
        platform_key_pricing: Some(lane),
        ..Default::default()
    });
    t.f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&service)
        .await
        .unwrap();
    t.f.state
        .db
        .collection::<crate::models::billing_rate_cache::BillingRateCache>(
            crate::models::billing_rate_cache::COLLECTION_NAME,
        )
        .insert_one(crate::models::billing_rate_cache::BillingRateCache {
            id: crate::models::billing_rate_cache::BillingRateCache::cache_id(&code, None),
            lago_metric_code: code,
            model: None,
            credits_per_unit_micros: 1_000_000,
            credits_per_unit_pico: Some(1_000_000_000_000),
            synced_at: Utc::now(),
            retired_at: None,
        })
        .await
        .unwrap();
    service.id
}
#[tokio::test]
async fn second_meter_failure_rolls_back_both_and_crash_after_first_settlement_recovers() {
    let (t, b) = setup_call().await;
    let premium = price_service(&t, ManagedCommsRole::Premium, BillingMetric::VoiceSeconds).await;
    let base =
        t.f.state
            .billing
            .prepare_managed_effect(
                &b.user_id,
                &t.f.line.voice.as_ref().unwrap().billing_service_id,
                ManagedCommsRole::Voice,
            )
            .await
            .unwrap();
    let premium =
        t.f.state
            .billing
            .prepare_managed_effect(&b.user_id, &premium, ManagedCommsRole::Premium)
            .await
            .unwrap();
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(
            doc! {},
            doc! {"$set":{"balance_credits":Credits::from_whole(2000)}},
        )
        .await
        .unwrap();
    let session_id = id();
    let db = &t.f.state.db;
    let result = transaction_service::run_db_only(
        db,
        &(db, &base, &premium, &b.user_id, &session_id),
        |tx, c| {
            Box::pin(async move {
                let (db, base, premium, owner, sid) = *c;
                calls::reserve_meters(db, tx, owner, sid, base, Some(premium), Utc::now()).await
            })
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(wallet(&t).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE)
            .count_documents(doc! {"billing_request_id":{"$regex":"^managed-call:"}})
            .await
            .unwrap(),
        0
    );
    db.collection::<bson::Document>(WALLETS)
        .update_many(
            doc! {},
            doc! {"$set":{"balance_credits":Credits::from_whole(10000)}},
        )
        .await
        .unwrap();
    let mut s = session::new_row(&b, "CAtwometers", "+18005550999", &b.phone_number);
    s.id = session_id;
    s.managed_claim_id = Some(id());
    s.managed_claim_generation = Some(1);
    let reference = occurrence()
        .reference(&t.f.line, &t.f.account, Some(&s.provider_call_id), None)
        .unwrap();
    s.admitted_reference = Some(reference.clone());
    transaction_service::run_db_only(db, &(db, &base, &premium, &s, &reference), |tx, c| {
        Box::pin(async move {
            let (db, base, premium, s, reference) = *c;
            let (base, premium) =
                calls::reserve_meters(db, tx, &s.user_id, &s.id, base, Some(premium), Utc::now())
                    .await?;
            db.collection::<CallSession>(SESSIONS)
                .insert_one(s)
                .session(&mut *tx)
                .await?;
            let r = ManagedCallAdmission {
                id: id(),
                session_id: s.id.clone(),
                managed_claim_id: s.managed_claim_id.clone().unwrap(),
                managed_claim_generation: 1,
                admitted: reference.clone(),
                base,
                premium,
                underwriting_snapshot: None,
                underwriting_hold_id: None,
                spend_hold_id: None,
                slot_held: true,
                financially_finalized: false,
                answer_dispatch_anchor: None,
                hangup_deadline: None,
                created_at: Utc::now(),
            };
            db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
                .insert_one(r)
                .session(&mut *tx)
                .await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(1900), "fixture")
        .await
        .unwrap();
    let r = calls::receipt(&t.f.state, &s.id).await.unwrap();
    crate::services::call_channels::billing::settle_admitted(
        &t.f.state,
        &r.base.billing_request_id,
        &b.user_id,
        1800,
        1,
    )
    .await
    .unwrap();
    assert!(
        !session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .billing_finalized
    );
    calls::finalize(&t.f.state, &ended).await.unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    assert!(
        session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .billing_finalized
    );
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(3630)
    );
}
#[tokio::test]
async fn terminated_missing_quantity_never_infers_elapsed_or_zero_and_admin_resolution_is_audited()
{
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let ended = session::terminal(&t.f.state, &s, Utc::now(), None, "fixture_terminated")
        .await
        .unwrap();
    assert!(ended.terminated_quantity_pending);
    assert_eq!(ended.provider_seconds, None);
    assert!(ended.hangup_confirmed);
    assert!(calls::finalize(&t.f.state, &ended).await.is_err());
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    super::super::make_managed_unready(&t.f.state.db).await;
    assert!(matches!(
        calls::resolve_evidence(&t.f.state, &t.actor, &s.id, 42, "unauthorized fixture").await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .provider_seconds,
        None
    );
    calls::resolve_evidence(
        &t.f.state,
        &t.f.admin,
        &s.id,
        42,
        "provider reference fixture",
    )
    .await
    .unwrap();
    assert!(
        session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .billing_finalized
    );
    assert!(
        calls::resolve_evidence(&t.f.state, &t.f.admin, &s.id, 43, "different evidence")
            .await
            .is_err()
    );
    calls::resolve_evidence(
        &t.f.state,
        &t.f.admin,
        &s.id,
        42,
        "provider reference fixture",
    )
    .await
    .unwrap();
    assert!(
        calls::resolve_evidence(&t.f.state, &t.f.admin, &s.id, 42, "changed reference")
            .await
            .is_err()
    );
    let audits: Vec<bson::Document> =
        t.f.state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type":"managed_call_evidence_resolved"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(audits.len(), 1);
    assert!(!format!("{audits:?}").contains("provider reference fixture"));
}
#[test]
fn telnyx_timestamp_and_detail_fixtures_require_exact_identity_and_round_up() {
    let at = Utc::now();
    assert_eq!(
        calls::timestamp_seconds(at, at + Duration::milliseconds(1001)).unwrap(),
        2
    );
    assert!(calls::timestamp_seconds(at, at - Duration::seconds(1)).is_err());
    let reference = AdmittedReference {
        trusted_account_id: id(),
        credential_service_id: id(),
        provider: "telnyx".into(),
        account_id: id(),
        line_id: id(),
        number_resource_id: id(),
        application_id: "connection".into(),
        provider_call_id: Some("control".into()),
        provider_message_id: None,
        call_leg_id: Some("leg".into()),
        call_session_id: Some("session".into()),
    };
    let fixture = json!({"data":[{"record_type":"call-control","call_leg_id":"leg","connection_id":"connection","answered_at":at.to_rfc3339(),"finished_at":(at+Duration::milliseconds(1500)).to_rfc3339()}]});
    assert_eq!(
        calls::detail_quantity(&reference, &fixture)
            .unwrap()
            .unwrap()
            .1,
        2
    );
    let mut wrong = fixture;
    wrong["data"][0]["call_leg_id"] = json!("foreign");
    assert!(
        calls::detail_quantity(&reference, &wrong)
            .unwrap()
            .is_none()
    );
}

async fn setup_sms() -> (LifecycleFixture, Binding) {
    let (mut t, voice) = setup_call().await;
    let service = price_service(&t, ManagedCommsRole::Sms, BillingMetric::Requests).await;
    let mut b = voice.clone();
    b.id = id();
    b.profile = Profile::TwilioSms;
    t.f.line =
        t.f.state
            .db
            .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
            .find_one(doc! {"_id":&t.f.line.id})
            .await
            .unwrap()
            .unwrap();
    t.f.line.sms = Some(Capability {
        application_id: b.application_id.clone(),
        billing_service_id: service,
    });
    t.f.line.regulatory.sms_sender_registration = Some("approved fixture sender".into());
    t.f.state
        .db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .replace_one(doc! {"_id":&t.f.line.id}, &t.f.line)
        .await
        .unwrap();
    let CredentialBinding::ManagedLine {
        managed_claim_id, ..
    } = &b.credential
    else {
        unreachable!()
    };
    t.f.state
        .db
        .collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(
            doc! {"_id":managed_claim_id},
            doc! {"$set":{"sms_enabled":true,"sms_binding_id":&b.id},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    t.f.state
        .db
        .collection::<Binding>(BINDINGS)
        .insert_one(&b)
        .await
        .unwrap();
    binding::live(&t.f.state, &b).await.unwrap();
    (t, b)
}
async fn pending_sms(t: &LifecycleFixture, b: &Binding, parts: i64) -> String {
    let now = Utc::now();
    let reference = occurrence()
        .reference(
            &t.f.line,
            &t.f.account,
            None,
            Some(&format!("SM{}", Uuid::new_v4().simple())),
        )
        .unwrap();
    let row = ManagedInboundSms {
        id: id(),
        trusted_account_id: t.f.account.id.clone(),
        provider_message_id: reference.provider_message_id.clone().unwrap(),
        line_id: t.f.line.id.clone(),
        occurred_at: now,
        parts,
        outcomes: (0..parts)
            .map(|part| SmsPartOutcome::Pending { part })
            .collect(),
        delivery_claimed: false,
        quarantined: false,
        financially_finalized: false,
        admitted: reference.clone(),
        created_at: now,
    };
    let CredentialBinding::ManagedLine {
        managed_claim_id,
        managed_claim_generation,
        ..
    } = &b.credential
    else {
        unreachable!()
    };
    let route = ManagedEffectRoute {
        id: id(),
        trusted_account_id: t.f.account.id.clone(),
        provider: t.f.line.provider.clone(),
        provider_effect_id: row.provider_message_id.clone(),
        line_id: t.f.line.id.clone(),
        managed_claim_id: Some(managed_claim_id.clone()),
        managed_claim_generation: Some(*managed_claim_generation),
        binding_id: Some(b.id.clone()),
        effect_id: row.id.clone(),
        kind: EffectKind::Sms,
        financially_finalized: false,
        admitted: reference,
        covered_from: now,
        covered_until: None,
        created_at: now,
    };
    t.f.state
        .db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    t.f.state
        .db
        .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
        .insert_one(route)
        .await
        .unwrap();
    row.id
}
#[tokio::test]
async fn inbound_sms_without_claim_flags_or_tenant_records_absorbed_liability_and_discards_content()
{
    let (t, b) = setup_sms().await;
    t.f.state
        .db
        .collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"current_claim_id":bson::Bson::Null,"state":"available"}},
        )
        .await
        .unwrap();
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("AccountSid", b.account_id.as_str()),
            ("MessageSid", "SMliability"),
            ("SmsStatus", "received"),
            ("NumSegments", "2"),
            ("NumMedia", "0"),
            ("Body", "private body never stored"),
            ("From", "+18005550999"),
            ("To", b.phone_number.as_str()),
            ("Timestamp", &Utc::now().to_rfc3339()),
        ])
        .finish();
    let headers = sign(&t, true, &body);
    ingress::receive(
        t.f.state.clone(),
        &t.f.line.id,
        true,
        &headers,
        body.as_bytes(),
    )
    .await
    .unwrap();
    ingress::receive(
        t.f.state.clone(),
        &t.f.line.id,
        true,
        &headers,
        body.as_bytes(),
    )
    .await
    .unwrap();
    let rows: Vec<ManagedInboundSms> =
        t.f.state
            .db
            .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].financially_finalized);
    assert!(
        rows[0]
            .outcomes
            .iter()
            .all(|p| matches!(p, SmsPartOutcome::Absorbed { .. }))
    );
    assert!(rows[0].quarantined);
    assert!(!rows[0].delivery_claimed);
    assert!(!format!("{:?}", bson::to_document(&rows[0]).unwrap()).contains("private body"));
    assert_eq!(
        t.f.state
            .db
            .collection::<CallSession>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn sms_partial_accounting_recovery_never_rebills_absorbed_parts() {
    let (t, b) = setup_sms().await;
    let id = pending_sms(&t, &b, 2).await;
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(
            doc! {},
            doc! {"$set":{"balance_credits":Credits::from_whole(31)}},
        )
        .await
        .unwrap();
    sms::recover_one(&t.f.state, &id).await.unwrap();
    let row =
        t.f.state
            .db
            .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
    assert!(matches!(
        row.outcomes[0],
        SmsPartOutcome::CustomerFunded { .. }
    ));
    assert!(matches!(row.outcomes[1], SmsPartOutcome::Absorbed { .. }));
    assert!(row.financially_finalized);
    assert!(!row.delivery_claimed);
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(31)
    );
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(
            doc! {},
            doc! {"$set":{"balance_credits":Credits::from_whole(1000)}},
        )
        .await
        .unwrap();
    super::super::make_managed_unready(&t.f.state.db).await;
    sms::recover_one(&t.f.state, &id).await.unwrap();
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(31)
    );
    let row =
        t.f.state
            .db
            .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
    assert!(matches!(row.outcomes[1], SmsPartOutcome::Absorbed { .. }));
}
#[tokio::test]
async fn sms_recovery_settles_funded_prefix_while_off_and_absorbs_pending_suffix() {
    let (t, b) = setup_sms().await;
    let liability = pending_sms(&t, &b, 2).await;
    let f =
        t.f.state
            .billing
            .prepare_managed_effect(
                &b.user_id,
                &t.f.line.sms.as_ref().unwrap().billing_service_id,
                ManagedCommsRole::Sms,
            )
            .await
            .unwrap();
    let request = format!("managed-inbound:{liability}:0");
    let claim = if let CredentialBinding::ManagedLine {
        managed_claim_id, ..
    } = &b.credential
    {
        managed_claim_id.clone()
    } else {
        unreachable!()
    };
    let db = &t.f.state.db;
    transaction_service::run_db_only(db,&(db,&f,&b.user_id,&request,&liability,&claim),|tx,c|Box::pin(async move{
        let(db,f,owner,request,id,claim)=*c;f.reserve(db,tx,owner,request,Utc::now()).await?;
        db.collection::<bson::Document>(ManagedInboundSms::COLLECTION_NAME).update_one(doc!{"_id":id},doc!{"$set":{"outcomes.0":bson::to_bson(&SmsPartOutcome::CustomerFunded{part:0,claim_id:claim.clone(),billing_request_id:request.clone()}).unwrap()}}).session(&mut *tx).await?;Ok(())
    })).await.unwrap();
    flags::set_platform_override(
        db,
        flags::MANAGED_LINES_FLAG_KEY,
        &flags::FlagTarget::User(t.f.owner.clone()),
        false,
        &t.f.admin.user_id,
    )
    .await
    .unwrap();
    super::super::make_managed_unready(db).await;
    sms::recover_one(&t.f.state, &liability).await.unwrap();
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(31)
    );
    let row = db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"_id":liability})
        .await
        .unwrap()
        .unwrap();
    assert!(row.financially_finalized);
    assert!(matches!(row.outcomes[1], SmsPartOutcome::Absorbed { .. }));
}
#[tokio::test]
async fn underwriting_authoring_requires_paid_ceiling_complete_fixtures_and_capacity() {
    let (t, _) = setup_call().await;
    let db = &t.f.state.db;
    db.collection::<bson::Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":&t.f.account.id},
            doc! {"$set":{"provider":"telnyx"}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"provider":"telnyx","provider_channel_limit":2_i64}},
        )
        .await
        .unwrap();
    let p = super::super::underwriting_policy();
    // The API takes exact decimal credit strings, never BSON extended JSON.
    let components: Vec<_> = p
        .components
        .iter()
        .map(|c| json!({"kind":c.kind,"credits_per_second":c.credits_per_second.to_string(),"increment_seconds":c.increment_seconds,"minimum_seconds":c.minimum_seconds}))
        .collect();
    let input = json!({"revision":0,"enabled":true,"acceptance_text":"underwritten exception, not reserved liability","acceptance_version":"v1","paid_tier_evidence_reference":"paid account invoice fixture","max_leg_seconds":1801,"ceiling_evidence":{"kind":"paid_inbound_test","reference":"observed inbound fixture","hangup_cause":"time_limit","paid_tier":true},"timestamp_fixture_reference":"duration rounding fixture","detail_record_options_reference":"options fixture","detail_record_fixture_reference":"validated call-control fixture","detail_record_fields":["call_leg_id","connection_id","answered_at","finished_at"],"supplier_tariff_revision":1,"fx_policy_revision":"fx fixture","character_tariff_reference":"standard tts character tariff fixture","components":components,"rolling_budget":"1000"});
    for bad in [
        json!({"$numberDecimal": "1"}),
        json!("one"),
        json!(1),
        // Exponent notation must still scale to whole picocredits.
        json!("1e-13"),
        json!("1.5e-12"),
        json!("0.0000000000001e0"),
    ] {
        let mut wire = input.clone();
        wire["components"][0]["credits_per_second"] = bad.clone();
        assert!(
            serde_json::from_value::<underwriting::UnderwritingInput>(wire).is_err(),
            "{bad}"
        );
    }
    // The shared Credits parser accepts exact exponent notation.
    for (exact, pico) in [("1e3", 1_000_000_000_000_000_i128), ("5e-12", 5)] {
        let mut wire = input.clone();
        wire["components"][0]["credits_per_second"] = json!(exact);
        let parsed = serde_json::from_value::<underwriting::UnderwritingInput>(wire).unwrap();
        assert_eq!(
            parsed.components[0].credits_per_second,
            crate::models::credits::Credits::from_pico(pico).unwrap(),
            "{exact}"
        );
    }
    for field in [
        "paid_tier_evidence_reference",
        "detail_record_options_reference",
        "detail_record_fixture_reference",
        "character_tariff_reference",
    ] {
        let mut bad = input.clone();
        bad[field] = json!("");
        assert!(
            underwriting::configure(
                &t.f.state,
                &t.f.admin,
                &t.f.account.id,
                serde_json::from_value(bad).unwrap()
            )
            .await
            .is_err(),
            "{field}"
        );
    }
    let mut bad = input.clone();
    bad["ceiling_evidence"]["paid_tier"] = json!(false);
    assert!(
        underwriting::configure(
            &t.f.state,
            &t.f.admin,
            &t.f.account.id,
            serde_json::from_value(bad).unwrap()
        )
        .await
        .is_err()
    );
    let mut bad = input.clone();
    bad["rolling_budget"] = json!("500");
    assert!(
        underwriting::configure(
            &t.f.state,
            &t.f.admin,
            &t.f.account.id,
            serde_json::from_value(bad).unwrap()
        )
        .await
        .is_err()
    );
    underwriting::configure(
        &t.f.state,
        &t.f.admin,
        &t.f.account.id,
        serde_json::from_value(input.clone()).unwrap(),
    )
    .await
    .unwrap();
    let saved = db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&t.f.account.id})
        .await
        .unwrap()
        .unwrap()
        .underwriting
        .unwrap();
    assert_eq!(saved.accepted_by, t.f.admin.user_id);
    assert_eq!(saved.revision, 1);
    assert_eq!(
        super::super::custody::worst_case_exposure(&saved).unwrap(),
        Credits::from_whole(360)
    );
    let mut bad = input;
    bad["revision"] = json!(1);
    bad["rolling_budget"] = json!("500");
    assert!(
        underwriting::configure(
            &t.f.state,
            &t.f.admin,
            &t.f.account.id,
            serde_json::from_value(bad).unwrap()
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn durable_telnyx_hangup_retries_same_command_id_until_confirmed() {
    let (t, mut b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let db = &t.f.state.db;
    let mut reference = s.admitted_reference.clone().unwrap();
    reference.provider = "telnyx".into();
    reference.call_leg_id = Some("fixture-leg".into());
    reference.call_session_id = Some("fixture-session".into());
    let mut service = t.f.service.clone();
    service.base_url = "https://api.telnyx.com".into();
    service.auth_method = "bearer".into();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&service.id}, service)
        .await
        .unwrap();
    db.collection::<bson::Document>(ACCOUNTS)
        .update_one(
            doc! {"_id":&t.f.account.id},
            doc! {"$set":{"provider":"telnyx"}},
        )
        .await
        .unwrap();
    b.profile = Profile::TelnyxRelay;
    db.collection::<Binding>(BINDINGS)
        .replace_one(doc! {"_id":&b.id}, &b)
        .await
        .unwrap();
    db.collection::<bson::Document>(SESSIONS).update_one(doc!{"_id":&s.id},doc!{"$set":{"profile":"telnyx_relay","admitted_reference":bson::to_bson(&reference).unwrap()}}).await.unwrap();
    db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME)
        .update_one(
            doc! {"session_id":&s.id},
            doc! {"$set":{"admitted":bson::to_bson(&reference).unwrap()}},
        )
        .await
        .unwrap();
    let server = MockServer::start().await;
    crate::services::call_channels::profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), server.uri());
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    coordinator::close(
        &t.f.state,
        &b,
        &session::load(&t.f.state, &s.id).await.unwrap(),
        false,
    )
    .await
    .unwrap();
    let first = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"result":"ok"}})))
        .mount(&server)
        .await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"call_control_id":s.provider_call_id,"is_alive":false,"call_duration":12}}))).mount(&server).await;
    super::super::make_managed_unready(db).await;
    coordinator::close(
        &t.f.state,
        &b,
        &session::load(&t.f.state, &s.id).await.unwrap(),
        false,
    )
    .await
    .unwrap();
    let second = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    assert_eq!(
        first.body_json::<serde_json::Value>().unwrap()["command_id"],
        second.body_json::<serde_json::Value>().unwrap()["command_id"]
    );
    let saved = session::load(&t.f.state, &s.id).await.unwrap();
    assert_eq!(saved.hangup_attempts, 2);
    assert!(saved.hangup_confirmed);
    assert!(saved.billing_finalized);
    assert_eq!(saved.provider_seconds, Some(12));
}
#[tokio::test]
async fn managed_webhook_routes_are_public_and_bounded() {
    use axum::{
        Router,
        body::Body,
        extract::DefaultBodyLimit,
        http::{Request, StatusCode},
        routing::post,
    };
    use tower::ServiceExt;
    let (t, _) = setup_call().await;
    let app = Router::new()
        .route(
            "/api/v1/webhooks/managed-lines/{id}/voice",
            post(crate::handlers::managed_comms::voice_webhook)
                .layer(DefaultBodyLimit::max(protocol::MAX_INGRESS)),
        )
        .with_state(t.f.state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/webhooks/managed-lines/{}/voice?secret=never_log",
                    t.f.line.id
                ))
                .body(Body::from(vec![b'x'; protocol::MAX_INGRESS + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

async fn setup_telnyx() -> (LifecycleFixture, Binding, ed25519_dalek::SigningKey) {
    use base64::Engine;
    let (mut t, mut b) = setup_call().await;
    let db = &t.f.state.db;
    let signer = ed25519_dalek::SigningKey::from_bytes(&[47; 32]);
    let public =
        base64::engine::general_purpose::STANDARD.encode(signer.verifying_key().to_bytes());
    t.f.account.provider = "telnyx".into();
    t.f.account.webhook_public_key = Some(public.clone());
    t.f.account.underwriting = Some(super::super::underwriting_policy());
    db.collection::<ProviderAccount>(ACCOUNTS)
        .replace_one(doc! {"_id":&t.f.account.id}, &t.f.account)
        .await
        .unwrap();
    t.f.service.base_url = "https://api.telnyx.com".into();
    t.f.service.auth_method = "bearer".into();
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&t.f.service.id}, &t.f.service)
        .await
        .unwrap();
    t.f.line = db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":&t.f.line.id})
        .await
        .unwrap()
        .unwrap();
    t.f.line.provider = "telnyx".into();
    t.f.line.provider_channel_limit = Some(1);
    t.f.line.standard_voice_options.clear();
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .replace_one(doc! {"_id":&t.f.line.id}, &t.f.line)
        .await
        .unwrap();
    let option = super::super::voice::default_option("telnyx");
    b.profile = Profile::TelnyxRelay;
    b.webhook_public_key = Some(public);
    b.voice = option.voice.clone();
    b.language = option.language.clone();
    db.collection::<Binding>(BINDINGS)
        .replace_one(doc! {"_id":&b.id}, &b)
        .await
        .unwrap();
    let CredentialBinding::ManagedLine {
        managed_claim_id, ..
    } = &b.credential
    else {
        unreachable!()
    };
    db.collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(
            doc! {"_id":managed_claim_id},
            doc! {"$set":{"standard_voice":bson::to_bson(&option).unwrap()}},
        )
        .await
        .unwrap();
    for (kind, resource) in [
        ("number_id", &t.f.line.number_resource_id),
        ("phone", &t.f.line.phone_number),
        ("application", &b.application_id),
    ] {
        db.collection::<ResourceClaim>(CLAIMS)
            .insert_one(ResourceClaim {
                id: id(),
                identity_key: session::identity(&["telnyx", kind, resource]),
                revision: 1,
                user_id: super::super::custody::sentinel(&t.f.line.id),
                provider: "telnyx".into(),
                resource_kind: kind.into(),
                resource_id: resource.clone(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
    }
    binding::live(&t.f.state, &b).await.unwrap();
    (t, b, signer)
}
async fn prepare_telnyx(t: &LifecycleFixture, b: &Binding, call: &str) -> CallSession {
    let mut event = occurrence();
    event.call_leg_id = Some(id());
    event.call_session_id = Some(id());
    calls::prepare(
        &t.f.state,
        b,
        call,
        "+18005550999",
        &b.phone_number,
        Some(&event),
    )
    .await
    .unwrap()
    .0
}
fn telnyx_event(
    s: &CallSession,
    event: &str,
    at: chrono::DateTime<Utc>,
    end: Option<chrono::DateTime<Utc>>,
) -> String {
    json!({"data":{"id":id(),"event_type":event,"occurred_at":at.to_rfc3339(),"payload":{
        "call_control_id":s.provider_call_id,"connection_id":s.admitted_reference.as_ref().unwrap().application_id,
        "call_leg_id":s.provider_call_leg_id,"call_session_id":s.provider_call_session_id,
        "end_time":end.map(|v|v.to_rfc3339())}}}).to_string()
}
fn telnyx_headers(signer: &ed25519_dalek::SigningKey, body: &str, at: i64) -> HeaderMap {
    use base64::Engine;
    use ed25519_dalek::Signer;
    let timestamp = at.to_string();
    let mut h = HeaderMap::new();
    h.insert("telnyx-timestamp", timestamp.parse().unwrap());
    h.insert(
        "telnyx-signature-ed25519",
        base64::engine::general_purpose::STANDARD
            .encode(
                signer
                    .sign(format!("{timestamp}|{body}").as_bytes())
                    .to_bytes(),
            )
            .parse()
            .unwrap(),
    );
    h
}
#[tokio::test]
async fn trusted_telnyx_source_two_is_fixture_gated_identity_bound_and_never_uses_occurrence_as_end()
 {
    let (t, b, signer) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v2:source-two==").await;
    let server = MockServer::start().await;
    crate::services::call_channels::profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), server.uri());
    let start = (Utc::now() - Duration::seconds(3))
        .with_nanosecond(123_456_789)
        .unwrap();
    let answer = telnyx_event(&s, "call.answered", start, None);
    let h = telnyx_headers(&signer, &answer, Utc::now().timestamp());
    ingress::receive(
        t.f.state.clone(),
        &t.f.line.id,
        false,
        &h,
        answer.as_bytes(),
    )
    .await
    .unwrap();
    // A mismatched signed leg cannot alter original financial evidence.
    let mut wrong: serde_json::Value = serde_json::from_str(&telnyx_event(
        &s,
        "call.hangup",
        Utc::now(),
        Some(start + Duration::seconds(2)),
    ))
    .unwrap();
    wrong["data"]["payload"]["call_leg_id"] = json!("other-leg");
    let wrong = wrong.to_string();
    let h = telnyx_headers(&signer, &wrong, Utc::now().timestamp());
    assert!(
        ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, wrong.as_bytes())
            .await
            .is_err()
    );
    let no_end = telnyx_event(&s, "call.hangup", Utc::now(), None);
    let h = telnyx_headers(&signer, &no_end, Utc::now().timestamp());
    ingress::receive(
        t.f.state.clone(),
        &t.f.line.id,
        false,
        &h,
        no_end.as_bytes(),
    )
    .await
    .unwrap();
    let pending = session::load(&t.f.state, &s.id).await.unwrap();
    assert!(pending.terminated_quantity_pending);
    assert!(!pending.billing_finalized);
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    let ended = telnyx_event(
        &s,
        "call.hangup",
        Utc::now(),
        Some(start + Duration::seconds(2)),
    );
    let h = telnyx_headers(&signer, &ended, Utc::now().timestamp());
    t.f.state
        .db
        .collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME)
        .update_one(
            doc! {"session_id":&s.id},
            doc! {"$set":{"underwriting_snapshot.timestamp_fixture_reference":bson::Bson::Null}},
        )
        .await
        .unwrap();
    ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, ended.as_bytes())
        .await
        .unwrap();
    assert!(
        session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .terminated_quantity_pending
    );
    t.f.state.db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME)
        .update_one(doc! {"session_id":&s.id},doc! {"$set":{"underwriting_snapshot.timestamp_fixture_reference":"verified rounding fixture"}}).await.unwrap();
    // Disabling live policy does not revoke the receipt's fixture-backed authority.
    super::super::make_managed_unready(&t.f.state.db).await;
    ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, ended.as_bytes())
        .await
        .unwrap();
    let saved = session::load(&t.f.state, &s.id).await.unwrap();
    assert_eq!(saved.provider_seconds, Some(2));
    assert!(saved.billing_finalized);
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(32)
    );
    // Replay-window verification precedes all persisted observation changes.
    let stale = telnyx_headers(&signer, &ended, Utc::now().timestamp() - 1000);
    assert!(
        ingress::receive(
            t.f.state.clone(),
            &t.f.line.id,
            false,
            &stale,
            ended.as_bytes()
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn telnyx_source_three_requires_frozen_schema_fixture_and_matching_leg_connection() {
    let (t, b, _) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v2:source-three==").await;
    let server = MockServer::start().await;
    crate::services::call_channels::profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(b.account_id.clone(), server.uri());
    let at = Utc::now() - Duration::seconds(3);
    let fixture = json!({"data":[{"record_type":"call-control","call_leg_id":s.provider_call_leg_id,"connection_id":b.application_id,"answered_at":at.to_rfc3339(),"finished_at":(at+Duration::milliseconds(2001)).to_rfc3339()}]});
    Mock::given(method("GET"))
        .and(path("/v2/detail_records"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture.clone()))
        .mount(&server)
        .await;
    // A 404 call read is never zero. A disabled fixture leaves the entire hold.
    t.f.state.db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc! {"session_id":&s.id},doc! {"$set":{"underwriting_snapshot.detail_record_fixture_reference":bson::Bson::Null}}).await.unwrap();
    assert!(calls::refresh_evidence(&t.f.state, &s).await.is_err());
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != "/v2/detail_records")
    );
    t.f.state.db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc! {"session_id":&s.id},doc! {"$set":{"underwriting_snapshot.detail_record_fixture_reference":"verified fixture"}}).await.unwrap();
    let mut wrong = fixture.clone();
    wrong["data"][0]["connection_id"] = json!("foreign");
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v2/detail_records"))
        .respond_with(ResponseTemplate::new(200).set_body_json(wrong))
        .mount(&server)
        .await;
    assert!(calls::refresh_evidence(&t.f.state, &s).await.is_err());
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v2/detail_records"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture))
        .mount(&server)
        .await;
    super::super::make_managed_unready(&t.f.state.db).await;
    calls::refresh_evidence(&t.f.state, &s).await.unwrap();
    let saved = session::load(&t.f.state, &s.id).await.unwrap();
    assert_eq!(saved.provider_seconds, Some(3));
    assert!(saved.billing_finalized);
}
#[tokio::test]
async fn underwriting_losses_consume_capacity_and_unresolved_holds_survive_window_boundary() {
    let (t, b, _) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v2:loss-one==").await;
    let r = calls::receipt(&t.f.state, &s.id).await.unwrap();
    let hold = r.underwriting_hold_id.unwrap();
    let budget =
        t.f.state
            .db
            .collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME)
            .find_one(doc! {"_id":&hold})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(budget.held, Credits::from_whole(360));
    let ended = session::terminal(
        &t.f.state,
        &s,
        Utc::now(),
        Some(1921),
        "authenticated_fixture",
    )
    .await
    .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    let loss =
        t.f.state
            .db
            .collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME)
            .find_one(doc! {"_id":hold})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(loss.recognized, Some(Credits::from_whole(540)));
    let second = prepare_telnyx(&t, &b, "v2:loss-two==").await;
    let ended = session::terminal(
        &t.f.state,
        &second,
        Utc::now(),
        Some(1921),
        "authenticated_fixture",
    )
    .await
    .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    let mut event = occurrence();
    event.call_leg_id = Some("third-leg".into());
    event.call_session_id = Some("third-session".into());
    assert!(matches!(
        calls::prepare(
            &t.f.state,
            &b,
            "v2:loss-three==",
            "+18005550999",
            &b.phone_number,
            Some(&event)
        )
        .await,
        Err(AppError::ManagedBudgetExhausted)
    ));
    assert_eq!(wallet(&t).await.reserved_credits, Credits::ZERO);
}
#[tokio::test]
async fn answer_anchor_is_once_only_and_timer_and_sweep_enforce_deadline_without_readiness() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    calls::anchor(&t.f.state, &s).await.unwrap();
    let first = calls::receipt(&t.f.state, &s.id).await.unwrap();
    calls::anchor(&t.f.state, &s).await.unwrap();
    let again = calls::receipt(&t.f.state, &s.id).await.unwrap();
    assert_eq!(first.answer_dispatch_anchor, again.answer_dispatch_anchor);
    assert_eq!(
        first.hangup_deadline.unwrap() - first.answer_dispatch_anchor.unwrap(),
        Duration::seconds(1740)
    );
    let response = json!({"sid":s.provider_call_id,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":s.from,"to":s.to,"direction":"inbound","status":"completed","duration":"5","end_time":Utc::now().to_rfc2822()});
    t.server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .mount(&t.server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&t.server)
        .await;
    t.f.state.db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc! {"session_id":&s.id},doc! {"$set":{"hangup_deadline":bson::DateTime::from_chrono(Utc::now()+Duration::milliseconds(20))}}).await.unwrap();
    super::super::make_managed_unready(&t.f.state.db).await;
    calls::arm(&t.f.state, &s);
    for _ in 0..60 {
        if session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .billing_finalized
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let saved = session::load(&t.f.state, &s.id).await.unwrap();
    assert!(saved.hangup_confirmed);
    assert!(saved.billing_finalized);
    // anchor() also arms a watchdog. Under load it can read the shortened
    // fixture deadline alongside the explicitly armed task, so both may attempt
    // termination before either confirms it. The sweep must add no attempts
    // after confirmation, regardless of that valid pre-confirmation race.
    assert!(saved.hangup_attempts >= 1);
    let confirmed_attempts = saved.hangup_attempts;
    let request = t
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&request.body).unwrap(),
        "Status=completed"
    );
    calls::sweep(&t.f.state).await.unwrap();
    assert_eq!(
        session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .hangup_attempts,
        confirmed_attempts
    );
}

#[tokio::test]
async fn twilio_hangup_requires_exact_completed_response_and_sweep_preserves_missing_quantity() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let mut value = json!({"sid":s.provider_call_id,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":s.from,"to":s.to,"direction":"inbound","status":"in-progress"});
    t.server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value.clone()))
        .mount(&t.server)
        .await;
    coordinator::close(&t.f.state, &b, &s, false).await.unwrap();
    assert!(
        !session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .hangup_confirmed
    );
    value["status"] = json!("completed");
    t.server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value.clone()))
        .mount(&t.server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value.clone()))
        .mount(&t.server)
        .await;
    coordinator::close(&t.f.state, &b, &s, false).await.unwrap();
    let pending = session::load(&t.f.state, &s.id).await.unwrap();
    assert!(pending.hangup_confirmed);
    assert!(pending.terminated_quantity_pending);
    assert!(!pending.billing_finalized);
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    t.f.state.db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME).update_one(doc! {"session_id":&s.id},doc! {"$set":{"answer_dispatch_anchor":bson::DateTime::now(),"hangup_deadline":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    value["duration"] = json!("7");
    value["end_time"] = json!(Utc::now().to_rfc2822());
    t.server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(&t.server)
        .await;
    super::super::make_managed_unready(&t.f.state.db).await;
    calls::sweep(&t.f.state).await.unwrap();
    let saved = session::load(&t.f.state, &s.id).await.unwrap();
    assert_eq!(saved.provider_seconds, Some(7));
    assert!(saved.billing_finalized);
    assert_eq!(saved.hangup_attempts, 2);
}
#[tokio::test]
async fn rejected_provider_identity_cannot_admit_after_capacity_returns() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    // The existing provider proof fixture answers exact identity for this call;
    // replacing only the SID makes the competing ringing request provable.
    let call = format!("CA{}", Uuid::new_v4().simple());
    let value = json!({"sid":call,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":s.from,"to":s.to,"direction":"inbound","status":"ringing"});
    t.server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(&t.server)
        .await;
    let body = call_body(&b, &call, Utc::now());
    let h = sign(&t, false, &body);
    assert_eq!(
        ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, body.as_bytes())
            .await
            .unwrap(),
        "<Response><Reject/></Response>"
    );
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(1), "provider_fixture")
        .await
        .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    assert_eq!(
        ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, body.as_bytes())
            .await
            .unwrap(),
        "<Response><Reject/></Response>"
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn eligibility_revocation_racing_call_admission_has_no_unreceipted_funding_or_answer() {
    let (t, b) = setup_call().await;
    let event = occurrence();
    let e = super::super::eligibility::load(&t.f.state.db, &b.user_id)
        .await
        .unwrap();
    let (admit, revoke) = tokio::join!(
        calls::prepare(
            &t.f.state,
            &b,
            "CAracing-revocation",
            "+18005550999",
            &b.phone_number,
            Some(&event)
        ),
        super::super::eligibility::revoke(
            &t.f.state,
            &t.f.admin,
            &b.user_id,
            e.revision,
            t.f.state.audit_chain_hmac_key.as_ref().as_ref()
        )
    );
    revoke.unwrap();
    if let Ok((s, _)) = admit {
        let r = calls::receipt(&t.f.state, &s.id).await.unwrap();
        assert_eq!(r.base.usage_row_ids.len(), 1);
        assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
        assert!(!s.answer_dispatched);
        assert!(binding::live(&t.f.state, &b).await.is_err());
    } else {
        assert_eq!(wallet(&t).await.reserved_credits, Credits::ZERO);
        assert_eq!(
            t.f.state
                .db
                .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}
#[test]
fn managed_send_receipts_resolve_their_own_durable_part_identities() {
    let receipt = |first: Option<String>| EventReceipt {
        id: id(),
        identity_key: id(),
        user_id: id(),
        binding_id: id(),
        binding_revision: 1,
        managed_claim_id: None,
        managed_claim_generation: None,
        admitted_reference: None,
        session_id: Some("session".into()),
        event_kind: "sms_send".into(),
        status: "dispatching".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: first,
        observed_parts: None,
        reserved_parts: Some(2),
        billing_finalized: false,
        from: None,
        to: None,
        created_at: Utc::now(),
    };
    let part =
        |first: &str, index| sms_delivery::outbound_request_id(&receipt(Some(first.into())), index);
    // Managed-line sends keep their stuck-request-exempt identity per part.
    assert_eq!(
        part("managed-sms-out:session:0", 1).unwrap(),
        "managed-sms-out:session:1"
    );
    assert_eq!(
        part("call-confirmation:op:0", 1).unwrap(),
        "call-confirmation:op:1"
    );
    // Owner-funded sends and legacy receipts keep the ordinary carrier identity.
    assert_eq!(
        part("adapter-sms-out:session:0", 1).unwrap(),
        "adapter-sms-out:session:1"
    );
    assert_eq!(
        sms_delivery::outbound_request_id(&receipt(None), 1).unwrap(),
        "adapter-sms-out:session:1"
    );
    for (first, index) in [
        ("managed-call:session:0", 0),
        ("proxy:session:0", 0),
        ("managed-sms-out:session", 0),
        ("managed-sms-out:session:0", 16),
    ] {
        assert!(part(first, index).is_err(), "{first} part {index}");
    }
}

#[tokio::test]
async fn managed_sms_send_budget_holds_retail_and_recovers_zero_before_dispatch() {
    let (t, b) = setup_sms().await;
    // The prepaid rental already recognizes 30 credits in this month's budget.
    // Leave room for the one-credit reply whose hold/recovery this test exercises.
    t.f.state
        .db
        .collection::<bson::Document>(ManagedCommsEligibility::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&b.user_id},
            doc! {"$set":{"monthly_spend_limit":Credits::from_whole(40)}},
        )
        .await
        .unwrap();
    let a = super::super::admission::read(&t.f.state.db, &b.user_id, &b.credential)
        .await
        .unwrap();
    let f =
        t.f.state
            .billing
            .prepare_managed_effect(
                &b.user_id,
                &a.line.sms.as_ref().unwrap().billing_service_id,
                ManagedCommsRole::Sms,
            )
            .await
            .unwrap();
    let mut s = session::new_row(
        &b,
        "sms:fixture-send-budget",
        "+18005550999",
        &b.phone_number,
    );
    s.managed_claim_id = Some(a.claim.id.clone());
    let reference = occurrence()
        .reference(&a.line, &a.account, None, Some("SMbudget"))
        .unwrap();
    let mut send = EventReceipt {
        id: id(),
        identity_key: id(),
        user_id: b.user_id.clone(),
        binding_id: b.id.clone(),
        binding_revision: b.revision,
        managed_claim_id: Some(a.claim.id.clone()),
        managed_claim_generation: Some(a.claim.managed_claim_generation),
        admitted_reference: Some(reference),
        session_id: Some(s.id.clone()),
        event_kind: "sms_send".into(),
        status: "dispatching".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: Some(format!("managed-sms-out:{}:0", s.id)),
        observed_parts: None,
        reserved_parts: Some(1),
        billing_finalized: false,
        from: Some(b.phone_number.clone()),
        to: Some(s.from.clone()),
        created_at: Utc::now(),
    };
    let db = &t.f.state.db;
    transaction_service::run_db_only(db, &(db, &a, &s, &send, &f), |tx, c| {
        Box::pin(async move {
            let (db, a, s, send, f) = *c;
            f.reserve(
                db,
                tx,
                &a.claim.user_id,
                send.billing_request_id.as_ref().unwrap(),
                Utc::now(),
            )
            .await?;
            db.collection::<EventReceipt>(EVENTS)
                .insert_one(send)
                .session(&mut *tx)
                .await?;
            sms::hold_send(db, tx, a, s, send).await
        })
    })
    .await
    .unwrap();
    let raw = db
        .collection::<bson::Document>(EVENTS)
        .find_one(doc! {"_id":&send.id})
        .await
        .unwrap()
        .unwrap();
    let hold = raw.get_str("managed_spend_hold_id").unwrap();
    let entry = db
        .collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME)
        .find_one(doc! {"_id":hold})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.held, Credits::from_whole(1));
    assert_eq!(entry.recognized, None);
    send.status = "rejected_before_dispatch".into();
    send.billing_finalized = true;
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&send.id},
            doc! {"$set":{"status":&send.status,"billing_finalized":true}},
        )
        .await
        .unwrap();
    super::super::make_managed_unready(db).await;
    crate::services::call_channels::sms_delivery::sweep(&t.f.state)
        .await
        .unwrap();
    let entry = db
        .collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME)
        .find_one(doc! {"_id":hold})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.recognized, Some(Credits::ZERO));
}

#[tokio::test]
async fn managed_binding_webhook_refuses_new_effects_even_with_a_valid_old_url_signature() {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let (t, b) = setup_call().await;
    let body = call_body(&b, &format!("CA{}", Uuid::new_v4().simple()), Utc::now());
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(coordinator::webhook_url(&t.f.state, &b).as_bytes());
    for (k, v) in protocol::form(body.as_bytes()).unwrap() {
        mac.update(k.as_bytes());
        mac.update(v.as_bytes());
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-twilio-signature",
        base64::engine::general_purpose::STANDARD
            .encode(mac.finalize().into_bytes())
            .parse()
            .unwrap(),
    );
    assert!(
        matches!(coordinator::ingress(t.f.state.clone(),&b.id,&headers,body.as_bytes()).await,
        Err(AppError::ChannelWebhookVerificationFailed(ref m)) if m=="New managed effects require the stable line webhook")
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::ZERO);
}

#[tokio::test]
async fn sms_line_rate_limit_absorbs_new_liability_and_reopens_after_rolling_window() {
    let (t, b) = setup_sms().await;
    let db = &t.f.state.db;
    db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"sms_per_minute":1_i64},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    let first = pending_sms(&t, &b, 1).await;
    sms::recover_one(&t.f.state, &first).await.unwrap();
    let second = pending_sms(&t, &b, 1).await;
    sms::recover_one(&t.f.state, &second).await.unwrap();
    let row = db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"_id":second})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(row.outcomes[0], SmsPartOutcome::Absorbed { .. }));
    assert!(row.financially_finalized);
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(31)
    );
    db.collection::<ManagedLineRateWindow>(ManagedLineRateWindow::COLLECTION_NAME).update_one(doc! {"key":format!("managed-rate:sms:{}",t.f.line.id)},doc! {"$set":{"events":[bson::DateTime::from_chrono(Utc::now()-Duration::seconds(61))]}}).await.unwrap();
    let third = pending_sms(&t, &b, 1).await;
    sms::recover_one(&t.f.state, &third).await.unwrap();
    let row = db
        .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
        .find_one(doc! {"_id":third})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        row.outcomes[0],
        SmsPartOutcome::CustomerFunded { .. }
    ));
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(32)
    );
}
#[tokio::test]
async fn managed_call_slots_use_lower_provider_limit_even_when_admin_allows_more() {
    let (t, b, _) = setup_telnyx().await;
    t.f.state
        .db
        .collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"call_slot_limit":2_i64},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    prepare_telnyx(&t, &b, "v2:provider-slot-one==").await;
    let mut event = occurrence();
    event.call_leg_id = Some(id());
    event.call_session_id = Some(id());
    assert!(
        matches!(calls::prepare(&t.f.state,&b,"v2:provider-slot-two==","+18005550999",&b.phone_number,Some(&event)).await,Err(AppError::Conflict(ref m)) if m=="Managed line call capacity is full")
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
}
#[tokio::test]
async fn stable_sms_receipt_recovers_original_send_before_route_without_current_claimant_authority()
{
    let (t, b) = setup_sms().await;
    let db = &t.f.state.db;
    let message = format!("SM{}", Uuid::new_v4().simple());
    let reference = occurrence()
        .reference(&t.f.line, &t.f.account, None, Some(&message))
        .unwrap();
    let CredentialBinding::ManagedLine {
        managed_claim_id,
        managed_claim_generation,
        ..
    } = &b.credential
    else {
        unreachable!()
    };
    let send = EventReceipt {
        id: id(),
        identity_key: id(),
        user_id: b.user_id.clone(),
        binding_id: b.id.clone(),
        binding_revision: b.revision,
        managed_claim_id: Some(managed_claim_id.clone()),
        managed_claim_generation: Some(*managed_claim_generation),
        admitted_reference: Some(reference),
        session_id: None,
        event_kind: "sms_send".into(),
        status: "accepted".into(),
        provider_message_id: Some(message.clone()),
        delivery: "accepted".into(),
        billing_request_id: None,
        observed_parts: Some(1),
        reserved_parts: Some(1),
        billing_finalized: true,
        from: Some(b.phone_number.clone()),
        to: Some("+18005550999".into()),
        created_at: Utc::now(),
    };
    db.collection::<EventReceipt>(EVENTS)
        .insert_one(&send)
        .await
        .unwrap();
    db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.f.line.id},
            doc! {"$set":{"current_claim_id":id()},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    super::super::make_managed_unready(db).await;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("AccountSid", b.account_id.as_str()),
            ("MessageSid", message.as_str()),
            ("SmsStatus", "delivered"),
            ("NumSegments", "1"),
            ("From", b.phone_number.as_str()),
            ("To", "+18005550999"),
        ])
        .finish();
    let h = sign(&t, true, &body);
    ingress::receive(t.f.state.clone(), &t.f.line.id, true, &h, body.as_bytes())
        .await
        .unwrap();
    let route = db
        .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
        .find_one(doc! {"provider_effect_id":&message})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(route.binding_id.as_deref(), Some(b.id.as_str()));
    assert_eq!(route.effect_id, send.id);
    assert_eq!(
        route.managed_claim_id.as_deref(),
        Some(managed_claim_id.as_str())
    );
    let saved = db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id":send.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.delivery, "delivered");
    assert_eq!(
        db.collection::<CallSession>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn committed_call_receipt_resumes_first_answer_after_lost_ack_without_second_funding() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let mut service = t.f.service.clone();
    service.proxy_operation_policy.as_mut().unwrap().rules.push(
        crate::models::downstream_service::ProxyOperationRule {
            method: "GET".into(),
            path_template: format!(
                "/2010-04-01/Accounts/{}/Calls/{}.json",
                b.account_id, s.provider_call_id
            ),
            ..Default::default()
        },
    );
    t.f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&service.id}, &service)
        .await
        .unwrap();
    Mock::given(method("GET")).and(path(format!("/2010-04-01/Accounts/{}/Calls/{}.json",b.account_id,s.provider_call_id))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"sid":s.provider_call_id,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":s.from,"to":s.to,"direction":"inbound","status":"ringing"}))).mount(&t.server).await;
    let body = call_body(&b, &s.provider_call_id, Utc::now());
    let h = sign(&t, false, &body);
    let result = ingress::receive(t.f.state.clone(), &t.f.line.id, false, &h, body.as_bytes())
        .await
        .unwrap();
    assert!(result.starts_with("<Response><Pause"), "{result}");
    assert!(result.contains(&ingress::url(&t.f.state, &t.f.line.id, false)));
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let r = calls::receipt(&t.f.state, &s.id).await.unwrap();
    assert!(r.answer_dispatch_anchor.is_some());
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
}

#[tokio::test]
async fn confirmed_termination_frees_slot_while_pending_money_holds_and_uncertain_hangup_keeps_it()
{
    let (t, b, _) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v2:slot-pending-quantity==").await;
    let original = calls::receipt(&t.f.state, &s.id).await.unwrap();
    assert!(original.underwriting_hold_id.is_some());
    let db = &t.f.state.db;
    let holds = db.collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME);
    let hold = holds
        .find_one(doc! {"_id":original.underwriting_hold_id.as_deref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    t.server.reset().await;
    coordinator::close(&t.f.state, &b, &s, false).await.unwrap();
    assert!(
        !session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .hangup_confirmed
    );
    assert!(calls::receipt(&t.f.state, &s.id).await.unwrap().slot_held);
    let mut event = occurrence();
    event.call_leg_id = Some(id());
    event.call_session_id = Some(id());
    assert!(
        calls::prepare(
            &t.f.state,
            &b,
            "v2:uncertain-keeps-slot==",
            &s.from,
            &s.to,
            Some(&event)
        )
        .await
        .is_err()
    );
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"result":"ok"}})))
        .mount(&t.server)
        .await;
    coordinator::close(&t.f.state, &b, &s, false).await.unwrap();
    let pending = session::load(&t.f.state, &s.id).await.unwrap();
    assert!(pending.hangup_confirmed && pending.terminated_quantity_pending);
    let receipt = calls::receipt(&t.f.state, &s.id).await.unwrap();
    assert!(!receipt.slot_held && !receipt.financially_finalized);
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    let after = holds
        .find_one(doc! {"_id":&hold.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.held, hold.held);
    assert!(after.recognized_at.is_none());
    let next = prepare_telnyx(&t, &b, "v2:confirmed-releases-slot==").await;
    assert!(
        calls::receipt(&t.f.state, &next.id)
            .await
            .unwrap()
            .slot_held
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(3600));
    // The terminal-observation path releases the second slot too; neither
    // observation is a duration and neither settles or waives managed money.
    let ended = session::terminal(
        &t.f.state,
        &next,
        Utc::now(),
        None,
        "trusted_provider_terminal",
    )
    .await
    .unwrap();
    assert!(ended.terminated_quantity_pending);
    assert!(
        !calls::receipt(&t.f.state, &next.id)
            .await
            .unwrap()
            .slot_held
    );
    db.collection::<bson::Document>(SESSIONS).update_many(doc! {"binding_id":&b.id},doc! {"$set":{"termination_observed_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(4))}}).await.unwrap();
    // Repair a legacy writer's already-terminated row with a retained slot.
    db.collection::<ManagedCallAdmission>(ManagedCallAdmission::COLLECTION_NAME)
        .update_one(
            doc! {"session_id":&next.id},
            doc! {"$set":{"slot_held":true}},
        )
        .await
        .unwrap();
    super::super::make_managed_unready(db).await;
    coordinator::recover_once(&t.f.state).await.unwrap();
    assert!(
        !calls::receipt(&t.f.state, &next.id)
            .await
            .unwrap()
            .slot_held
    );
    assert!(
        session::load(&t.f.state, &s.id)
            .await
            .unwrap()
            .terminated_quantity_pending
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(3600));
}

#[tokio::test]
async fn rate_window_ttl_and_typed_metadata_quarantine_preserve_replay_identity_without_tenant_data()
 {
    let (t, b) = setup_sms().await;
    let id = pending_sms(&t, &b, 1).await;
    sms::recover_one(&t.f.state, &id).await.unwrap();
    let db = &t.f.state.db;
    let rate = db
        .collection::<ManagedLineRateWindow>(ManagedLineRateWindow::COLLECTION_NAME)
        .find_one(doc! {"key":format!("managed-rate:sms:{}",t.f.line.id)})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rate.events.len(), 1);
    assert_eq!(
        (rate.expires_at.unwrap() - rate.events[0].to_chrono()).num_seconds(),
        120
    );
    let encoded = bson::to_document(&rate).unwrap();
    assert!(encoded.get_datetime("expires_at").is_ok());
    assert!(
        encoded.get_array("events").unwrap()[0]
            .as_datetime()
            .is_some()
    );
    let indexes: Vec<_> = db
        .collection::<ManagedLineRateWindow>(ManagedLineRateWindow::COLLECTION_NAME)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(indexes.iter().any(|i| {
        i.options.as_ref().is_some_and(|o| {
            o.name.as_deref() == Some("managed_line_rate_expiry")
                && o.expire_after == Some(std::time::Duration::ZERO)
        })
    }));
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("AccountSid", b.account_id.as_str()),
            ("MessageSid", "SM99999999999999999999999999999999"),
            ("SmsStatus", "delivered"),
            ("NumSegments", "1"),
            ("From", b.phone_number.as_str()),
            ("To", "+18005550999"),
        ])
        .finish();
    let headers = sign(&t, true, &body);
    for _ in 0..2 {
        ingress::receive(
            t.f.state.clone(),
            &t.f.line.id,
            true,
            &headers,
            body.as_bytes(),
        )
        .await
        .unwrap();
    }
    let rows: Vec<_> = db
        .collection::<ManagedLineQuarantine>(ManagedLineQuarantine::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let encoded = bson::to_document(&rows[0]).unwrap();
    assert_eq!(encoded.len(), 6);
    assert!(encoded.get_datetime("occurred_at").is_ok());
    assert!(!format!("{encoded:?}").contains(&b.user_id));
    assert!(!format!("{encoded:?}").contains(&b.phone_number));
    assert_eq!(rows[0].kind, "unknown_sms_receipt");
    let indexes: Vec<_> = db
        .collection::<ManagedLineQuarantine>(ManagedLineQuarantine::COLLECTION_NAME)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        indexes
            .iter()
            .all(|i| i.options.as_ref().is_none_or(|o| o.expire_after.is_none()))
    );
}

#[path = "extras_tests.rs"]
mod extras_tests;
