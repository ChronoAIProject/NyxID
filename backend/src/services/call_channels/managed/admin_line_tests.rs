//! M7 admin line operations: suspension, resume, retirement and revalidated edits.
use super::*;
use crate::services::call_channels::managed::{
    admission, costs,
    custody::{RegulatoryInput, SupplierTariffInput, UpdateInput},
    lifecycle::{LineRevisionInput, RetireInput},
    underwriting,
};
use serde_json::json;

async fn saved_line(t: &LifecycleFixture) -> ManagedPhoneLine {
    t.f.state
        .db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":&t.f.line.id})
        .await
        .unwrap()
        .unwrap()
}
async fn claim_bindings(t: &LifecycleFixture, claim: &str) -> Vec<Binding> {
    use futures::TryStreamExt;
    t.f.state
        .db
        .collection::<Binding>(BINDINGS)
        .find(doc! {"credential.managed_claim_id":claim})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn audit(t: &LifecycleFixture, event: &str) -> bson::Document {
    t.f.state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":event})
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("missing {event} audit"))
}
fn update_input(revision: i64, slots: i64) -> UpdateInput {
    UpdateInput {
        revision,
        regulatory: RegulatoryInput {
            requirement_reference: "operator-review-2".into(),
            registration_status: "approved".into(),
            sms_sender_registration: None,
        },
        supplier_tariffs: vec![SupplierTariffInput {
            kind: CostKind::CarrierInbound,
            amount: "0.0085".into(),
            currency: "USD".into(),
            unit: "minute".into(),
            increment: 60,
            minimum: 60,
            source: "https://www.twilio.com/en-us/voice/pricing/us".into(),
            revision: 1,
            fx_policy_revision: "fx-1".into(),
            effective_at: Utc::now(),
        }],
        call_slot_limit: slots,
        calls_per_minute: 20,
        sms_per_minute: 20,
    }
}

#[tokio::test]
async fn admin_suspend_fences_admission_and_resume_restores_or_lapses_without_charge() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    let line = saved_line(&t).await;
    let input = |revision| LineRevisionInput { revision };
    assert!(
        lifecycle::suspend(&t.f.state, &t.actor, &line.id, input(line.revision))
            .await
            .is_err(),
        "merchants cannot suspend platform inventory"
    );
    assert!(
        lifecycle::suspend(&t.f.state, &t.f.admin, &line.id, input(line.revision - 1))
            .await
            .is_err(),
        "stale revisions are refused"
    );
    let suspended = lifecycle::suspend(&t.f.state, &t.f.admin, &line.id, input(line.revision))
        .await
        .unwrap();
    assert_eq!(suspended.state, LineState::Suspended);
    assert_eq!(suspended.suspension, Some(Suspension::Admin));
    assert_eq!(suspended.claim_id.as_deref(), Some(c.as_str()));
    assert!(!suspended.termination_pending);
    let saved = saved_claim(&t, &c).await;
    assert_eq!(
        (saved.state, saved.suspension),
        (LineState::Suspended, Some(Suspension::Admin))
    );
    let bindings = claim_bindings(&t, &c).await;
    assert!(!bindings.is_empty() && bindings.iter().all(|b| !b.enabled));
    assert!(
        admission::read(&t.f.state.db, &t.f.owner, &bindings[0].credential)
            .await
            .is_err()
    );
    let line = saved_line(&t).await;
    assert!(
        lifecycle::retire(
            &t.f.state,
            &t.f.admin,
            &line.id,
            RetireInput {
                revision: line.revision,
                supplier_released_at: None,
            },
        )
        .await
        .is_err(),
        "claimed inventory must be released before retirement"
    );
    let resumed = lifecycle::resume(&t.f.state, &t.f.admin, &line.id, input(line.revision))
        .await
        .unwrap();
    assert_eq!(
        (resumed.state, resumed.suspension),
        (LineState::Claimed, None)
    );
    assert!(claim_bindings(&t, &c).await.iter().all(|b| b.enabled));
    let bindings = claim_bindings(&t, &c).await;
    admission::read(&t.f.state.db, &t.f.owner, &bindings[0].credential)
        .await
        .unwrap();

    // Coverage that ends during an admin suspension is not renewed, extended or
    // back-charged: resume leaves an unpaid claim with a fresh retention window.
    let line = saved_line(&t).await;
    lifecycle::suspend(&t.f.state, &t.f.admin, &line.id, input(line.revision))
        .await
        .unwrap();
    shift_cutoff(&t, &c, Utc::now() - Duration::days(9)).await;
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    assert_eq!(
        saved_claim(&t, &c).await.suspension,
        Some(Suspension::Admin)
    );
    let line = saved_line(&t).await;
    let lapsed = lifecycle::resume(&t.f.state, &t.f.admin, &line.id, input(line.revision))
        .await
        .unwrap();
    assert_eq!(
        (lapsed.state, lapsed.suspension),
        (LineState::Suspended, Some(Suspension::Unpaid))
    );
    let saved = saved_claim(&t, &c).await;
    assert!(saved.suspended_at.unwrap() > Utc::now() - Duration::minutes(1));
    assert_eq!(periods(&t, &c).await.len(), 1);
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedLineNotice>(ManagedLineNotice::COLLECTION_NAME)
            .count_documents(doc! {"key":format!("unpaid:{c}:0")})
            .await
            .unwrap(),
        1
    );
    lifecycle::sweep_once(&t.f.state).await.unwrap();
    assert_eq!(
        saved_claim(&t, &c).await.state,
        LineState::Suspended,
        "retention restarts at resume and is not consumed by the suspension"
    );
    for event in ["managed_line_suspended", "managed_line_admin_resumed"] {
        let row = audit(&t, event).await;
        let encoded = serde_json::to_string(&row).unwrap();
        assert!(encoded.contains(&line.id));
        assert!(!encoded.contains(&t.f.line.phone_number));
    }
}

#[tokio::test]
async fn retire_requires_unclaimed_inventory_and_records_supplier_release_once() {
    let t = setup().await;
    let line = saved_line(&t).await;
    let custody_before =
        t.f.state
            .db
            .collection::<bson::Document>(CLAIMS)
            .count_documents(doc! {"user_id":custody::sentinel(&line.id)})
            .await
            .unwrap();
    let retire = |revision, supplier_released_at| RetireInput {
        revision,
        supplier_released_at,
    };
    assert!(
        lifecycle::retire(
            &t.f.state,
            &t.f.admin,
            &line.id,
            retire(line.revision, Some(Utc::now() + Duration::days(1))),
        )
        .await
        .is_err(),
        "a future supplier release is refused"
    );
    assert!(
        lifecycle::retire(&t.f.state, &t.actor, &line.id, retire(line.revision, None))
            .await
            .is_err()
    );
    let retired = lifecycle::retire(
        &t.f.state,
        &t.f.admin,
        &line.id,
        retire(line.revision, None),
    )
    .await
    .unwrap();
    assert_eq!(retired.state, LineState::Retired);
    assert!(
        lifecycle::quote(&t.f.state, &t.f.owner, &line.id, QuoteKind::Claim)
            .await
            .is_err()
    );
    let line = saved_line(&t).await;
    assert!(
        lifecycle::retire(
            &t.f.state,
            &t.f.admin,
            &line.id,
            retire(line.revision, None)
        )
        .await
        .is_err()
    );
    let released = Utc::now() - Duration::hours(1);
    lifecycle::retire(
        &t.f.state,
        &t.f.admin,
        &line.id,
        retire(line.revision, Some(released)),
    )
    .await
    .unwrap();
    let line = saved_line(&t).await;
    assert_eq!(line.state, LineState::Retired);
    assert!(line.supplier_released_at.is_some());
    assert!(
        lifecycle::retire(
            &t.f.state,
            &t.f.admin,
            &line.id,
            retire(line.revision, Some(released)),
        )
        .await
        .is_err(),
        "supplier release is recorded once"
    );
    // Retirement and supplier release never remove platform custody records.
    assert_eq!(
        t.f.state
            .db
            .collection::<bson::Document>(CLAIMS)
            .count_documents(doc! {"user_id":custody::sentinel(&line.id)})
            .await
            .unwrap(),
        custody_before
    );
    audit(&t, "managed_line_retired").await;
    audit(&t, "managed_line_supplier_released").await;
    assert!(
        custody::update_with_origin(
            &t.f.state,
            &t.f.admin,
            &line.id,
            update_input(line.revision, 2),
            Some(&t.server.uri()),
        )
        .await
        .is_err(),
        "retired lines cannot be edited"
    );
}

#[tokio::test]
async fn admin_edit_reproves_provider_and_keeps_accepted_auto_renewal() {
    let t = setup().await;
    let c = claim(&t).await.claim_id.unwrap();
    let line = saved_line(&t).await;
    assert!(
        custody::update_with_origin(
            &t.f.state,
            &t.actor,
            &line.id,
            update_input(line.revision, 2),
            Some(&t.server.uri()),
        )
        .await
        .is_err()
    );
    assert!(
        custody::update_with_origin(
            &t.f.state,
            &t.f.admin,
            &line.id,
            update_input(line.revision - 1, 2),
            Some(&t.server.uri()),
        )
        .await
        .is_err()
    );
    let mut invalid = update_input(line.revision, 2);
    invalid.supplier_tariffs.clear();
    assert!(
        custody::update_with_origin(
            &t.f.state,
            &t.f.admin,
            &line.id,
            invalid,
            Some(&t.server.uri()),
        )
        .await
        .is_err()
    );
    let updated = custody::update_with_origin(
        &t.f.state,
        &t.f.admin,
        &line.id,
        update_input(line.revision, 2),
        Some(&t.server.uri()),
    )
    .await
    .unwrap();
    assert_eq!(updated.revision, line.revision + 1);
    let saved = saved_line(&t).await;
    assert_eq!(
        (
            saved.call_slot_limit,
            saved.calls_per_minute,
            saved.revision
        ),
        (2, 20, line.revision + 1)
    );
    assert_eq!(saved.regulatory.verified_by, t.f.admin.user_id);
    assert_eq!(saved.supplier_tariffs.len(), 1);
    assert_eq!(saved.phone_number, line.phone_number);
    assert_eq!(saved.current_claim_id.as_deref(), Some(c.as_str()));
    // The automatic renewal quote is minted after the edit within the renewal
    // window, then the line is edited again; accepted auto-renewal still funds.
    shift_cutoff(&t, &c, Utc::now() + Duration::hours(2)).await;
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(doc! {}, doc! {"$set":{"balance_credits":Credits::ZERO}})
        .await
        .unwrap();
    lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
        .await
        .unwrap();
    assert_eq!(periods(&t, &c).await.len(), 1);
    let saved = saved_line(&t).await;
    custody::update_with_origin(
        &t.f.state,
        &t.f.admin,
        &line.id,
        update_input(saved.revision, 3),
        Some(&t.server.uri()),
    )
    .await
    .unwrap();
    t.f.state
        .db
        .collection::<bson::Document>(WALLETS)
        .update_many(
            doc! {},
            doc! {"$set":{"balance_credits":Credits::from_whole(1000)}},
        )
        .await
        .unwrap();
    lifecycle::sweep_with_origin(&t.f.state, Some(&t.server.uri()))
        .await
        .unwrap();
    assert_eq!(periods(&t, &c).await.len(), 2);
    let row = audit(&t, "managed_line_updated").await;
    assert!(
        !serde_json::to_string(&row)
            .unwrap()
            .contains("operator-review-2")
    );
}

async fn account_row(db: &mongodb::Database, id: &str) -> ProviderAccount {
    db.collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn retired_telnyx_voice_lines_free_registration_and_underwriting_capacity() {
    let f = super::super::fixture().await;
    let db = &f.state.db;
    // Only Telnyx voice lines reserve underwriting capacity in this fixture.
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.line.id})
        .await
        .unwrap();
    let mut account = f.account.clone();
    account.provider = "telnyx".into();
    let mut policy = super::super::underwriting_policy();
    policy.rolling_budget = Credits::from_whole(720);
    account.underwriting = Some(policy);
    db.collection::<ProviderAccount>(ACCOUNTS)
        .replace_one(doc! {"_id":&account.id}, &account)
        .await
        .unwrap();
    super::super::insert_rental(&f).await;
    let telnyx_line = |phone: &str| {
        let mut line = f.line.clone();
        line.id = id();
        line.provider = "telnyx".into();
        line.number_resource_id = id();
        line.phone_number = phone.into();
        line.voice.as_mut().unwrap().application_id = id();
        line.provider_channel_limit = Some(2);
        line.state = LineState::Available;
        line.current_claim_id = None;
        line
    };
    let first = telnyx_line("+18005550131");
    custody::persist(&f.state, &f.admin, &first, &account, &f.service)
        .await
        .unwrap();
    let second = telnyx_line("+18005550132");
    let account = account_row(db, &account.id).await;
    assert!(
        custody::persist(&f.state, &f.admin, &second, &account, &f.service)
            .await
            .is_err(),
        "two live lines of two channels exceed the budget"
    );
    lifecycle::retire(
        &f.state,
        &f.admin,
        &first.id,
        RetireInput {
            revision: first.revision,
            supplier_released_at: None,
        },
    )
    .await
    .unwrap();
    let account = account_row(db, &account.id).await;
    custody::persist(&f.state, &f.admin, &second, &account, &f.service)
        .await
        .unwrap();
    // The underwriting budget check shares the selector: only the second
    // line's two channels count against the same 720-credit budget.
    let components: Vec<_> = super::super::underwriting_policy()
        .components
        .iter()
        .map(|c| json!({"kind":c.kind,"credits_per_second":c.credits_per_second.to_string(),"increment_seconds":c.increment_seconds,"minimum_seconds":c.minimum_seconds}))
        .collect();
    let input = json!({"revision":1,"enabled":true,"acceptance_text":"underwritten exception, not reserved liability","acceptance_version":"v2","paid_tier_evidence_reference":"paid account invoice fixture","max_leg_seconds":1801,"ceiling_evidence":{"kind":"written_confirmation","reference":"provider ceiling letter"},"timestamp_fixture_reference":null,"detail_record_options_reference":"options fixture","detail_record_fixture_reference":"validated call-control fixture","detail_record_fields":["call_leg_id","connection_id","answered_at","finished_at"],"supplier_tariff_revision":1,"fx_policy_revision":"fx fixture","character_tariff_reference":"tts character tariff fixture","components":components,"rolling_budget":"720"});
    underwriting::configure(
        &f.state,
        &f.admin,
        &account.id,
        serde_json::from_value(input).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        account_row(db, &account.id)
            .await
            .underwriting
            .unwrap()
            .revision,
        2
    );
}

#[tokio::test]
async fn rotation_skips_supplier_released_numbers_and_proves_the_account_when_none_remain() {
    use crate::models::downstream_service::ProxyOperationRule;
    let f = super::super::fixture().await;
    let db = &f.state.db;
    let (server, mut service) = super::super::twilio_proof_fixture(&f).await;
    let account_path = format!("/2010-04-01/Accounts/{}.json", f.account.account_id);
    service
        .proxy_operation_policy
        .as_mut()
        .unwrap()
        .rules
        .push(ProxyOperationRule {
            method: "GET".into(),
            path_template: account_path.clone(),
            ..Default::default()
        });
    db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&service.id}, &service)
        .await
        .unwrap();
    super::super::insert_rental(&f).await;
    // A number the supplier already released has no provider resources left.
    let mut released = f.line.clone();
    released.id = id();
    released.number_resource_id = id();
    released.phone_number = "+18005550133".into();
    released.voice.as_mut().unwrap().application_id = id();
    released.state = LineState::Retired;
    released.current_claim_id = None;
    released.supplier_released_at = Some(Utc::now() - Duration::days(1));
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .insert_one(&released)
        .await
        .unwrap();
    let uri = server.uri();
    let rotate = |secret: &str| {
        custody::rotate_with_origin(
            &f.state,
            &f.admin,
            &f.account.id,
            custody::RotationInput {
                expected_revision: 1,
                credential: zeroize::Zeroizing::new(format!("{}:{secret}", f.account.account_id)),
            },
            Some(&uri),
        )
    };
    rotate("successor-one").await.unwrap();
    // Every number is now released: rotation proves the account itself, and
    // names the reason when that read is unavailable.
    db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.line.id},
            doc! {"$set":{"state":"retired","current_claim_id":bson::Bson::Null,"supplier_released_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let refused = rotate("successor-two").await.unwrap_err().to_string();
    assert!(refused.contains("provider account identity"), "{refused}");
    Mock::given(method("GET"))
        .and(path(account_path))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"sid": f.account.account_id, "status": "active"})),
        )
        .mount(&server)
        .await;
    rotate("successor-two").await.unwrap();
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
        format!("{}:successor-two", f.account.account_id)
    );
}

#[tokio::test]
async fn cost_policy_takes_rfc3339_anchors_and_refuses_extended_json() {
    let f = super::super::fixture().await;
    let db = &f.state.db;
    let base = json!({"revision":0,"fx":[],"schemas":[],"inventory":{"cycle":"calendar_month",
        "cycle_anchor":"2026-10-01T00:00:00.123456Z","amount":"1.2","currency":"USD",
        "source":"https://supplier.example/rental","fx_policy_revision":"fx-1"}});
    for bad in [
        json!({"$date": "2026-10-01T00:00:00Z"}),
        json!("2026-13-01T00:00:00Z"),
        json!("yesterday"),
        json!(1_790_000_000_000_i64),
    ] {
        let mut wire = base.clone();
        wire["inventory"]["cycle_anchor"] = bad.clone();
        assert!(
            serde_json::from_value::<costs::PolicyInput>(wire).is_err(),
            "{bad}"
        );
    }
    costs::configure(
        &f.state,
        &f.admin,
        &f.line.id,
        serde_json::from_value(base.clone()).unwrap(),
    )
    .await
    .unwrap();
    let stored = db
        .collection::<ManagedCostPolicy>(ManagedCostPolicy::COLLECTION_NAME)
        .find_one(doc! {"line_id":&f.line.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.inventory.unwrap().cycle_anchor,
        "2026-10-01T00:00:00.123Z"
            .parse::<chrono::DateTime<Utc>>()
            .unwrap()
    );
    // Re-saving the same sub-millisecond anchor is not an anchor rewrite.
    let mut again = base;
    again["revision"] = json!(1);
    costs::configure(
        &f.state,
        &f.admin,
        &f.line.id,
        serde_json::from_value(again).unwrap(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn listings_batch_coverage_quotes_bindings_and_live_calls_across_claims() {
    use crate::handlers::managed_comms::{Page, admin_lines, lines};
    use axum::extract::{Query, State};
    let f = super::super::fixture().await;
    let db = &f.state.db;
    let now = Utc::now();
    let stored = |t: chrono::DateTime<Utc>| bson::DateTime::from_chrono(t).to_chrono().to_rfc3339();
    // A second claimed line with a past, a current and a pre-funded period.
    let binding = id();
    let mut claim2 = f.claim.clone();
    claim2.id = id();
    let mut line2 = f.line.clone();
    line2.id = id();
    line2.number_resource_id = id();
    line2.phone_number = "+18005550134".into();
    line2.voice.as_mut().unwrap().application_id = id();
    line2.current_claim_id = Some(claim2.id.clone());
    claim2.line_id = line2.id.clone();
    claim2.voice_binding_id = Some(binding.clone());
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .insert_one(&line2)
        .await
        .unwrap();
    db.collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .insert_one(&claim2)
        .await
        .unwrap();
    let windows = [
        (now - Duration::days(40), now - Duration::days(10)),
        (now - Duration::days(10), now + Duration::days(20)),
        (now + Duration::days(20), now + Duration::days(50)),
    ];
    for (index, (from, until)) in windows.iter().enumerate() {
        let mut p = f.period.clone();
        p.id = id();
        p.claim_id = claim2.id.clone();
        p.period_index = index as i64;
        p.covered_from = *from;
        p.covered_until = *until;
        p.billing_request_id = format!("managed-line-rental:{}:{index}", claim2.id);
        db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
            .insert_one(&p)
            .await
            .unwrap();
    }
    db.collection::<bson::Document>(BINDINGS)
        .insert_one(doc! {"_id":&binding,"user_id":&f.owner,"revision":7_i64})
        .await
        .unwrap();
    // Two open quotes for the first claim (newest wins), an accepted one for the second.
    let quote = |claim: &ManagedLineClaim, age: i64, accepted: bool| ManagedLineQuote {
        id: id(),
        user_id: f.owner.clone(),
        line_id: claim.line_id.clone(),
        kind: QuoteKind::Renewal {
            claim_id: claim.id.clone(),
            generation: claim.managed_claim_generation,
            period_index: 1,
        },
        line_revision: 1,
        eligibility_revision: 1,
        automatic_request_id: None,
        rental_rate: rate(&f.line.rental_billing_service_id),
        terms: claim.accepted_terms.clone(),
        usage_rates: vec![],
        standard_voice_options: vec![],
        accept_before: now + Duration::minutes(10),
        accepted_at: accepted.then_some(now),
        cancelled_at: None,
        created_at: now - Duration::minutes(age),
        expires_at: Some(now + Duration::days(1)),
    };
    let older = quote(&f.claim, 5, false);
    let newer = quote(&f.claim, 1, false);
    for q in [&older, &newer, &quote(&claim2, 1, true)] {
        db.collection::<ManagedLineQuote>(ManagedLineQuote::COLLECTION_NAME)
            .insert_one(q)
            .await
            .unwrap();
    }
    for (claim, held) in [(&claim2.id, true), (&claim2.id, true), (&f.claim.id, false)] {
        db.collection::<bson::Document>(ManagedCallAdmission::COLLECTION_NAME)
            .insert_one(
                doc! {"_id":id(),"session_id":id(),"managed_claim_id":claim,"slot_held":held},
            )
            .await
            .unwrap();
    }

    let merchant = lines(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Query(Page::default()),
    )
    .await
    .unwrap()
    .0;
    let find = |v: &serde_json::Value, line: &str| {
        v["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == line)
            .cloned()
            .unwrap()
    };
    let first = find(&merchant, &f.line.id)["claim"].clone();
    assert_eq!(first["period_index"], 0);
    assert_eq!(first["covered_from"], stored(f.period.covered_from));
    assert_eq!(first["covered_until"], stored(f.period.covered_until));
    assert_eq!(first["renewal_quote"]["id"], newer.id);
    assert!(first["voice_binding_revision"].is_null());
    let second = find(&merchant, &line2.id)["claim"].clone();
    assert_eq!(second["period_index"], 2);
    assert_eq!(second["covered_from"], stored(windows[1].0));
    assert_eq!(second["covered_until"], stored(windows[2].1));
    assert_eq!(second["voice_binding_revision"], 7);
    assert!(second["renewal_quote"].is_null());

    // The operator listing authorizes against both platform roles: a merchant
    // is refused as forbidden, never as an internal error.
    assert!(matches!(
        admin_lines(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Query(Page::default()),
        )
        .await,
        Err(crate::errors::AppError::Forbidden(_))
    ));
    let admin = admin_lines(
        State(f.state.clone()),
        test_auth_user(&f.admin.user_id),
        Query(Page::default()),
    )
    .await
    .unwrap()
    .0;
    let first = find(&admin, &f.line.id);
    assert_eq!(first["live_calls"], 0);
    assert_eq!(
        first["claim"]["covered_until"],
        stored(f.period.covered_until)
    );
    let second = find(&admin, &line2.id);
    assert_eq!(second["live_calls"], 2);
    assert_eq!(second["claim"]["covered_until"], stored(windows[2].1));
    assert_eq!(second["claim"]["owner_id"], f.owner);
}

#[tokio::test]
async fn unreadable_rows_never_fail_listings_and_admin_responses_name_them() {
    use crate::handlers::managed_comms::{Page, admin_lines, lines, managed_accounts};
    use crate::services::call_channels::managed::listing;
    use axum::extract::{Query, State};
    let f = super::super::fixture().await;
    let db = &f.state.db;
    // Fixed IDs order rows around the cursor: readable, malformed, readable.
    let ids = [
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-4000-8000-000000000002",
        "00000000-0000-4000-8000-000000000003",
        "00000000-0000-4000-8000-000000000004",
    ];
    let available = |id: &str, n: u32| {
        let mut l = f.line.clone();
        l.id = id.into();
        l.number_resource_id = format!("PN{n}");
        l.phone_number = format!("+1800555{:04}", 2000 + n);
        if let Some(voice) = l.voice.as_mut() {
            voice.application_id = format!("AP{n}");
        }
        l.state = LineState::Available;
        l.current_claim_id = None;
        l
    };
    let lines_collection = db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME);
    lines_collection
        .insert_many([available(ids[0], 1), available(ids[2], 3)])
        .await
        .unwrap();
    db.collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .insert_one(doc! {"_id":ids[1],"state":"available","provider":"twilio","phone_number":"+18005552002","number_resource_id":7_i64})
        .await
        .unwrap();
    // A readable claimed line whose current claim document is malformed.
    let broken_claim = id();
    let mut claimed = available(ids[3], 4);
    claimed.state = LineState::Claimed;
    claimed.current_claim_id = Some(broken_claim.clone());
    lines_collection.insert_one(&claimed).await.unwrap();
    db.collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .insert_one(doc! {"_id":&broken_claim,"user_id":&f.owner,"line_id":ids[3],"state":"claimed","managed_claim_generation":"one"})
        .await
        .unwrap();
    let broken_account = id();
    db.collection::<bson::Document>(crate::models::channel_adapter::ACCOUNTS)
        .insert_one(doc! {"_id":&broken_account,"provider":"twilio","account_id":5_i64})
        .await
        .unwrap();
    let listed = |v: &serde_json::Value, key: &str| -> Vec<String> {
        v[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap().to_owned())
            .collect()
    };

    // Merchants only see the unreadable rows omitted.
    let merchant = lines(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Query(Page::default()),
    )
    .await
    .unwrap()
    .0;
    let visible = listed(&merchant, "lines");
    for expected in [ids[0], ids[2], f.line.id.as_str()] {
        assert!(visible.iter().any(|id| id == expected), "{expected}");
    }
    assert!(!visible.iter().any(|id| id == ids[1] || id == ids[3]));
    assert!(merchant.get("unreadable_ids").is_none());
    // A cursor at the unreadable row still pages to the rows after it.
    let after = lines(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Query(Page {
            after: Some(ids[1].into()),
        }),
    )
    .await
    .unwrap()
    .0;
    let after = listed(&after, "lines");
    assert!(after.iter().any(|id| id == ids[2]));
    assert!(!after.iter().any(|id| id == ids[0] || id == ids[1]));

    // Operators see every readable row and the IDs that need repair.
    let admin = admin_lines(
        State(f.state.clone()),
        test_auth_user(&f.admin.user_id),
        Query(Page::default()),
    )
    .await
    .unwrap()
    .0;
    let rows = listed(&admin, "lines");
    for expected in [ids[0], ids[2], ids[3], f.line.id.as_str()] {
        assert!(rows.iter().any(|id| id == expected), "{expected}");
    }
    let unreadable: Vec<&str> = admin["unreadable_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(unreadable.contains(&ids[1]));
    assert!(unreadable.contains(&broken_claim.as_str()));
    let line4 = admin["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == ids[3])
        .unwrap();
    assert!(line4["claim"].is_null());
    let accounts = managed_accounts(
        State(f.state.clone()),
        test_auth_user(&f.admin.user_id),
        Query(Page::default()),
    )
    .await
    .unwrap()
    .0;
    assert!(listed(&accounts, "accounts").contains(&f.account.id));
    assert!(
        accounts["unreadable_ids"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(broken_account))
    );

    // The cursor is taken from the raw page, so it advances past an
    // unreadable row that ends a page.
    let only = doc! {"$in":&ids[..3]};
    let first = listing::page::<ManagedPhoneLine>(
        db,
        ManagedPhoneLine::COLLECTION_NAME,
        doc! {"_id":only.clone()},
        2,
    )
    .await
    .unwrap();
    assert_eq!(
        first.rows.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
        [ids[0]]
    );
    assert_eq!(first.unreadable, [ids[1]]);
    assert_eq!(first.next_after_id.as_deref(), Some(ids[1]));
    let second = listing::page::<ManagedPhoneLine>(
        db,
        ManagedPhoneLine::COLLECTION_NAME,
        doc! {"_id":{"$in":&ids[..3],"$gt":ids[1]}},
        2,
    )
    .await
    .unwrap();
    assert_eq!(
        second
            .rows
            .iter()
            .map(|l| l.id.as_str())
            .collect::<Vec<_>>(),
        [ids[2]]
    );
    assert!(second.unreadable.is_empty() && second.next_after_id.is_none());
}
