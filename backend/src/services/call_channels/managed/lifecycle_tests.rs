static COMMIT_FAULT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use super::*;
use crate::models::{
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    service_billing::{BillingMetric, LanePricing, PricingSyncStatus, ServiceBilling},
    usage_meter::{COLLECTION_NAME as USAGE, UsageMeterRow},
};
use crate::services::billing;
use crate::services::call_channels::managed::lifecycle::{
    self, ClaimInput, ContinueInput, ReleaseInput,
};

struct LifecycleFixture {
    f: Fixture,
    server: MockServer,
    actor: AuditActor,
}
async fn setup() -> LifecycleFixture {
    let mut f = fixture().await;
    enable(&f).await;
    let (server, service) = twilio_proof_fixture(&f).await;
    let mut config = f.state.config.clone();
    config.billing_enabled = true;
    config.lago_api_url = Some(server.uri());
    config.lago_api_key = Some("fixture-lago".into());
    f.state = crate::test_utils::test_app_state_with_config(f.state.db.clone(), config);
    let db = &f.state.db;
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&service.id}, &service)
        .await
        .unwrap();
    f.service = service;
    db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.claim.id})
        .await
        .unwrap();
    db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.period.id})
        .await
        .unwrap();
    db.collection::<bson::Document>(USAGE)
        .delete_many(doc! {})
        .await
        .unwrap();
    f.line.state = LineState::Available;
    f.line.current_claim_id = None;
    f.line.next_claim_generation = 1;
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .replace_one(doc! {"_id":&f.line.id}, &f.line)
        .await
        .unwrap();
    insert_rental(&f).await;
    for (service_id, role, metric) in [
        (
            &f.line.rental_billing_service_id,
            ManagedCommsRole::Rental,
            BillingMetric::PhoneNumberDays,
        ),
        (
            &f.line.voice.as_ref().unwrap().billing_service_id,
            ManagedCommsRole::Voice,
            BillingMetric::VoiceSeconds,
        ),
    ] {
        let lane = LanePricing {
            metric,
            credits_per_unit: "1".into(),
            lago_metric_code: format!("managed_{}", metric.as_str()),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: vec![],
            operations: vec![],
            model_prices: vec![],
        };
        db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME).update_one(doc!{"_id":service_id},doc!{"$set":{"managed_comms_role":bson::to_bson(&role).unwrap(),"billing":bson::to_bson(&ServiceBilling{platform_key_pricing:Some(lane.clone()),..Default::default()}).unwrap()}}).await.unwrap();
        db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
            crate::models::billing_rate_cache::COLLECTION_NAME,
        )
        .insert_one(crate::models::billing_rate_cache::BillingRateCache {
            id: crate::models::billing_rate_cache::BillingRateCache::cache_id(
                &lane.lago_metric_code,
                None,
            ),
            lago_metric_code: lane.lago_metric_code,
            model: None,
            credits_per_unit_micros: 1_000_000,
            credits_per_unit_pico: Some(1_000_000_000_000),
            synced_at: Utc::now(),
            retired_at: None,
        })
        .await
        .unwrap();
    }
    let now = bson::DateTime::now();
    db.collection::<bson::Document>(WALLETS).insert_one(doc!{"_id":id(),"owner_id":&f.owner,"lago_customer_id":"customer","lago_wallet_id":"provider-wallet","lago_subscription_id":"subscription","plan_kind":"prepaid","balance_credits":Credits::from_whole(1000),"reserved_credits":Credits::ZERO,"pending_lago_debits":Credits::ZERO,"pending_topup_expiry_credits":Credits::ZERO,"overdraft_cap_credits":Credits::ZERO,"has_payment_instrument":false,"suspended":false,"collection_state":"good","created_at":now,"updated_at":now,"balance_synced_at":now}).await.unwrap();
    billing::taxes::seed_test_profiles(db).await;
    db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME).insert_one(doc!{"_id":&f.claim.agent_id,"user_id":&f.owner,"kind":"specialist","destroyed_at":bson::Bson::Null}).await.unwrap();
    let actor = AuditActor::from_auth_user(&test_auth_user(&f.owner));
    LifecycleFixture { f, server, actor }
}
async fn input(t: &LifecycleFixture) -> ClaimInput {
    let q = lifecycle::quote(&t.f.state, &t.f.owner, &t.f.line.id, QuoteKind::Claim)
        .await
        .unwrap();
    ClaimInput {
        standard_voice: None,
        request_id: id(),
        quote_id: q.id,
        line_id: t.f.line.id.clone(),
        agent_id: t.f.claim.agent_id.clone(),
        voice_tier: VoiceTier::Standard,
        enable_sms: false,
        caller_confirmations: false,
        auto_renew: true,
        accepted_terms_versions: q.terms,
    }
}
async fn claim(t: &LifecycleFixture) -> ManagedLifecycleRequest {
    lifecycle::claim_with_origin(&t.f.state, &t.actor, input(t).await, Some(&t.server.uri()))
        .await
        .unwrap()
}
async fn saved_claim(t: &LifecycleFixture, id: &str) -> ManagedLineClaim {
    t.f.state
        .db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}
async fn periods(t: &LifecycleFixture, id: &str) -> Vec<ManagedLinePeriod> {
    use futures::TryStreamExt;
    t.f.state
        .db
        .collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .find(doc! {"claim_id":id})
        .sort(doc! {"period_index":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn wallet(t: &LifecycleFixture) -> BillingWallet {
    t.f.state
        .db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! {"owner_id":&t.f.owner})
        .await
        .unwrap()
        .unwrap()
}
async fn shift_cutoff(t: &LifecycleFixture, claim: &str, until: chrono::DateTime<Utc>) {
    let timeline = periods(t, claim).await;
    let delta = until - timeline.last().unwrap().covered_until;
    // Move the whole fixture timeline together. Moving only its newest period
    // would leave an older paid interval covering the simulated unpaid cutoff.
    for p in timeline {
        t.f.state.db.collection::<bson::Document>(ManagedLinePeriod::COLLECTION_NAME).update_one(doc!{"_id":&p.id},doc!{"$set":{"covered_from":bson::DateTime::from_chrono(p.covered_from+delta),"covered_until":bson::DateTime::from_chrono(p.covered_until+delta)}}).await.unwrap();
    }
}
async fn renewal_input(t: &LifecycleFixture, claim: &str) -> ContinueInput {
    let c = saved_claim(t, claim).await;
    let p = periods(t, claim).await.pop().unwrap();
    let q = lifecycle::quote(
        &t.f.state,
        &t.f.owner,
        &t.f.line.id,
        QuoteKind::Renewal {
            claim_id: c.id,
            generation: c.managed_claim_generation,
            period_index: p.period_index + 1,
        },
    )
    .await
    .unwrap();
    ContinueInput {
        request_id: id(),
        quote_id: q.id,
        auto_renew: true,
        accepted_terms_versions: q.terms,
    }
}
#[tokio::test]
async fn claim_is_atomic_idempotent_and_payload_bound() {
    let t = setup().await;
    let input = input(&t).await;
    let r =
        lifecycle::claim_with_origin(&t.f.state, &t.actor, input.clone(), Some(&t.server.uri()))
            .await
            .unwrap();
    let again =
        lifecycle::claim_with_origin(&t.f.state, &t.actor, input.clone(), Some(&t.server.uri()))
            .await
            .unwrap();
    assert_eq!(r.id, again.id);
    let mut changed = input;
    changed.auto_renew = false;
    assert!(matches!(
        lifecycle::claim_with_origin(&t.f.state, &t.actor, changed, Some(&t.server.uri())).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    let p = periods(&t, r.claim_id.as_deref().unwrap()).await;
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].quantity, 30);
    assert_eq!(p[0].covered_until - p[0].covered_from, Duration::days(30));
    let rows: Vec<UsageMeterRow> = {
        use futures::TryStreamExt;
        t.f.state
            .db
            .collection::<UsageMeterRow>(USAGE)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].quantity, Some(30));
    assert!(rows[0].released);
    assert_eq!(
        rows[0].funding.as_ref().unwrap().settlement_rate,
        Some(Credits::from_whole(1))
    );
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(30)
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<Binding>(BINDINGS)
            .count_documents(doc! {"enabled":true})
            .await
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn two_replica_claim_race_admits_one_financial_identity() {
    let t = setup().await;
    let a = input(&t).await;
    let b = input(&t).await;
    let origin = t.server.uri();
    let peer = separate_client_state(&t, &format!("managed-claim-peer-{}", id())).await;
    let (a, b) = tokio::join!(
        lifecycle::claim_with_origin(&t.f.state, &t.actor, a, Some(&origin)),
        lifecycle::claim_with_origin(&peer, &t.actor, b, Some(&origin))
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(30)
    );
}
#[tokio::test]
async fn unfunded_claim_rolls_back_quote_claim_binding_budget_and_meter() {
    let t = setup().await;
    let input = input(&t).await;
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(doc! {}, doc! {"$set":{"balance_credits":Credits::ZERO}})
        .await
        .unwrap();
    assert!(
        lifecycle::claim_with_origin(&t.f.state, &t.actor, input.clone(), Some(&t.server.uri()))
            .await
            .is_err()
    );
    for collection in [
        ManagedLineClaim::COLLECTION_NAME,
        ManagedLinePeriod::COLLECTION_NAME,
        ManagedBudgetEntry::COLLECTION_NAME,
        BINDINGS,
        USAGE,
    ] {
        assert_eq!(
            t.f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    let q =
        t.f.state
            .db
            .collection::<ManagedLineQuote>(ManagedLineQuote::COLLECTION_NAME)
            .find_one(doc! {"_id":input.quote_id})
            .await
            .unwrap()
            .unwrap();
    assert!(q.accepted_at.is_none());
}
#[tokio::test]
async fn renewal_commits_full_coverage_before_cutoff_and_replays_once() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    let old = periods(&t, &c).await[0].clone();
    let input = renewal_input(&t, &c).await;
    let r = lifecycle::continue_with_origin(
        &t.f.state,
        &t.actor,
        input.clone(),
        Some(&t.server.uri()),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        lifecycle::continue_with_origin(&t.f.state, &t.actor, input, Some(&t.server.uri()), false)
            .await
            .unwrap()
            .id,
        r.id
    );
    let p = periods(&t, &c).await;
    assert_eq!(p.len(), 2);
    assert_eq!(p[1].covered_from, old.covered_until);
    assert_eq!(p[1].covered_until - p[1].covered_from, Duration::days(30));
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(60)
    );
}
#[tokio::test]
async fn renewal_at_cutoff_is_refused_and_unfunded_quote_never_grants_grace() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    let input = renewal_input(&t, &c).await;
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(doc! {}, doc! {"$set":{"balance_credits":Credits::ZERO}})
        .await
        .unwrap();
    assert!(
        lifecycle::continue_with_origin(
            &t.f.state,
            &t.actor,
            input.clone(),
            Some(&t.server.uri()),
            false
        )
        .await
        .is_err()
    );
    assert_eq!(periods(&t, &c).await.len(), 1);
    shift_cutoff(&t, &c, Utc::now() - Duration::seconds(1)).await;
    assert!(
        lifecycle::continue_with_origin(&t.f.state, &t.actor, input, Some(&t.server.uri()), false)
            .await
            .is_err()
    );
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    let c = saved_claim(&t, &c).await;
    assert_eq!(c.suspension, Some(Suspension::Unpaid));
    assert!(c.suspended_at.unwrap() < Utc::now());
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedLineNotice>(ManagedLineNotice::COLLECTION_NAME)
            .count_documents(doc! {"kind":"unpaid"})
            .await
            .unwrap(),
        1
    );
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedLineNotice>(ManagedLineNotice::COLLECTION_NAME)
            .count_documents(doc! {"kind":"unpaid"})
            .await
            .unwrap(),
        1
    );
}
#[tokio::test]
async fn release_racing_renewal_keeps_only_committed_periods_and_no_refund() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    let renew = renewal_input(&t, &c).await;
    let release = ReleaseInput {
        request_id: id(),
        line_id: t.f.line.id.clone(),
        claim_id: c.clone(),
    };
    let origin = t.server.uri();
    let (renew, release_result) = tokio::join!(
        lifecycle::continue_with_origin(&t.f.state, &t.actor, renew, Some(&origin), false),
        lifecycle::release(&t.f.state, &t.actor, release.clone(), false)
    );
    let release_result = release_result.unwrap();
    assert_eq!(
        lifecycle::release(&t.f.state, &t.actor, release, false)
            .await
            .unwrap()
            .id,
        release_result.id
    );
    let p = periods(&t, &c).await;
    assert_eq!(p.len(), if renew.is_ok() { 2 } else { 1 });
    assert!(p.iter().all(|p| p.funded));
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(30 * p.len() as i64)
    );
    let line =
        t.f.state
            .db
            .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
            .find_one(doc! {"_id":&t.f.line.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(line.state, LineState::Cooldown);
    assert!(line.available_at.unwrap() >= Utc::now() + Duration::days(29));
    assert_eq!(
        t.f.state
            .db
            .collection::<ResourceClaim>(CLAIMS)
            .count_documents(doc! {"user_id":custody::sentinel(&line.id)})
            .await
            .unwrap(),
        3
    );
}
#[tokio::test]
async fn renewal_keeps_racing_admissions_valid_while_suspension_and_resume_refuse() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    let db = &t.f.state.db;
    let credential = crate::models::channel_adapter::CredentialBinding::ManagedLine {
        line_id: t.f.line.id.clone(),
        managed_claim_id: c.clone(),
        managed_claim_generation: saved_claim(&t, &c).await.managed_claim_generation,
    };
    let current = periods(&t, &c).await[0].clone();
    let fences = || async {
        (
            super::counters(db, ManagedLineClaim::COLLECTION_NAME, &c)
                .await
                .0,
            super::counters(db, ManagedPhoneLine::COLLECTION_NAME, &t.f.line.id)
                .await
                .0,
        )
    };
    let before = super::admission::read(db, &t.f.owner, &credential)
        .await
        .unwrap();
    assert_eq!(before.period.id, current.id);
    let unchanged = fences().await;
    let input = renewal_input(&t, &c).await;
    lifecycle::continue_with_origin(&t.f.state, &t.actor, input, Some(&t.server.uri()), false)
        .await
        .unwrap();
    assert_eq!(periods(&t, &c).await.len(), 2);
    // A renewal changes no admission authority: an admission read before it
    // commits still succeeds, covered by the period that covers now.
    assert_eq!(fences().await, unchanged);
    super::admit(db, &before).await.unwrap();
    assert_eq!(
        super::admission::read(db, &t.f.owner, &credential)
            .await
            .unwrap()
            .period
            .id,
        current.id
    );
    // Unpaid suspension and resume advance the authority version, so the
    // earlier read stays refused across both.
    shift_cutoff(&t, &c, Utc::now() - Duration::days(1)).await;
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    let suspended_claim = saved_claim(&t, &c).await;
    assert_eq!(suspended_claim.state, LineState::Suspended);
    let suspended = fences().await;
    assert!(suspended.0 > unchanged.0 && suspended.1 > unchanged.1);
    assert!(super::admit(db, &before).await.is_err());
    let q = lifecycle::quote(
        &t.f.state,
        &t.f.owner,
        &t.f.line.id,
        QuoteKind::Resume {
            claim_id: c.clone(),
            generation: suspended_claim.managed_claim_generation,
        },
    )
    .await
    .unwrap();
    lifecycle::continue_with_origin(
        &t.f.state,
        &t.actor,
        ContinueInput {
            request_id: id(),
            quote_id: q.id,
            auto_renew: true,
            accepted_terms_versions: q.terms,
        },
        Some(&t.server.uri()),
        false,
    )
    .await
    .unwrap();
    let resumed = fences().await;
    assert!(resumed.0 > suspended.0 && resumed.1 > suspended.1);
    assert!(super::admit(db, &before).await.is_err());
    let after = super::admission::read(db, &t.f.owner, &credential)
        .await
        .unwrap();
    super::admit(db, &after).await.unwrap();
}
#[tokio::test]
async fn resume_is_prospective_and_topup_does_not_clear_other_suspensions() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() - Duration::days(1)).await;
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    let claim = saved_claim(&t, &c).await;
    let q = lifecycle::quote(
        &t.f.state,
        &t.f.owner,
        &t.f.line.id,
        QuoteKind::Resume {
            claim_id: c.clone(),
            generation: claim.managed_claim_generation,
        },
    )
    .await
    .unwrap();
    let input = ContinueInput {
        request_id: id(),
        quote_id: q.id,
        auto_renew: true,
        accepted_terms_versions: q.terms,
    };
    let before = Utc::now();
    let r = lifecycle::continue_with_origin(
        &t.f.state,
        &t.actor,
        input.clone(),
        Some(&t.server.uri()),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        lifecycle::continue_with_origin(&t.f.state, &t.actor, input, Some(&t.server.uri()), false)
            .await
            .unwrap()
            .id,
        r.id
    );
    let p = periods(&t, &c).await;
    assert_eq!(p.len(), 2);
    assert!(p[1].covered_from >= before - Duration::milliseconds(1));
    assert_eq!(saved_claim(&t, &c).await.state, LineState::Claimed);
    for suspension in ["admin", "eligibility_revoked"] {
        t.f.state.db.collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c},doc!{"$set":{"state":"suspended","suspension":suspension,"suspended_at":bson::DateTime::now()}}).await.unwrap();
        assert!(
            lifecycle::quote(
                &t.f.state,
                &t.f.owner,
                &t.f.line.id,
                QuoteKind::Resume {
                    claim_id: c.clone(),
                    generation: 1
                }
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn changed_rental_price_requires_new_consent_and_old_quotes_fail_closed() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    let old = renewal_input(&t, &c).await;
    t.f.state.db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME).update_one(doc!{"_id":&t.f.line.rental_billing_service_id},doc!{"$set":{"billing.platform_key_pricing.credits_per_unit":"2"},"$currentDate":{"updated_at":true}}).await.unwrap();
    t.f.state.db.collection::<bson::Document>(crate::models::billing_rate_cache::COLLECTION_NAME).update_one(doc!{"lago_metric_code":"managed_phone_number_days"},doc!{"$set":{"credits_per_unit_micros":2_000_000_i64,"credits_per_unit_pico":2_000_000_000_000_i64}}).await.unwrap();
    assert!(
        lifecycle::continue_with_origin(&t.f.state, &t.actor, old, Some(&t.server.uri()), false)
            .await
            .is_err()
    );
    for _ in 0..3 {
        lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
            .await
            .unwrap();
    }
    assert_eq!(
        t.f.state
            .db
            .collection::<bson::Document>(ManagedLineQuote::COLLECTION_NAME)
            .count_documents(doc! {"automatic_request_id":{"$type":"string"}})
            .await
            .unwrap(),
        0
    );
    let new = renewal_input(&t, &c).await;
    assert!(
        lifecycle::continue_with_origin(
            &t.f.state,
            &t.actor,
            new.clone(),
            Some(&t.server.uri()),
            true
        )
        .await
        .is_err()
    );
    lifecycle::continue_with_origin(&t.f.state, &t.actor, new, Some(&t.server.uri()), false)
        .await
        .unwrap();
    assert_eq!(periods(&t, &c).await[1].rate.rate, Credits::from_whole(2));
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(90)
    );
}
#[tokio::test]
async fn monthly_managed_spend_counts_full_retail_including_benefits() {
    let t = setup().await;
    t.f.state
        .db
        .collection::<bson::Document>(ManagedCommsEligibility::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&t.f.owner},
            doc! {"$set":{"monthly_spend_limit":Credits::from_whole(29)}},
        )
        .await
        .unwrap();
    let now = bson::DateTime::now();
    t.f.state.db.collection::<bson::Document>(crate::models::usage_allowance::COLLECTION_NAME).insert_one(doc!{"_id":id(),"service_id":&t.f.line.rental_billing_service_id,"service_slug":"managed-rental","metric":"phone_number_days","quantity":30_i64,"recurrence":"one_time","target_kind":"all_users","target_user_ids":[],"target_org_ids":[],"target_group_ids":[],"is_active":true,"created_by":"admin","created_at":now,"updated_at":now}).await.unwrap();
    t.f.state
        .db
        .collection::<bson::Document>(crate::models::billing_tax_profile::COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    let input = input(&t).await;
    let err =
        lifecycle::claim_with_origin(&t.f.state, &t.actor, input.clone(), Some(&t.server.uri()))
            .await
            .unwrap_err();
    assert!(matches!(
        err,
        crate::errors::AppError::ManagedBudgetExhausted
    ));
    t.f.state
        .db
        .collection::<bson::Document>(ManagedCommsEligibility::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&t.f.owner},
            doc! {"$set":{"monthly_spend_limit":Credits::from_whole(30)}},
        )
        .await
        .unwrap();
    lifecycle::claim_with_origin(&t.f.state, &t.actor, input, Some(&t.server.uri()))
        .await
        .unwrap();
    let entry =
        t.f.state
            .db
            .collection::<ManagedBudgetEntry>(ManagedBudgetEntry::COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(entry.recognized, Some(Credits::from_whole(30)));
}
#[tokio::test]
async fn unpaid_retention_auto_release_cooldown_and_operator_approval() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() - Duration::days(8)).await;
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    let lines =
        t.f.state
            .db
            .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME);
    let line = lines
        .find_one(doc! {"_id":&t.f.line.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line.state, LineState::Cooldown);
    assert!(
        lifecycle::approve_reassignment(&t.f.state, &t.f.admin, &line.id, line.revision)
            .await
            .is_err()
    );
    t.f.state.db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&line.id},doc!{"$set":{"available_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    let ready = lines
        .find_one(doc! {"_id":&line.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready.state, LineState::PendingReassignment);
    assert!(
        lifecycle::quote(&t.f.state, &t.f.owner, &line.id, QuoteKind::Claim)
            .await
            .is_err()
    );
    assert!(
        lifecycle::approve_reassignment(&t.f.state, &t.actor, &line.id, ready.revision)
            .await
            .is_err()
    );
    lifecycle::approve_reassignment(&t.f.state, &t.f.admin, &line.id, ready.revision)
        .await
        .unwrap();
    assert_eq!(
        lines
            .find_one(doc! {"_id":&line.id})
            .await
            .unwrap()
            .unwrap()
            .state,
        LineState::Available
    );
}
#[tokio::test]
async fn flag_off_lapses_without_charge_and_release_remains_available() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() - Duration::seconds(1)).await;
    flags::set_platform_override(
        &t.f.state.db,
        flags::MANAGED_LINES_FLAG_KEY,
        &flags::FlagTarget::User(t.f.owner.clone()),
        false,
        &t.f.admin.user_id,
    )
    .await
    .unwrap();
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    assert_eq!(
        saved_claim(&t, &c).await.suspension,
        Some(Suspension::Unpaid)
    );
    assert_eq!(periods(&t, &c).await.len(), 1);
    lifecycle::release(
        &t.f.state,
        &t.actor,
        ReleaseInput {
            request_id: id(),
            line_id: t.f.line.id.clone(),
            claim_id: c,
        },
        false,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn erasure_preparation_fences_renewal_drains_effects_and_retains_financial_custody() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    let renewal = renewal_input(&t, &c).await;
    let route = id();
    let effect = id();
    t.f.state.db.collection::<bson::Document>(ManagedEffectRoute::COLLECTION_NAME).insert_one(doc!{"_id":&route,"managed_claim_id":&c,"financially_finalized":false,"effect_id":&effect,"provider_effect_id":"retained-provider-id"}).await.unwrap();
    lifecycle::prepare_erasure(&t.f.state.db, &t.f.owner)
        .await
        .unwrap();
    assert_eq!(saved_claim(&t, &c).await.state, LineState::Releasing);
    assert_eq!(
        t.f.state
            .db
            .collection::<Binding>(BINDINGS)
            .count_documents(doc! {"enabled":true})
            .await
            .unwrap(),
        0
    );
    assert!(
        lifecycle::continue_with_origin(
            &t.f.state,
            &t.actor,
            renewal,
            Some(&t.server.uri()),
            false
        )
        .await
        .is_err()
    );
    let mut tx = t.f.state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert!(
        lifecycle::erasure_guard(&t.f.state.db, &t.f.owner, &mut tx)
            .await
            .is_err()
    );
    tx.abort_transaction().await.unwrap();
    t.f.state
        .db
        .collection::<bson::Document>(ManagedEffectRoute::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&route},
            doc! {"$set":{"financially_finalized":true}},
        )
        .await
        .unwrap();
    lifecycle::prepare_erasure(&t.f.state.db, &t.f.owner)
        .await
        .unwrap();
    assert_eq!(saved_claim(&t, &c).await.state, LineState::Cooldown);
    assert!(
        !lifecycle::erase_personal_fields(&t.f.state.db, &t.f.owner)
            .await
            .unwrap()
    );
    assert!(
        lifecycle::erase_personal_fields(&t.f.state.db, &t.f.owner)
            .await
            .unwrap()
    );
    let erased = saved_claim(&t, &c).await;
    assert_eq!(erased.user_id, "");
    assert_eq!(erased.agent_id, "");
    assert!(erased.voice_binding_id.is_none());
    assert_eq!(periods(&t, &c).await.len(), 1);
    assert!(
        t.f.state
            .db
            .collection::<bson::Document>(ManagedEffectRoute::COLLECTION_NAME)
            .find_one(doc! {"_id":route})
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<ResourceClaim>(CLAIMS)
            .count_documents(doc! {"user_id":custody::sentinel(&t.f.line.id)})
            .await
            .unwrap(),
        3
    );
}
async fn separate_client_state(t: &LifecycleFixture, app: &str) -> AppState {
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.app_name = Some(app.into());
    let client = mongodb::Client::with_options(options).unwrap();
    crate::test_utils::test_app_state_with_config(
        client.database(t.f.state.db.name()),
        t.f.state.config.clone(),
    )
}
async fn uncertain_commit(t: &LifecycleFixture, app: &str) {
    t.f.state.db.client().database("admin").run_command(doc!{"configureFailPoint":"failCommand","mode":{"times":1},"data":{"failCommands":["commitTransaction"],"appName":app,"writeConcernError":{"code":64,"errmsg":"fixture commit acknowledgement lost","errInfo":{"wtimeout":true}},"errorLabels":["UnknownTransactionCommitResult"]}}).await.expect("dedicated test replica requires enableTestCommands=1");
}
#[tokio::test]
async fn claim_unknown_commit_recovers_original_request_identity() {
    let _fault_guard = COMMIT_FAULT.lock().await;
    let t = setup().await;
    let app = format!("managed-unknown-{}", id());
    let peer = separate_client_state(&t, &app).await;
    let input = input(&t).await;
    uncertain_commit(&t, &app).await;
    let r = lifecycle::claim_with_origin(&peer, &t.actor, input.clone(), Some(&t.server.uri()))
        .await
        .unwrap();
    assert_eq!(
        lifecycle::claim_with_origin(&peer, &t.actor, input, Some(&t.server.uri()))
            .await
            .unwrap()
            .id,
        r.id
    );
    assert_eq!(periods(&t, r.claim_id.as_deref().unwrap()).await.len(), 1);
    assert_eq!(
        wallet(&t).await.pending_lago_debits,
        Credits::from_whole(30)
    );
}
#[tokio::test]
async fn renewal_unknown_commit_recovers_before_another_period_or_suspension() {
    let _fault_guard = COMMIT_FAULT.lock().await;
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    let input = renewal_input(&t, &c).await;
    let app = format!("managed-renew-unknown-{}", id());
    let peer = separate_client_state(&t, &app).await;
    uncertain_commit(&t, &app).await;
    let r = lifecycle::continue_with_origin(
        &peer,
        &t.actor,
        input.clone(),
        Some(&t.server.uri()),
        false,
    )
    .await
    .unwrap();
    flags::set_platform_override(
        &t.f.state.db,
        flags::MANAGED_LINES_FLAG_KEY,
        &flags::FlagTarget::User(t.f.owner.clone()),
        false,
        &t.f.admin.user_id,
    )
    .await
    .unwrap();
    assert_eq!(
        lifecycle::continue_with_origin(&peer, &t.actor, input, Some(&t.server.uri()), false)
            .await
            .unwrap()
            .id,
        r.id
    );
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    assert_eq!(saved_claim(&t, &c).await.state, LineState::Claimed);
    assert_eq!(periods(&t, &c).await.len(), 2);
}
#[tokio::test]
async fn release_and_resume_unknown_commits_remain_idempotent() {
    let _fault_guard = COMMIT_FAULT.lock().await;
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() - Duration::hours(1)).await;
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    let q = lifecycle::quote(
        &t.f.state,
        &t.f.owner,
        &t.f.line.id,
        QuoteKind::Resume {
            claim_id: c.clone(),
            generation: 1,
        },
    )
    .await
    .unwrap();
    let input = ContinueInput {
        request_id: id(),
        quote_id: q.id,
        auto_renew: false,
        accepted_terms_versions: q.terms,
    };
    let app = format!("managed-resume-unknown-{}", id());
    let peer = separate_client_state(&t, &app).await;
    uncertain_commit(&t, &app).await;
    let r = lifecycle::continue_with_origin(
        &peer,
        &t.actor,
        input.clone(),
        Some(&t.server.uri()),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        lifecycle::continue_with_origin(&peer, &t.actor, input, Some(&t.server.uri()), false)
            .await
            .unwrap()
            .id,
        r.id
    );
    uncertain_commit(&t, &app).await;
    let input = ReleaseInput {
        request_id: id(),
        line_id: t.f.line.id.clone(),
        claim_id: c.clone(),
    };
    let r = lifecycle::release(&peer, &t.actor, input.clone(), false)
        .await
        .unwrap();
    assert_eq!(
        lifecycle::release(&peer, &t.actor, input, false)
            .await
            .unwrap()
            .id,
        r.id
    );
    assert_eq!(periods(&t, &c).await.len(), 2);
}

#[tokio::test]
async fn renewal_sweep_precommits_once_and_unfunded_pending_renewal_stays_uncovered() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(2)).await;
    lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
        .await
        .unwrap();
    assert_eq!(periods(&t, &c).await.len(), 2);
    lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
        .await
        .unwrap();
    assert_eq!(periods(&t, &c).await.len(), 2);
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedLineNotice>(ManagedLineNotice::COLLECTION_NAME)
            .count_documents(doc! {"kind":"renewal"})
            .await
            .unwrap(),
        1
    );
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(1)).await;
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(doc! {}, doc! {"$set":{"balance_credits":Credits::ZERO}})
        .await
        .unwrap();
    lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
        .await
        .unwrap();
    assert_eq!(periods(&t, &c).await.len(), 2);
    assert_eq!(saved_claim(&t, &c).await.state, LineState::Claimed);
    shift_cutoff(&t, &c, Utc::now() - Duration::milliseconds(1)).await;
    lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
        .await
        .unwrap();
    assert_eq!(
        saved_claim(&t, &c).await.suspension,
        Some(Suspension::Unpaid)
    );
    assert_eq!(periods(&t, &c).await.len(), 2);
}

fn mandarin() -> StandardVoiceOption {
    StandardVoiceOption {
        voice: "cmn-CN-Standard-A".into(),
        language: "cmn-CN".into(),
    }
}
#[tokio::test]
async fn claim_voice_requires_standard_live_allowlist_and_quote_consent() {
    let t = setup().await;
    let mut denied = input(&t).await;
    denied.standard_voice = Some(mandarin());
    assert!(
        lifecycle::claim_with_origin(&t.f.state, &t.actor, denied, Some(&t.server.uri()))
            .await
            .is_err()
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    super::voice::configure(
        &t.f.state,
        &t.f.admin,
        &t.f.line.id,
        super::voice::AllowlistInput {
            revision: 1,
            options: vec![super::voice::default_option("twilio"), mandarin()],
        },
    )
    .await
    .unwrap();
    let mut accepted = input(&t).await;
    accepted.standard_voice = Some(mandarin());
    let receipt =
        lifecycle::claim_with_origin(&t.f.state, &t.actor, accepted, Some(&t.server.uri()))
            .await
            .unwrap();
    let claim = saved_claim(&t, receipt.claim_id.as_ref().unwrap()).await;
    assert_eq!(claim.standard_voice, Some(mandarin()));
    let binding =
        t.f.state
            .db
            .collection::<Binding>(BINDINGS)
            .find_one(doc! {"_id":claim.voice_binding_id.as_ref().unwrap()})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(binding.voice, mandarin().voice);
    assert_eq!(binding.language, mandarin().language);
    crate::services::call_channels::binding::validate_voice(&binding).unwrap();
    assert!(
        super::voice::validate_option(
            "twilio",
            &StandardVoiceOption {
                voice: "cmn-CN-Wavenet-A".into(),
                language: "cmn-CN".into()
            }
        )
        .is_err()
    );
    assert!(
        super::voice::validate_option(
            "telnyx",
            &StandardVoiceOption {
                voice: "elevenlabs-premium".into(),
                language: "zh-CN".into()
            }
        )
        .is_err()
    );
}
#[tokio::test]
async fn standard_voice_change_is_idle_only_revisioned_and_metadata_audited() {
    let t = setup().await;
    super::voice::configure(
        &t.f.state,
        &t.f.admin,
        &t.f.line.id,
        super::voice::AllowlistInput {
            revision: 1,
            options: vec![super::voice::default_option("twilio"), mandarin()],
        },
    )
    .await
    .unwrap();
    let receipt = claim(&t).await;
    let c = saved_claim(&t, receipt.claim_id.as_ref().unwrap()).await;
    let binding = c.voice_binding_id.as_ref().unwrap();
    let call = id();
    t.f.state
        .db
        .collection::<bson::Document>(SESSIONS)
        .insert_one(
            doc! {"_id":&call,"binding_id":binding,"managed_claim_id":&c.id,"state":"active"},
        )
        .await
        .unwrap();
    assert!(
        super::voice::select(
            &t.f.state,
            &t.actor,
            &t.f.line.id,
            super::voice::SelectionInput {
                claim_id: c.id.clone(),
                binding_revision: 1,
                option: mandarin()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<Binding>(BINDINGS)
            .find_one(doc! {"_id":binding})
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    t.f.state
        .db
        .collection::<bson::Document>(SESSIONS)
        .update_one(doc! {"_id":&call}, doc! {"$set":{"state":"closed"}})
        .await
        .unwrap();
    super::voice::select(
        &t.f.state,
        &t.actor,
        &t.f.line.id,
        super::voice::SelectionInput {
            claim_id: c.id.clone(),
            binding_revision: 1,
            option: mandarin(),
        },
    )
    .await
    .unwrap();
    let b =
        t.f.state
            .db
            .collection::<Binding>(BINDINGS)
            .find_one(doc! {"_id":binding})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(b.revision, 2);
    assert_eq!(b.language, "cmn-CN");
    let audit =
        t.f.state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"managed_line_standard_voice_changed"})
            .await
            .unwrap()
            .unwrap();
    let encoded = serde_json::to_string(&audit).unwrap();
    assert!(!encoded.contains("cmn-CN"));
    assert!(!encoded.contains("fixture-secret"));
}
#[tokio::test]
async fn quotes_are_throttled_and_only_unaccepted_quotes_have_ttl() {
    let t = setup().await;
    let receipt = claim(&t).await;
    let accepted =
        t.f.state
            .db
            .collection::<bson::Document>(ManagedLineQuote::COLLECTION_NAME)
            .find_one(doc! {"accepted_at":{"$type":"date"}})
            .await
            .unwrap()
            .unwrap();
    assert!(!accepted.contains_key("expires_at"));
    shift_cutoff(
        &t,
        receipt.claim_id.as_ref().unwrap(),
        Utc::now() + Duration::hours(2),
    )
    .await;
    let c = saved_claim(&t, receipt.claim_id.as_ref().unwrap()).await;
    let kind = QuoteKind::Renewal {
        claim_id: c.id.clone(),
        generation: 1,
        period_index: 1,
    };
    for _ in 0..8 {
        let q = lifecycle::quote(&t.f.state, &t.f.owner, &t.f.line.id, kind.clone())
            .await
            .unwrap();
        assert!(q.automatic_request_id.is_none());
        assert!(q.expires_at.unwrap() > q.accept_before);
    }
    t.f.state.db.collection::<bson::Document>(crate::models::coordination::TOKEN_BUCKET_COLLECTION_NAME).update_many(doc!{"namespace":"managed-line-lifecycle"},doc!{"$set":{"tokens_millis":0_i64,"last_refill_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(1))}}).await.unwrap();
    assert!(matches!(
        lifecycle::quote(&t.f.state, &t.f.owner, &t.f.line.id, kind).await,
        Err(crate::errors::AppError::RateLimited)
    ));
    let mut indexes =
        t.f.state
            .db
            .collection::<bson::Document>(ManagedLineQuote::COLLECTION_NAME)
            .list_indexes()
            .await
            .unwrap();
    let mut found = false;
    use futures::TryStreamExt;
    while let Some(index) = indexes.try_next().await.unwrap() {
        if index
            .options
            .as_ref()
            .is_some_and(|o| o.name.as_deref() == Some("managed_unaccepted_quote_expiry"))
        {
            assert_eq!(
                index.options.unwrap().expire_after,
                Some(std::time::Duration::ZERO)
            );
            found = true;
        }
    }
    assert!(found);
}
#[tokio::test]
async fn sweep_does_not_mint_manual_quotes_or_rotate_automatic_quotes() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(2)).await;
    let quotes =
        t.f.state
            .db
            .collection::<bson::Document>(ManagedLineQuote::COLLECTION_NAME);
    t.f.state
        .db
        .collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(doc! {"_id":&c}, doc! {"$set":{"auto_renew":false}})
        .await
        .unwrap();
    for _ in 0..3 {
        lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
            .await
            .unwrap();
    }
    assert_eq!(
        quotes
            .count_documents(doc! {"kind.kind":"renewal"})
            .await
            .unwrap(),
        0
    );
    let manual = renewal_input(&t, &c).await;
    assert!(
        quotes
            .find_one(doc! {"_id":manual.quote_id})
            .await
            .unwrap()
            .unwrap()
            .get("automatic_request_id")
            .is_none_or(|v| v == &bson::Bson::Null)
    );
    t.f.state
        .db
        .collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(doc! {"_id":&c}, doc! {"$set":{"auto_renew":true}})
        .await
        .unwrap();
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(doc! {}, doc! {"$set":{"balance_credits":Credits::ZERO}})
        .await
        .unwrap();
    lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
        .await
        .unwrap();
    let auto = quotes
        .find_one(doc! {"automatic_request_id":{"$type":"string"}})
        .await
        .unwrap()
        .unwrap();
    assert!(
        auto.get_datetime("accept_before").unwrap().to_chrono() > Utc::now() + Duration::hours(1)
    );
    quotes.update_one(doc!{"_id":auto.get_str("_id").unwrap()},doc!{"$set":{"accept_before":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    for _ in 0..3 {
        lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
            .await
            .unwrap();
    }
    assert_eq!(
        quotes
            .count_documents(doc! {"automatic_request_id":{"$type":"string"}})
            .await
            .unwrap(),
        1
    );
    assert_eq!(periods(&t, &c).await.len(), 1);
}
#[tokio::test]
async fn completed_erasure_removes_managed_preparation_marker_and_owner_fields() {
    use crate::services::personal_erasure_service as erasure;
    let t = setup().await;
    let claim = claim(&t).await.claim_id.unwrap();
    erasure::ensure_indexes(&t.f.state.db).await.unwrap();
    for _ in 0..64 {
        match erasure::accept(
            &t.f.state.db,
            &t.f.owner,
            &t.f.owner,
            Some("machine-data-provider-revocation-v2"),
        )
        .await
        {
            Ok(_) => break,
            Err(crate::errors::AppError::ErasurePreparing) => {
                erasure::prepare_one_for_test(&t.f.state.db, &t.f.state.encryption_keys)
                    .await
                    .unwrap();
            }
            Err(e) => panic!("erasure acceptance: {e}"),
        }
    }
    let marker =
        t.f.state
            .db
            .collection::<bson::Document>(ManagedLineErasure::COLLECTION_NAME);
    assert_eq!(
        marker
            .count_documents(doc! {"user_id":&t.f.owner})
            .await
            .unwrap(),
        1
    );
    for _ in 0..160 {
        if t.f
            .state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&t.f.owner})
            .await
            .unwrap()
            .is_none()
        {
            break;
        }
        t.f.state
            .db
            .collection::<bson::Document>(crate::models::personal_erasure::COLLECTION_NAME)
            .update_many(doc! {}, doc! {"$set":{"retry_at":bson::DateTime::now()}})
            .await
            .unwrap();
        erasure::process_one(&t.f.state).await.unwrap();
    }
    // Empty local inventory does not attest vendor history. Complete the same
    // explicit inventory reconciliation required by the erasure integration tests.
    crate::services::telemetry_erasure_service::drain_once(&t.f.state.db, &Default::default())
        .await
        .unwrap();
    crate::services::telemetry_erasure_service::operator_reconcile_empty_inventory(
        &t.f.state.db,
        &t.f.owner,
        &t.f.admin.user_id,
        "evidence://managed-erasure-test-inventory",
        "Verified empty fixture vendor inventory",
    )
    .await
    .unwrap();
    for _ in 0..10 {
        t.f.state
            .db
            .collection::<bson::Document>(crate::models::personal_erasure::COLLECTION_NAME)
            .update_many(doc! {}, doc! {"$set":{"retry_at":bson::DateTime::now()}})
            .await
            .unwrap();
        erasure::process_one(&t.f.state).await.unwrap();
        if erasure::status(&t.f.state.db, &t.f.owner)
            .await
            .unwrap()
            .unwrap()
            .status
            == crate::models::personal_erasure::ErasureStatus::Complete
        {
            break;
        }
    }
    let job = erasure::status(&t.f.state.db, &t.f.owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        job.status,
        crate::models::personal_erasure::ErasureStatus::Complete,
        "{job:?}"
    );
    for collection in [
        ManagedLineErasure::COLLECTION_NAME,
        ManagedCommsEligibility::COLLECTION_NAME,
        ManagedLineQuote::COLLECTION_NAME,
        ManagedLineNotice::COLLECTION_NAME,
        ManagedLineClaim::COLLECTION_NAME,
    ] {
        assert_eq!(
            t.f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {"user_id":&t.f.owner})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    assert_eq!(saved_claim(&t, &claim).await.user_id, "");
    // Documented immutable financial attribution survives personal deletion.
    assert!(
        t.f.state
            .db
            .collection::<bson::Document>(ManagedLifecycleRequest::COLLECTION_NAME)
            .find_one(doc! {"actor_id":&t.f.owner})
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(periods(&t, &claim).await.len(), 1);
}

#[path = "admin_line_tests.rs"]
mod admin_line_tests;
#[path = "effects_tests.rs"]
mod effects_tests;
