//! M5/M6 use the same real replica-set fixture as accepted effect admission.
use super::*;
use crate::models::assistant_conversation::AgentRole;
use crate::services::assistant_acknowledgement_service::ChatAuthority;
use crate::services::call_channels::managed::{confirmations, costs, synthesis};
use chrono::Datelike;

async fn allow_fixture_operations(t: &LifecycleFixture, operations: &[(&str, String)]) {
    use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};
    let rows =
        t.f.state
            .db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME);
    let mut service = rows
        .find_one(doc! {"_id":&t.f.service.id})
        .await
        .unwrap()
        .unwrap();
    let policy = service
        .proxy_operation_policy
        .get_or_insert_with(|| ProxyOperationPolicy { rules: vec![] });
    policy
        .rules
        .extend(operations.iter().map(|(method, path)| ProxyOperationRule {
            method: (*method).into(),
            path_template: path.clone(),
            ..Default::default()
        }));
    rows.update_one(
        doc! {"_id":&service.id},
        doc! {"$set":{"proxy_operation_policy":bson::to_bson(policy).unwrap()}},
    )
    .await
    .unwrap();
}

async fn premium(t: &LifecycleFixture, b: &mut Binding, limit: i64) {
    let service = price_service(t, ManagedCommsRole::Premium, BillingMetric::VoiceSeconds).await;
    let option = StandardVoiceOption {
        voice: "ElevenLabs.eleven_flash_v2_5.fixture".into(),
        language: "en-US".into(),
    };
    allow_fixture_operations(
        t,
        &[
            ("GET", "/v2/user_profile".into()),
            ("GET", "/v2/integration_secrets".into()),
        ],
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/v2/user_profile"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"user_id":t.f.account.account_id}})),
        )
        .mount(&t.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/integration_secrets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"id":"metadata-id","identifier":"fixture-integration"}]}),
        ))
        .mount(&t.server)
        .await;
    synthesis::configure_with_origin(
        &t.f.state,
        &t.f.admin,
        &t.f.account.id,
        synthesis::VoiceAccountInput {
            revision: 0,
            per_call_characters: 1000,
            greeting: None,
            wrapup: None,
            premium: Some(synthesis::PremiumInput {
                api_key_ref: "fixture-integration".into(),
                voices: vec![option.clone()],
                billing_account_identity: "fixture-elevenlabs-account".into(),
                monthly_characters: limit,
                plan_tier: "Scale".into(),
                oem_terms_version: "eua-v1".into(),
                admin_acknowledged: true,
                billing_service_id: service,
            }),
        },
        Some(&t.server.uri()),
    )
    .await
    .unwrap();
    synthesis::set_enabled(&t.f.state, &t.f.admin, true)
        .await
        .unwrap();
    let db = &t.f.state.db;
    db.collection::<bson::Document>(ManagedCommsEligibility::COLLECTION_NAME).update_one(doc!{"user_id":&t.f.owner},doc!{"$set":{"premium_allowed":true,"required_terms.elevenlabs_eua":"eua-v1","accepted_terms.elevenlabs_eua":"eua-v1"}}).await.unwrap();
    // Binding has no managed_claim_id field: derive the immutable credential.
    let CredentialBinding::ManagedLine {
        managed_claim_id, ..
    } = &b.credential
    else {
        unreachable!()
    };
    db.collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":managed_claim_id},doc!{"$set":{"voice_tier":"premium","accepted_terms.elevenlabs_eua":"eua-v1","standard_voice":bson::to_bson(&option).unwrap()}}).await.unwrap();
    b.voice = option.voice;
    b.language = option.language;
    db.collection::<Binding>(BINDINGS)
        .replace_one(doc! {"_id":&b.id}, &*b)
        .await
        .unwrap();
    binding::live(&t.f.state, b).await.unwrap();
}
#[tokio::test]
async fn unicode_blocks_uncertainty_and_proven_unwritten_credit_are_atomic() {
    let (t, b, _) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v3:scalar-fixture").await;
    let text = "🦀界e\u{301}".repeat(100);
    assert_eq!(text.chars().count(), 400);
    let (a, c) = tokio::join!(
        synthesis::debit(&t.f.state, &s, &text, false),
        synthesis::debit(&t.f.state, &s, &text, false)
    );
    let a = a.unwrap();
    let _uncertain = c.unwrap();
    assert_eq!(
        synthesis::allowance(&t.f.state.db, &s.id)
            .await
            .unwrap()
            .consumed,
        800
    );
    synthesis::unwritten(&t.f.state.db, &a).await.unwrap();
    synthesis::unwritten(&t.f.state.db, &a).await.unwrap();
    assert_eq!(
        synthesis::allowance(&t.f.state.db, &s.id)
            .await
            .unwrap()
            .consumed,
        400
    );
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(2), "fixture")
        .await
        .unwrap();
    coordinator::finalize_call(&t.f.state, &ended)
        .await
        .unwrap();
    let allowance = synthesis::allowance(&t.f.state.db, &s.id).await.unwrap();
    assert!(allowance.reserve_released);
    assert_eq!(allowance.consumed, 400);
    assert!(
        synthesis::debit(&t.f.state, &s, "late", false)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn proven_unwritten_wrapup_credit_survives_terminal_reserve_release() {
    let (t, mut b, _) = setup_telnyx().await;
    premium(&t, &mut b, 2000).await;
    let s = prepare_telnyx(&t, &b, "v3:wrapup-credit").await;
    let block = synthesis::debit(&t.f.state, &s, synthesis::WRAPUP, true)
        .await
        .unwrap();
    let reserve_month = synthesis::allowance(&t.f.state.db, &s.id)
        .await
        .unwrap()
        .reserve_month_id
        .unwrap();
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(2), "fixture")
        .await
        .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    synthesis::unwritten(&t.f.state.db, &block).await.unwrap();
    synthesis::unwritten(&t.f.state.db, &block).await.unwrap();
    let month =
        t.f.state
            .db
            .collection::<ManagedCharacterMonth>(ManagedCharacterMonth::COLLECTION_NAME)
            .find_one(doc! {"_id":&reserve_month})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(month.consumed, 0);
    assert_eq!(
        synthesis::allowance(&t.f.state.db, &s.id)
            .await
            .unwrap()
            .consumed,
        0
    );
}
#[tokio::test]
async fn standard_wrapup_is_reserved_and_stale_writer_cannot_debit() {
    let (t, b, _) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v3:wrap-fixture").await;
    t.f.state
        .db
        .collection::<bson::Document>(ManagedCharacterAllowance::COLLECTION_NAME)
        .update_one(doc! {"session_id":&s.id}, doc! {"$set":{"limit":700_i64}})
        .await
        .unwrap();
    synthesis::debit(&t.f.state, &s, &"a".repeat(400), false)
        .await
        .unwrap();
    assert!(synthesis::debit(&t.f.state, &s, "x", false).await.is_err());
    synthesis::debit(&t.f.state, &s, synthesis::WRAPUP, true)
        .await
        .unwrap();
    assert!(
        synthesis::debit(&t.f.state, &s, &"x".repeat(300), true)
            .await
            .is_err()
    );
    t.f.state
        .db
        .collection::<bson::Document>(SESSIONS)
        .update_one(doc! {"_id":&s.id}, doc! {"$inc":{"generation":1_i64}})
        .await
        .unwrap();
    assert!(
        synthesis::debit(&t.f.state, &s, "stale", true)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn premium_dual_meter_budget_kill_and_same_seconds_settlement() {
    let (t, mut b, _) = setup_telnyx().await;
    premium(&t, &mut b, 1000).await;
    let s = prepare_telnyx(&t, &b, "v3:premium-fixture").await;
    let receipt = calls::receipt(&t.f.state, &s.id).await.unwrap();
    assert!(receipt.premium.is_some());
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(3600));
    let first_block = "a".repeat(400);
    let second_block = "b".repeat(400);
    let (a, c) = tokio::join!(
        synthesis::debit(&t.f.state, &s, &first_block, false),
        synthesis::debit(&t.f.state, &s, &second_block, false)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(c.is_ok()), 1);
    synthesis::set_enabled(&t.f.state, &t.f.admin, false)
        .await
        .unwrap();
    assert!(
        synthesis::debit(&t.f.state, &s, "kill", false)
            .await
            .is_err()
    );
    synthesis::debit(&t.f.state, &s, synthesis::WRAPUP, true)
        .await
        .unwrap();
    assert!(
        synthesis::fallback_before_dispatch(&t.f.state, &s)
            .await
            .is_err()
    );
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(1900), "fixture")
        .await
        .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    for m in std::iter::once(receipt.base).chain(receipt.premium) {
        let row =
            t.f.state
                .db
                .collection::<UsageMeterRow>(USAGE)
                .find_one(doc! {"_id":&m.usage_row_ids[0]})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(row.quantity, Some(1800));
        assert!(row.released);
    }
    let months:Vec<ManagedCharacterMonth>=t.f.state.db.collection::<ManagedCharacterMonth>(ManagedCharacterMonth::COLLECTION_NAME).find(doc!{"identity_key":{"$ne":session::identity(&["elevenlabs-policy","fixture-elevenlabs-account"])}}).await.unwrap().try_collect().await.unwrap();
    assert_eq!(months.len(), 1);
    assert_eq!(
        months[0].consumed,
        400 + synthesis::WRAPUP.chars().count() as i64
    );
}
#[tokio::test]
async fn proven_pre_dispatch_fallback_releases_only_premium_and_standard_still_writes() {
    let (t, mut b, _) = setup_telnyx().await;
    premium(&t, &mut b, 2000).await;
    let s = prepare_telnyx(&t, &b, "v3:fallback-fixture").await;
    synthesis::fallback_before_dispatch(&t.f.state, &s)
        .await
        .unwrap();
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1800));
    synthesis::debit(&t.f.state, &s, "standard greeting", false)
        .await
        .unwrap();
    assert!(
        synthesis::fallback_before_dispatch(&t.f.state, &s)
            .await
            .is_err()
    );
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(12), "fixture")
        .await
        .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    let r = calls::receipt(&t.f.state, &s.id).await.unwrap();
    let m = r.premium.unwrap();
    let row =
        t.f.state
            .db
            .collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id":&m.usage_row_ids[0]})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(row.quantity, Some(0));
}
#[tokio::test]
async fn shared_elevenlabs_identity_has_one_monthly_budget_across_telnyx_accounts() {
    let (t, mut b, _) = setup_telnyx().await;
    premium(&t, &mut b, 1000).await;
    let s = prepare_telnyx(&t, &b, "v3:shared-budget").await;
    let mut config = synthesis::config(&t.f.state.db, &t.f.account.id)
        .await
        .unwrap()
        .unwrap();
    config.id = id();
    config.trusted_account_id = id();
    t.f.state
        .db
        .collection::<ManagedVoiceAccount>(ManagedVoiceAccount::COLLECTION_NAME)
        .insert_one(&config)
        .await
        .unwrap();
    let mut second = s.clone();
    second.id = id();
    second.identity_key = id();
    second.provider_call_id = "v3:second-account".into();
    t.f.state
        .db
        .collection::<CallSession>(SESSIONS)
        .insert_one(&second)
        .await
        .unwrap();
    let db = &t.f.state.db;
    crate::services::transaction_service::run_db_only(db, &(db, &second, &config), |tx, c| {
        Box::pin(async move {
            let (db, s, c) = *c;
            synthesis::initialize(db, tx, s, Some(c), true).await
        })
    })
    .await
    .unwrap();
    let first_block = "a".repeat(400);
    let second_block = "b".repeat(400);
    let (a, c) = tokio::join!(
        synthesis::debit(&t.f.state, &s, &first_block, false),
        synthesis::debit(&t.f.state, &second, &second_block, false)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(c.is_ok()), 1);
}

fn tariff(kind: CostKind, unit: &str) -> SupplierTariff {
    SupplierTariff {
        kind,
        amount: "0.12".into(),
        currency: "USD".into(),
        unit: unit.into(),
        increment: 60,
        minimum: 60,
        source: "https://supplier.example/tariff".into(),
        revision: 1,
        fx_policy_revision: "fx-1".into(),
        effective_at: Utc::now() - chrono::Duration::days(1),
    }
}
fn fx() -> Vec<CostFxRate> {
    vec![CostFxRate {
        currency: "USD".into(),
        credits_per_unit: "100".into(),
        revision: "fx-1".into(),
    }]
}
async fn cost_row(t: &LifecycleFixture, effect: &str) -> ManagedCommsCostEvidence {
    t.f.state
        .db
        .collection::<ManagedCommsCostEvidence>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .find_one(doc! {"effect_id":effect})
        .await
        .unwrap()
        .unwrap()
}
#[test]
fn supplier_sign_rounding_currency_and_quality_are_independent() {
    let (v, q) = costs::estimate(&tariff(CostKind::Relay, "minute"), 61).unwrap();
    assert_eq!(q, 120);
    assert_eq!(v, "0.24");
    assert_eq!(costs::amount("-0.012340").unwrap().to_string(), "0.01234");
    assert!(costs::amount("NaN").is_err());
    assert_eq!(
        costs::convert("0.24", "USD", "fx-1", &fx()).unwrap(),
        Credits::from_whole(24)
    );
    assert!(costs::convert("1", "SGD", "fx-1", &fx()).is_err());
    assert!(costs::convert("1", "USD", "new-policy", &fx()).is_err());
    let a = Utc::now();
    let anchor = a.with_day(31).unwrap_or(a);
    let (from, until) = costs::cycle(anchor, 1).unwrap();
    assert!(until > from);
}
#[tokio::test]
async fn twilio_connectivity_is_actual_but_tariffs_do_not_establish_actual_margin() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let mut row = cost_row(&t, &s.id).await;
    row.policy = Some(ManagedCostPolicy {
        id: id(),
        line_id: t.f.line.id.clone(),
        revision: 1,
        fx: fx(),
        schemas: vec![],
        inventory: None,
    });
    let c=costs::twilio_record(&row,&json!({"sid":s.provider_call_id,"account_sid":b.account_id,"price":"-0.01","price_unit":"usd","status":"completed"})).unwrap();
    assert_eq!(c.kind, CostKind::CarrierInbound);
    assert_eq!(c.amount.as_deref(), Some("0.01"));
    assert!(
        costs::twilio_record(
            &row,
            &json!({"sid":"wrong","account_sid":b.account_id,"price":"-1","price_unit":"usd"})
        )
        .is_err()
    );
    row.components = row
        .required_components
        .iter()
        .map(|kind| CostComponent {
            kind: *kind,
            source: CostSource::TariffEstimate,
            provider_record_id: None,
            amount: Some("0.01".into()),
            currency: "USD".into(),
            fx_policy_revision: "fx-1".into(),
            supplier_rounded_quantity: Some(60),
        })
        .collect();
    assert_eq!(costs::quality(&row), (true, false));
    row.components[0] = c;
    assert_eq!(costs::quality(&row), (true, false));
    for c in &mut row.components {
        c.source = CostSource::ProviderRecord;
        c.provider_record_id = Some("verified-component".into());
    }
    assert_eq!(costs::quality(&row), (true, true));
    row.components.pop();
    assert_eq!(costs::quality(&row), (false, false));
}
#[tokio::test]
async fn telnyx_verified_record_type_identity_and_carrier_fee_are_bound() {
    let (t, b) = setup_sms().await;
    let effect = pending_sms(&t, &b, 1).await;
    // pending_sms predates M6 registration, so register it with the same parent identity.
    let r =
        t.f.state
            .db
            .collection::<ManagedInboundSms>(ManagedInboundSms::COLLECTION_NAME)
            .find_one(doc! {"_id":&effect})
            .await
            .unwrap()
            .unwrap();
    let db = &t.f.state.db;
    crate::services::transaction_service::run_db_only(db, &(db, &t.f.line, &r), |tx, c| {
        Box::pin(async move {
            let (db, l, r) = *c;
            costs::register(db, tx, l, &r.id, EffectKind::Sms, &r.admitted, r.created_at).await
        })
    })
    .await
    .unwrap();
    let mut row = cost_row(&t, &effect).await;
    row.admitted.as_mut().unwrap().provider = "telnyx".into();
    let r = row.admitted.as_ref().unwrap();
    let schema = CostRecordSchema {
        kind: CostKind::SmsPart,
        record_type: "messaging".into(),
        amount_field: "cost".into(),
        currency_field: "currency".into(),
        options_reference: "verified-options".into(),
        fixture_reference: "verified-fixture".into(),
    };
    let mut v = json!({"data":[{"id":"record-one","record_type":"messaging","message_id":r.provider_message_id,"messaging_profile_id":r.application_id,"cost":{"amount":"-0.02","currency":"USD"},"carrier_fee":{"amount":"0.003","currency":"USD"}}]});
    assert_eq!(
        costs::telnyx_record(&row, &schema, &v)
            .unwrap()
            .amount
            .as_deref(),
        Some("0.023")
    );
    v["data"][0]["messaging_profile_id"] = json!("foreign");
    assert!(costs::telnyx_record(&row, &schema, &v).is_err());
}
#[tokio::test]
async fn capture_and_inventory_continue_flag_off_and_retained_reference_survives_erasure() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let ended = session::terminal(&t.f.state, &s, Utc::now(), Some(61), "fixture")
        .await
        .unwrap();
    calls::finalize(&t.f.state, &ended).await.unwrap();
    let anchor = Utc::now() - chrono::Duration::days(62);
    costs::configure(
        &t.f.state,
        &t.f.admin,
        &t.f.line.id,
        costs::PolicyInput {
            revision: 0,
            fx: fx(),
            schemas: vec![],
            inventory: Some(costs::InventoryTariffInput {
                cycle: "calendar_month".into(),
                cycle_anchor: anchor,
                amount: "1.2".into(),
                currency: "USD".into(),
                source: "https://supplier.example/rental".into(),
                fx_policy_revision: "fx-1".into(),
            }),
        },
    )
    .await
    .unwrap();
    let mut row = cost_row(&t, &s.id).await;
    row.policy = Some(ManagedCostPolicy {
        id: id(),
        line_id: t.f.line.id.clone(),
        revision: 1,
        fx: fx(),
        schemas: vec![],
        inventory: None,
    });
    row.tariffs = vec![
        tariff(CostKind::Relay, "minute"),
        tariff(CostKind::Stt, "minute"),
        tariff(CostKind::Tts, "second"),
    ];
    // Keep the erasure owner marker while editing the controlled fixture.
    let mut d = costs::evidence_document(&row).unwrap();
    d.insert("tenant_owner_id", &t.f.owner);
    t.f.state
        .db
        .collection::<bson::Document>(ManagedCommsCostEvidence::COLLECTION_NAME)
        .replace_one(doc! {"_id":&row.id}, d)
        .await
        .unwrap();
    Mock::given(method("GET")).and(path(format!("/2010-04-01/Accounts/{}/Calls/{}.json",b.account_id,s.provider_call_id))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"sid":s.provider_call_id,"account_sid":b.account_id,"price":"-0.01","price_unit":"USD","status":"completed"}))).mount(&t.server).await;
    flags::set_platform_override(
        &t.f.state.db,
        flags::MANAGED_LINES_FLAG_KEY,
        &flags::FlagTarget::User(t.f.owner.clone()),
        false,
        &t.f.admin.user_id,
    )
    .await
    .unwrap();
    assert!(
        crate::services::call_channels::managed::require_enabled(&t.f.state.db, &t.f.owner)
            .await
            .is_err()
    );
    costs::capture(&t.f.state, &row.id).await.unwrap();
    let c = cost_row(&t, &s.id).await;
    assert_eq!(costs::quality(&c), (true, false));
    assert_eq!(c.billed_credits, Some(Credits::from_whole(61)));
    assert!(
        !costs::retain_for_erasure(&t.f.state, &t.f.owner)
            .await
            .unwrap()
    );
    assert!(
        costs::retain_for_erasure(&t.f.state, &t.f.owner)
            .await
            .unwrap()
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
    costs::capture(&t.f.state, &row.id).await.unwrap();
    assert_eq!(
        cost_row(&t, &s.id).await.billed_credits,
        Some(Credits::from_whole(61))
    );
    costs::sweep(&t.f.state).await.unwrap();
    let count =
        t.f.state
            .db
            .collection::<ManagedInventoryCost>(ManagedInventoryCost::COLLECTION_NAME)
            .count_documents(doc! {"line_id":&t.f.line.id})
            .await
            .unwrap();
    assert!(count >= 2);
    t.f.state
        .db
        .collection::<bson::Document>(ManagedPhoneLine::COLLECTION_NAME)
        .update_one(doc! {"_id":&t.f.line.id}, doc! {"$set":{"state":"retired"}})
        .await
        .unwrap();
    costs::sweep(&t.f.state).await.unwrap();
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedInventoryCost>(ManagedInventoryCost::COLLECTION_NAME)
            .count_documents(doc! {"line_id":&t.f.line.id})
            .await
            .unwrap(),
        count
    );
    let from = Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let report = costs::report(
        &t.f.state.db,
        costs::ReportQuery {
            from,
            until: from + chrono::Duration::days(1),
            after: None,
            line_id: Some(t.f.line.id.clone()),
        },
    )
    .await
    .unwrap();
    let day = &report.lines[0].days[0];
    assert!(day.actual_margin_credits.is_none());
    assert!(day.estimated_margin_credits.is_some());
    assert_eq!(day.effect_count, 1);
    let detail = costs::effect_report(
        &t.f.state.db,
        costs::EffectQuery {
            line_id: t.f.line.id.clone(),
            from,
            until: from + chrono::Duration::days(1),
            after: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(detail.effects.len(), 1);
    // Another line may share this rental price. Its retained period must not
    // inflate this line's revenue, including after tenant fields are erased.
    let mut claim =
        t.f.state
            .db
            .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
            .find_one(doc! {"line_id":&t.f.line.id})
            .await
            .unwrap()
            .unwrap();
    let mut period =
        t.f.state
            .db
            .collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
            .find_one(doc! {"claim_id":&claim.id})
            .await
            .unwrap()
            .unwrap();
    claim.id = id();
    claim.line_id = id();
    claim.user_id.clear();
    period.id = id();
    period.claim_id = claim.id.clone();
    period.billing_request_id = format!("managed-line-rental:{}:0", claim.id);
    t.f.state
        .db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .insert_one(claim)
        .await
        .unwrap();
    t.f.state
        .db
        .collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME)
        .insert_one(period)
        .await
        .unwrap();
    let again = costs::report(
        &t.f.state.db,
        costs::ReportQuery {
            from,
            until: from + chrono::Duration::days(1),
            after: None,
            line_id: Some(t.f.line.id.clone()),
        },
    )
    .await
    .unwrap();
    assert_eq!(again.lines[0].days[0].billed_credits, day.billed_credits);
}

async fn call_turn(t: &LifecycleFixture, voice: &Binding) -> (CallSession, ChatAuthority) {
    let s = if voice.profile == Profile::TelnyxRelay {
        prepare_telnyx(t, voice, &format!("v3:call-turn-{}", id())).await
    } else {
        prepare(t, voice).await
    };
    let turn = id();
    let key = id();
    let origin = TurnAuthority {
        binding_id: voice.id.clone(),
        session_id: s.id.clone(),
        binding_revision: voice.revision,
        generation: s.generation,
        credential_identity: s.credential_identity.clone(),
    };
    let now = bson::DateTime::now();
    let db = &t.f.state.db;
    db.collection::<bson::Document>(SESSIONS).update_one(doc!{"_id":&s.id},doc!{"$set":{"state":"active","claim_id":"fixture-writer","active_turn_id":&turn,"lease_expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(10))}}).await.unwrap();
    db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).insert_one(doc!{"_id":&s.conversation_id,"user_id":&s.user_id,"title":"fixture","model":"fixture","credential_api_key_id":&key,"message_count":0_i64,"created_at":now,"updated_at":now,"active_turn":{"turn_id":&turn,"started_at":now,"channel_adapter_origin":bson::to_bson(&origin).unwrap()}}).await.unwrap();
    let chat = ChatAuthority {
        voice_parent_conversation_id: None,
        platform_model_context: None,
        automation_work: None,
        execution_group_id: None,
        execution_unsupported: false,
        execution_voice_task: false,
        execution_task: None,
        execution_permission: None,
        business_system_success: false,
        collaboration_thread: false,
        channel_adapter_origin: Some(origin),
        invocation_controlled: false,
        invocation_id: None,
        platform_thread: false,
        platform_origin: None,
        org_agent_access: None,
        turn_id: Some(turn),
        turn_stopped: false,
        turn_live: true,
        machine_node_ids: vec![],
        saved_login_ids: vec![],
        conversation_id: s.conversation_id.clone(),
        user_id: s.user_id.clone(),
        api_key_id: key,
        role: AgentRole::Subagent,
        agent_id: voice.agent_id.clone(),
        agent_name: "fixture".into(),
        guest: true,
        confirmation_policy: None,
        report_to: None,
        with_nyxbot: false,
    };
    (session::load(&t.f.state, &s.id).await.unwrap(), chat)
}
async fn confirmation_fixture() -> (
    LifecycleFixture,
    Binding,
    Binding,
    CallSession,
    ChatAuthority,
) {
    let (t, sms) = setup_sms().await;
    allow_fixture_operations(
        &t,
        &[(
            "POST",
            format!("/2010-04-01/Accounts/{}/Messages.json", sms.account_id),
        )],
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/subscriptions/subscription/entitlements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entitlements":[{"code":"*"}]
        })))
        .mount(&t.server)
        .await;
    let CredentialBinding::ManagedLine {
        managed_claim_id, ..
    } = &sms.credential
    else {
        unreachable!()
    };
    let c = saved_claim(&t, managed_claim_id).await;
    let v = binding::load(&t.f.state, c.voice_binding_id.as_deref().unwrap())
        .await
        .unwrap();
    t.f.state
        .db
        .collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(
            doc! {"_id":managed_claim_id},
            doc! {"$set":{"caller_confirmations":true},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    let (s, chat) = call_turn(&t, &v).await;
    (t, v, sms, s, chat)
}
fn confirmation(text: &str, ordinal: i64) -> confirmations::SendInput {
    confirmations::SendInput {
        text: text.into(),
        ordinal,
        caller_consented: true,
    }
}
#[tokio::test]
async fn caller_confirmation_has_no_destination_replays_ordinals_and_limits_two_slots() {
    let (t, v, sms, s, chat) = confirmation_fixture().await;
    assert!(confirmations::eligible(&t.f.state.db, &chat).await);
    let service = confirmations::service_for(&t.f.state.db, &chat)
        .await
        .unwrap();
    let definitions = crate::services::mcp_service::generate_tool_definitions(
        std::slice::from_ref(&service),
        None,
    );
    assert!(definitions.iter().any(|d| d.name == confirmations::TOOL));
    assert!(
        !definitions
            .iter()
            .any(|d| d.name == "nyxid__text_current_caller")
    );
    let search = crate::services::mcp_service::search_all_tools_ranked(
        &[service],
        "text current caller",
        &std::collections::HashMap::new(),
    );
    assert!(search.matches.iter().any(|d| d.name == confirmations::TOOL));
    let account =
        crate::services::assistant_account_tools::virtual_service_for(&t.f.state.db, &chat).await;
    assert!(
        !account
            .endpoints
            .iter()
            .any(|e| e.name == "text_current_caller")
    );

    assert!(
        serde_json::from_value::<confirmations::SendInput>(
            json!({"text":"hello","caller_consented":true,"ordinal":0,"to":"+18005550000"})
        )
        .is_err()
    );
    Mock::given(method("POST"))
        .and(path(format!(
            "/2010-04-01/Accounts/{}/Messages.json",
            sms.account_id
        )))
        .respond_with(|_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(
                json!({"sid":format!("SM{}", Uuid::new_v4().simple()),"num_segments":"1"}),
            )
        })
        .expect(2)
        .mount(&t.server)
        .await;
    let first = confirmations::send(&t.f.state, &chat, "invocation-a", confirmation("Booked", 0))
        .await
        .unwrap();
    let replay = confirmations::send(&t.f.state, &chat, "invocation-a", confirmation("Booked", 0))
        .await
        .unwrap();
    assert_eq!(first["operation_id"], replay["operation_id"]);
    assert_eq!(replay["replayed"], true);
    assert!(
        confirmations::send(
            &t.f.state,
            &chat,
            "invocation-a",
            confirmation("Changed payload", 0)
        )
        .await
        .is_err()
    );
    confirmations::send(
        &t.f.state,
        &chat,
        "invocation-a",
        confirmation("Directions", 1),
    )
    .await
    .unwrap();
    assert!(
        confirmations::send(&t.f.state, &chat, "invocation-b", confirmation("Third", 0))
            .await
            .is_err()
    );
    let op =
        t.f.state
            .db
            .collection::<ManagedCallConfirmation>(ManagedCallConfirmation::COLLECTION_NAME)
            .find_one(doc! {"session_id":&s.id,"ordinal":0_i64})
            .await
            .unwrap()
            .unwrap();
    assert!(op.model_asserted_caller_consent);
    assert_eq!(op.voice_binding_id, v.id);
    assert_eq!(op.sms_binding_id, sms.id);
    assert!(op.billing_request_ids[0].starts_with(&format!("call-confirmation:{}:", op.id)));
    let receipt =
        t.f.state
            .db
            .collection::<EventReceipt>(EVENTS)
            .find_one(doc! {"_id":&op.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(receipt.to.as_deref(), Some(s.from.as_str()));
    assert_eq!(receipt.from.as_deref(), Some(sms.phone_number.as_str()));
    assert!(receipt.billing_finalized);
    let audits: Vec<bson::Document> =
        t.f.state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type":"call_confirmation_admitted"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(audits.len(), 2);
    for audit in audits {
        let text = format!("{audit:?}");
        assert!(!text.contains("Booked"));
        assert!(!text.contains(&s.from));
    }
}
#[tokio::test]
async fn uncertain_confirmation_is_never_retried_and_binding_or_turn_changes_refuse() {
    let (t, _v, sms, _s, chat) = confirmation_fixture().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/2010-04-01/Accounts/{}/Messages.json",
            sms.account_id
        )))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&t.server)
        .await;
    let first = confirmations::send(
        &t.f.state,
        &chat,
        "uncertain",
        confirmation("Please confirm", 0),
    )
    .await
    .unwrap();
    assert_eq!(first["status"], "uncertain");
    let replay = confirmations::send(
        &t.f.state,
        &chat,
        "uncertain",
        confirmation("Please confirm", 0),
    )
    .await
    .unwrap();
    assert_eq!(replay["replayed"], true);
    let operation = first["operation_id"].as_str().unwrap();
    crate::services::call_channels::sms_delivery::record_correlated(
        &t.f.state,
        &sms,
        crate::services::call_channels::sms_delivery::Receipt {
            event: "confirmation-delivery",
            message: "SMuncertain-receipt",
            from: &sms.phone_number,
            to: "+18005550999",
            status: "delivered",
            parts: Some(1),
            send: Some(operation),
        },
    )
    .await
    .unwrap();
    let receipt =
        t.f.state
            .db
            .collection::<EventReceipt>(EVENTS)
            .find_one(doc! {"_id":operation})
            .await
            .unwrap()
            .unwrap();
    assert!(receipt.billing_finalized);
    assert_eq!(receipt.delivery, "delivered");
    t.f.state
        .db
        .collection::<bson::Document>(BINDINGS)
        .update_one(doc! {"_id":&sms.id}, doc! {"$inc":{"revision":1_i64}})
        .await
        .unwrap();
    // Managed binding identity must still match its trusted account and claim;
    // a changed revision is accepted only after a fresh receipt, never retargeted.
    t.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&chat.conversation_id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        confirmations::send(&t.f.state, &chat, "after-stop", confirmation("late", 1))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn concurrent_confirmation_invocations_claim_only_two_and_compensate_failed_funding() {
    let (t, _, sms, s, chat) = confirmation_fixture().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/2010-04-01/Accounts/{}/Messages.json",
            sms.account_id
        )))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&t.server)
        .await;
    let (a, b, c) = tokio::join!(
        confirmations::send(&t.f.state, &chat, "a", confirmation("a", 0)),
        confirmations::send(&t.f.state, &chat, "b", confirmation("b", 0)),
        confirmations::send(&t.f.state, &chat, "c", confirmation("c", 0))
    );
    assert_eq!(
        usize::from(a.is_ok()) + usize::from(b.is_ok()) + usize::from(c.is_ok()),
        2
    );
    assert_eq!(
        t.f.state
            .db
            .collection::<ManagedCallConfirmation>(ManagedCallConfirmation::COLLECTION_NAME)
            .count_documents(doc! {"session_id":&s.id})
            .await
            .unwrap(),
        2
    );
    assert_eq!(wallet(&t).await.reserved_credits, Credits::from_whole(1802));
}
#[test]
fn confirmation_annotations_and_secret_debug_are_metadata_only() {
    let endpoint = confirmations::endpoint();
    assert!(
        endpoint.request_body_schema.unwrap()["properties"]
            .get("to")
            .is_none()
    );
    let binding = CallerConfirmationBinding {
        id: id(),
        voice_binding_id: id(),
        voice_binding_revision: 1,
        sms_binding_id: id(),
        sms_binding_revision: 1,
        voice_credential_identity: "secret-voice-identity".into(),
        sms_credential_identity: "secret-sms-identity".into(),
        enabled: true,
        fence: 0,
    };
    assert!(!format!("{binding:?}").contains("secret-voice"));
    assert_eq!(
        crate::services::mcp_service::tool_title(confirmations::TOOL, &[]).as_deref(),
        Some("Text the current caller")
    );
}

#[tokio::test]
async fn small_synthesis_deltas_share_prepaid_blocks_and_credit_remainder_once() {
    let (t, b, _) = setup_telnyx().await;
    let (s, _) = call_turn(&t, &b).await;
    let mut writer = synthesis::Writer::default();
    // 257 three-scalar deltas, 771 scalars: only two 400-character debits.
    for _ in 0..257 {
        let mut remaining = 3;
        while remaining > 0 {
            let n = writer
                .capacity(&t.f.state, &s)
                .await
                .unwrap()
                .min(remaining);
            writer.attempting(n).unwrap();
            remaining -= n;
        }
    }
    let db = &t.f.state.db;
    assert_eq!(
        db.collection::<ManagedCharacterBlock>(ManagedCharacterBlock::COLLECTION_NAME)
            .count_documents(doc! {"session_id":&s.id})
            .await
            .unwrap(),
        2
    );
    assert_eq!(synthesis::allowance(db, &s.id).await.unwrap().consumed, 800);
    writer.finish(db).await.unwrap();
    writer.finish(db).await.unwrap();
    assert_eq!(synthesis::allowance(db, &s.id).await.unwrap().consumed, 771);
    let last = db
        .collection::<bson::Document>(ManagedCharacterBlock::COLLECTION_NAME)
        .find_one(doc! {"session_id":&s.id,"status":"proven_unwritten"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last.get_i64("credited_characters").unwrap(), 29);
    // A duplicate stale refund cannot credit even the already-written prefix.
    let block: ManagedCharacterBlock = bson::from_document(last).unwrap();
    synthesis::unwritten(db, &block).await.unwrap();
    assert_eq!(synthesis::allowance(db, &s.id).await.unwrap().consumed, 771);
}
#[tokio::test]
async fn crashed_or_uncertain_writer_keeps_entire_prepaid_block_consumed() {
    let (t, b, _) = setup_telnyx().await;
    let (s, _) = call_turn(&t, &b).await;
    let db = &t.f.state.db;
    let mut crashed = synthesis::Writer::default();
    assert_eq!(crashed.capacity(&t.f.state, &s).await.unwrap(), 400);
    crashed.attempting(1).unwrap();
    drop(crashed); // no durable row can prove the rest was unwritten
    let mut uncertain = synthesis::Writer::default();
    uncertain.capacity(&t.f.state, &s).await.unwrap();
    uncertain.attempting(3).unwrap();
    uncertain.uncertain();
    uncertain.finish(db).await.unwrap();
    assert_eq!(synthesis::allowance(db, &s.id).await.unwrap().consumed, 800);
    assert_eq!(
        db.collection::<ManagedCharacterBlock>(ManagedCharacterBlock::COLLECTION_NAME)
            .count_documents(doc! {"session_id":&s.id,"status":"uncertain"})
            .await
            .unwrap(),
        2
    );
}
#[tokio::test]
async fn default_call_has_no_greeting_and_chinese_wrapup_fits_reserved_capacity() {
    let (t, b, _) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v3:no-default-greeting").await;
    let a = synthesis::allowance(&t.f.state.db, &s.id).await.unwrap();
    assert!(a.greeting.is_none());
    assert_eq!(a.consumed, 0);
    for language in ["en-US", "zh-CN", "zh-TW"] {
        let text = synthesis::wrapup(language, a.wrapup.as_deref());
        assert!(text.chars().count() as i64 <= synthesis::WRAPUP_RESERVE);
        if language.starts_with("zh") {
            assert!(text.contains("再见") || text.contains("再見"));
        }
    }
    let wrap = synthesis::wrapup("zh-CN", None);
    synthesis::debit(&t.f.state, &s, wrap, true).await.unwrap();
    assert_eq!(
        synthesis::allowance(&t.f.state.db, &s.id)
            .await
            .unwrap()
            .wrapup_consumed,
        wrap.chars().count() as i64
    );
}
#[tokio::test]
async fn configured_greeting_and_wrapup_are_bounded_and_snapshotted_on_call() {
    let (t, b, _) = setup_telnyx().await;
    for (greeting, wrapup) in [
        (None, Some("界".repeat(301))),
        (Some("x".repeat(401)), None),
    ] {
        assert!(
            synthesis::configure(
                &t.f.state,
                &t.f.admin,
                &t.f.account.id,
                synthesis::VoiceAccountInput {
                    revision: 0,
                    per_call_characters: 700,
                    greeting,
                    wrapup,
                    premium: None
                }
            )
            .await
            .is_err()
        );
    }
    let wrap = "謝謝您的來電，再見。".to_string();
    synthesis::configure(
        &t.f.state,
        &t.f.admin,
        &t.f.account.id,
        synthesis::VoiceAccountInput {
            revision: 0,
            per_call_characters: 700,
            greeting: Some("歡迎來電。".into()),
            wrapup: Some(wrap.clone()),
            premium: None,
        },
    )
    .await
    .unwrap();
    let s = prepare_telnyx(&t, &b, "v3:configured-speech").await;
    let a = synthesis::allowance(&t.f.state.db, &s.id).await.unwrap();
    assert_eq!(a.greeting.as_deref(), Some("歡迎來電。"));
    assert_eq!(synthesis::wrapup("en-US", a.wrapup.as_deref()), wrap);
}
#[tokio::test]
async fn confirmation_refuses_non_call_ended_call_and_disabled_toggle() {
    let (t, v, _sms, s, chat) = confirmation_fixture().await;
    let mut non_call = chat.clone();
    non_call.channel_adapter_origin = None;
    assert!(!confirmations::eligible(&t.f.state.db, &non_call).await);
    assert!(
        confirmations::send(&t.f.state, &non_call, "outside", confirmation("hello", 0))
            .await
            .is_err()
    );
    // Live voice call, but owner revoked the standing authorization.
    confirmations::configure(
        &t.f.state,
        &AuditActor::from_auth_user(&test_auth_user(&v.user_id)),
        &v.id,
        confirmations::ToggleInput {
            enabled: false,
            sms_binding_id: None,
            voice_revision: v.revision,
        },
    )
    .await
    .unwrap();
    assert!(!confirmations::eligible(&t.f.state.db, &chat).await);
    assert!(
        confirmations::send(&t.f.state, &chat, "disabled", confirmation("hello", 0))
            .await
            .is_err()
    );
    session::terminal(&t.f.state, &s, Utc::now(), Some(1), "fixture-ended")
        .await
        .unwrap();
    // Restore only the owner opt-in: ended-session refusal is independent of disable.
    t.f.state
        .db
        .collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&s.managed_claim_id},
            doc! {"$set":{"caller_confirmations":true}},
        )
        .await
        .unwrap();
    assert!(!confirmations::eligible(&t.f.state.db, &chat).await);
    assert!(
        confirmations::send(&t.f.state, &chat, "ended", confirmation("hello", 0))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn integration_reference_is_proved_for_configuration_and_successor_rotation() {
    let (t, mut b, _) = setup_telnyx().await;
    premium(&t, &mut b, 10000).await;
    let db = &t.f.state.db;
    let c = synthesis::config(db, &t.f.account.id)
        .await
        .unwrap()
        .unwrap();
    let p = c.premium.as_ref().unwrap();
    let make_input = |reference: &str| synthesis::VoiceAccountInput {
        revision: c.revision,
        per_call_characters: c.per_call_characters,
        greeting: None,
        wrapup: None,
        premium: Some(synthesis::PremiumInput {
            api_key_ref: reference.into(),
            voices: p.voices.clone(),
            billing_account_identity: p.billing_account_identity.clone(),
            monthly_characters: p.monthly_characters,
            plan_tier: p.plan_tier.clone(),
            oem_terms_version: p.oem_terms_version.clone(),
            admin_acknowledged: true,
            billing_service_id: p.billing_service_id.clone(),
        }),
    };
    assert!(
        synthesis::configure_with_origin(
            &t.f.state,
            &t.f.admin,
            &t.f.account.id,
            make_input("missing-reference"),
            Some(&t.server.uri())
        )
        .await
        .is_err()
    );
    assert_eq!(
        synthesis::config(db, &t.f.account.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        c.revision
    );
    let l = &t.f.line;
    let app = &l.voice.as_ref().unwrap().application_id;
    allow_fixture_operations(
        &t,
        &[
            ("GET", format!("/v2/phone_numbers/{}", l.number_resource_id)),
            (
                "GET",
                format!("/v2/phone_numbers/{}/voice", l.number_resource_id),
            ),
            ("GET", format!("/v2/call_control_applications/{app}")),
        ],
    )
    .await;
    Mock::given(method("GET")).and(path(format!("/v2/phone_numbers/{}",l.number_resource_id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"id":l.number_resource_id,"phone_number":l.phone_number,"status":"active","connection_id":app}}))).mount(&t.server).await;
    Mock::given(method("GET")).and(path(format!("/v2/phone_numbers/{}/voice",l.number_resource_id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"id":l.number_resource_id,"phone_number":l.phone_number,"connection_id":app}}))).mount(&t.server).await;
    Mock::given(method("GET")).and(path(format!("/v2/call_control_applications/{app}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"id":app,"webhook_event_url":custody::stable_url(&t.f.state.config.base_url,&l.id,false),"inbound":{"channel_limit":1}}}))).mount(&t.server).await;
    custody::rotate_with_origin(
        &t.f.state,
        &t.f.admin,
        &t.f.account.id,
        custody::RotationInput {
            expected_revision: 1,
            credential: zeroize::Zeroizing::new("successor-fixture".into()),
        },
        Some(&t.server.uri()),
    )
    .await
    .unwrap();
    let saved = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":&t.f.service.id})
        .await
        .unwrap()
        .unwrap();
    let requests = t.server.received_requests().await.unwrap();
    assert!(requests.iter().any(|r| {
        r.url.path() == "/v2/integration_secrets"
            && r.headers
                .get("authorization")
                .is_some_and(|v| v == "Bearer successor-fixture")
    }));
    Mock::given(method("GET"))
        .and(path("/v2/integration_secrets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
        .with_priority(1)
        .mount(&t.server)
        .await;
    assert!(
        custody::rotate_with_origin(
            &t.f.state,
            &t.f.admin,
            &t.f.account.id,
            custody::RotationInput {
                expected_revision: 1,
                credential: zeroize::Zeroizing::new("unproved-successor".into())
            },
            Some(&t.server.uri())
        )
        .await
        .is_err()
    );
    let after = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.credential_encrypted, after.credential_encrypted);
    // Only metadata listing is requested, never a value retrieval endpoint.
    assert!(
        t.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().contains("integration_secrets"))
            .all(|r| r.url.path() == "/v2/integration_secrets")
    );
}

#[tokio::test]
async fn busy_line_daily_totals_cover_more_than_thousand_effects_with_paged_detail() {
    let (t, b) = setup_call().await;
    let s = prepare(&t, &b).await;
    let mut row = cost_row(&t, &s.id).await;
    let from = Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    row.created_at = from + chrono::Duration::hours(1);
    row.billed_credits = Some(Credits::from_pico(7).unwrap());
    row.required_components = vec![CostKind::CarrierInbound];
    row.policy = Some(ManagedCostPolicy {
        id: id(),
        line_id: t.f.line.id.clone(),
        revision: 1,
        fx: fx(),
        schemas: vec![],
        inventory: None,
    });
    row.components = vec![CostComponent {
        kind: CostKind::CarrierInbound,
        source: CostSource::ProviderRecord,
        provider_record_id: Some("fixture".into()),
        amount: Some("0.000000000001".into()),
        currency: "USD".into(),
        fx_policy_revision: "fx-1".into(),
        supplier_rounded_quantity: Some(1),
    }];
    row.absorbed_seconds = Some(2);
    row.absorbed_sms_parts = 1;
    let mut docs = vec![];
    for n in 0..1105 {
        row.id = id();
        row.effect_id = format!("busy-{n}");
        // Three exceptional effects distinguish estimates, missing coverage and missing FX.
        row.components[0].source = if n == 1102 {
            CostSource::TariffEstimate
        } else {
            CostSource::ProviderRecord
        };
        row.components[0].amount = if n == 1103 {
            None
        } else {
            Some("0.000000000001".into())
        };
        row.components[0].fx_policy_revision = if n == 1104 { "missing-fx" } else { "fx-1" }.into();
        docs.push(costs::evidence_document(&row).unwrap());
    }
    let rows =
        t.f.state
            .db
            .collection::<bson::Document>(ManagedCommsCostEvidence::COLLECTION_NAME);
    rows.delete_one(doc! {"effect_id":&s.id}).await.unwrap();
    rows.insert_many(docs).await.unwrap();
    let q = || costs::ReportQuery {
        from,
        until: from + chrono::Duration::days(1),
        after: None,
        line_id: Some(t.f.line.id.clone()),
    };
    let report = costs::report(&t.f.state.db, q()).await.unwrap();
    let day = &report.lines[0].days[0];
    assert_eq!(day.effect_count, 1105);
    assert_eq!(day.unknown_components, 2);
    assert_eq!(
        day.provider_actual_cost_credits,
        Credits::from_pico(1102 * 100).unwrap().to_string()
    );
    assert_eq!(
        day.estimated_cost_credits,
        Credits::from_pico(100).unwrap().to_string()
    );
    assert_eq!(day.absorbed_seconds, 2210);
    assert_eq!(day.absorbed_sms_parts, 1105);
    assert!(day.actual_margin_credits.is_none());
    assert!(day.estimated_margin_credits.is_none()); // unknown FX/coverage and missing inventory
    let mut after = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = costs::effect_report(
            &t.f.state.db,
            costs::EffectQuery {
                line_id: t.f.line.id.clone(),
                from,
                until: from + chrono::Duration::days(1),
                after,
            },
        )
        .await
        .unwrap();
        assert!(page.effects.len() <= 100);
        for effect in page.effects {
            assert!(seen.insert(effect.effect_id));
        }
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 1105);
    // Detail pagination never changes the totals.
    assert_eq!(
        costs::report(&t.f.state.db, q()).await.unwrap().lines[0].days[0]
            .provider_actual_cost_credits,
        day.provider_actual_cost_credits
    );
}

#[tokio::test]
async fn managed_socket_sends_no_default_greeting_after_verified_setup() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (t, b, _) = setup_telnyx().await;
    let s = prepare_telnyx(&t, &b, "v3:quiet-setup").await;
    let state = t.f.state.clone();
    let voice = b.clone();
    let call = s.clone();
    let app = axum::Router::new().route(
        "/socket",
        axum::routing::get(move |ws: axum::extract::WebSocketUpgrade| {
            let state = state.clone();
            let b = voice.clone();
            let s = call.clone();
            async move { ws.on_upgrade(move |socket| coordinator::socket(state, b, s, socket)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/socket"))
        .await
        .unwrap();
    ws.send(Message::Text(json!({"type":"setup","sessionId":"provider-session","accountSid":b.account_id,"callSid":s.provider_call_id,"from":s.from,"to":s.to,"direction":"inbound"}).to_string().into())).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if session::load(&t.f.state, &s.id)
                .await
                .unwrap()
                .provider_session_id
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), ws.next())
            .await
            .is_err()
    );
    assert_eq!(
        synthesis::allowance(&t.f.state.db, &s.id)
            .await
            .unwrap()
            .consumed,
        0
    );
    ws.close(None).await.unwrap();
    server.abort();
}

async fn forward(db: &mongodb::Database, ids: &[String]) -> crate::errors::AppResult<()> {
    let ids = ids.to_vec();
    crate::services::transaction_service::run_db_only(db, &(db, &ids), |tx, c| {
        Box::pin(
            async move { crate::services::billing::meter::forward_in_session(c.0, tx, c.1).await },
        )
    })
    .await
}
async fn meter_row(t: &LifecycleFixture, request: &str) -> bson::Document {
    t.f.state
        .db
        .collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
        .find_one(doc! {"billing_request_id":request})
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn managed_sweep_releases_only_unadmitted_confirmation_parts() {
    use crate::services::call_channels::managed::reservations;
    let (t, _v, sms, _s, chat) = confirmation_fixture().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/2010-04-01/Accounts/{}/Messages.json",
            sms.account_id
        )))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&t.server)
        .await;
    // An admitted operation whose send is uncertain keeps its forwarded part.
    let admitted = confirmations::send(&t.f.state, &chat, "admitted", confirmation("Booked", 0))
        .await
        .unwrap();
    assert_eq!(admitted["status"], "uncertain");
    let admitted = format!(
        "call-confirmation:{}:0",
        admitted["operation_id"].as_str().unwrap()
    );
    assert_eq!(
        meter_row(&t, &admitted).await.get_bool("forwarded"),
        Ok(true)
    );
    let base = wallet(&t).await.reserved_credits;
    // A process that reserved parts and stopped before admission compensates
    // nothing; one part is old, the other still inside the in-flight window.
    let live = binding::live(&t.f.state, &sms).await.unwrap();
    let crashed = format!("call-confirmation:{}:0", Uuid::new_v4());
    let in_flight = format!("call-confirmation:{}:0", Uuid::new_v4());
    for request in [&crashed, &in_flight] {
        crate::services::call_channels::billing::reserve(&t.f.state, &sms, &live, request)
            .await
            .unwrap();
    }
    assert_eq!(
        wallet(&t).await.reserved_credits,
        base.checked_add(Credits::from_whole(2)).unwrap()
    );
    let old = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(10));
    t.f.state
        .db
        .collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
        .update_many(
            doc! {"billing_request_id":{"$in":[&crashed, &admitted]}},
            doc! {"$set":{"created_at":old}},
        )
        .await
        .unwrap();
    reservations::sweep(&t.f.state).await.unwrap();
    let released = meter_row(&t, &crashed).await;
    assert_eq!(released.get_bool("released"), Ok(true));
    assert_eq!(released.get_str("status"), Ok("abandoned"));
    let waiting = meter_row(&t, &in_flight).await;
    assert_eq!(waiting.get_str("status"), Ok("reserved"));
    assert_eq!(waiting.get_bool("released"), Ok(false));
    // The sweep never releases an admitted operation's part.
    let kept = meter_row(&t, &admitted).await;
    assert_eq!(kept.get_bool("forwarded"), Ok(true));
    assert_eq!(kept.get_bool("released"), Ok(false));
    assert_eq!(
        wallet(&t).await.reserved_credits,
        base.checked_add(Credits::from_whole(1)).unwrap()
    );
    // An admission racing the release refuses instead of binding a released hold;
    // an unreleased part is bound (forwarded) by its admission.
    assert!(matches!(
        forward(&t.f.state.db, std::slice::from_ref(&crashed)).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    forward(&t.f.state.db, std::slice::from_ref(&in_flight))
        .await
        .unwrap();
    assert_eq!(
        meter_row(&t, &in_flight).await.get_bool("forwarded"),
        Ok(true)
    );
}
#[tokio::test]
async fn prepared_managed_sends_finish_unsent_and_fence_the_original_writer() {
    use crate::services::call_channels::managed::reservations;
    let (t, _v, sms, s, _chat) = confirmation_fixture().await;
    let db = &t.f.state.db;
    let live = binding::live(&t.f.state, &sms).await.unwrap();
    let base = wallet(&t).await.reserved_credits;
    let receipt = |session: &str, created_at| EventReceipt {
        id: id(),
        identity_key: id(),
        user_id: sms.user_id.clone(),
        binding_id: sms.id.clone(),
        binding_revision: sms.revision,
        managed_claim_id: s.managed_claim_id.clone(),
        managed_claim_generation: s.managed_claim_generation,
        admitted_reference: s.admitted_reference.clone(),
        session_id: Some(session.into()),
        event_kind: "sms_send".into(),
        status: "prepared".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: Some(format!("managed-sms-out:{session}:0")),
        observed_parts: None,
        reserved_parts: Some(1),
        billing_finalized: false,
        from: Some(sms.phone_number.clone()),
        to: Some(s.from.clone()),
        created_at,
    };
    // Admitted sends: parts reserved, then forwarded inside admission.
    let stale = receipt(&id(), Utc::now() - chrono::Duration::minutes(10));
    let recent = receipt(&id(), Utc::now());
    for send in [&stale, &recent] {
        let request = send.billing_request_id.clone().unwrap();
        crate::services::call_channels::billing::reserve(&t.f.state, &sms, &live, &request)
            .await
            .unwrap();
        forward(db, std::slice::from_ref(&request)).await.unwrap();
        db.collection::<EventReceipt>(EVENTS)
            .insert_one(send)
            .await
            .unwrap();
    }
    assert_eq!(
        wallet(&t).await.reserved_credits,
        base.checked_add(Credits::from_whole(2)).unwrap()
    );
    reservations::finish_prepared_sends(&t.f.state, Utc::now() - chrono::Duration::minutes(5))
        .await
        .unwrap();
    let events = db.collection::<EventReceipt>(EVENTS);
    let finished = events
        .find_one(doc! {"_id":&stale.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.status, "rejected_before_dispatch");
    assert_eq!(finished.delivery, "failed");
    assert!(finished.billing_finalized);
    assert_eq!(
        meter_row(&t, stale.billing_request_id.as_deref().unwrap())
            .await
            .get_bool("released"),
        Ok(true)
    );
    assert_eq!(
        wallet(&t).await.reserved_credits,
        base.checked_add(Credits::from_whole(1)).unwrap()
    );
    // The original writer can no longer pass its dispatch barrier.
    assert_eq!(
        events
            .update_one(
                doc! {"_id":&stale.id,"status":"prepared"},
                doc! {"$set":{"status":"dispatching"}},
            )
            .await
            .unwrap()
            .matched_count,
        0
    );
    // A send inside the recovery window is left to its writer.
    let pending = events
        .find_one(doc! {"_id":&recent.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, "prepared");
    assert!(!pending.billing_finalized);
}

async fn uncertain_confirmation(
    t: &LifecycleFixture,
    sms: &Binding,
    chat: &ChatAuthority,
    invocation: &str,
) -> String {
    Mock::given(method("POST"))
        .and(path(format!(
            "/2010-04-01/Accounts/{}/Messages.json",
            sms.account_id
        )))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&t.server)
        .await;
    let sent = confirmations::send(&t.f.state, chat, invocation, confirmation("Booked", 0))
        .await
        .unwrap();
    assert_eq!(sent["status"], "uncertain");
    sent["operation_id"].as_str().unwrap().to_owned()
}
async fn dispatched_hours_ago(t: &LifecycleFixture, send: &str, hours: i64) {
    t.f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":send},
            doc! {"$set":{"dispatched_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(hours))}},
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn uncertain_part_with_a_known_message_settles_on_the_provider_read() {
    use crate::services::call_channels::managed::reservations;
    let (t, _v, sms, s, chat) = confirmation_fixture().await;
    let operation = uncertain_confirmation(&t, &sms, &chat, "known").await;
    let request = format!("call-confirmation:{operation}:0");
    let held = wallet(&t).await.reserved_credits;
    // The provider message became known without part evidence.
    t.f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&operation},
            doc! {"$set":{"provider_message_id":"SMknown"}},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path(format!(
            "/2010-04-01/Accounts/{}/Messages/SMknown.json",
            sms.account_id
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sid":"SMknown","account_sid":sms.account_id,"from":sms.phone_number,
            "to":s.from,"num_segments":"1","status":"delivered"
        })))
        .mount(&t.server)
        .await;
    // Even past the evidence window, a provider-confirmed read decides the part.
    dispatched_hours_ago(&t, &operation, 73).await;
    reservations::sweep(&t.f.state).await.unwrap();
    let receipt =
        t.f.state
            .db
            .collection::<bson::Document>(EVENTS)
            .find_one(doc! {"_id":&operation})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(receipt.get_bool("billing_finalized"), Ok(true));
    assert_eq!(receipt.get_i64("observed_parts"), Ok(1));
    assert!(!receipt.contains_key("terminal_evidence"));
    let meter = meter_row(&t, &request).await;
    assert_eq!(meter.get_bool("released"), Ok(true));
    assert_eq!(meter.get_i64("quantity"), Ok(1));
    assert_eq!(
        wallet(&t).await.reserved_credits,
        held.checked_sub(Credits::from_whole(1)).unwrap()
    );
}
#[tokio::test]
async fn unconfirmed_part_is_waived_after_the_evidence_window_and_a_late_callback_never_charges() {
    use crate::services::call_channels::managed::{costs, reservations};
    let (t, _v, sms, s, chat) = confirmation_fixture().await;
    let operation = uncertain_confirmation(&t, &sms, &chat, "unconfirmed").await;
    let request = format!("call-confirmation:{operation}:0");
    let before = wallet(&t).await;
    // Inside the window the uncertain part stays held.
    dispatched_hours_ago(&t, &operation, 71).await;
    reservations::sweep(&t.f.state).await.unwrap();
    assert_eq!(
        meter_row(&t, &request).await.get_bool("released"),
        Ok(false)
    );
    dispatched_hours_ago(&t, &operation, 73).await;
    for _ in 0..2 {
        reservations::sweep(&t.f.state).await.unwrap();
    }
    let events = t.f.state.db.collection::<bson::Document>(EVENTS);
    let receipt = events
        .find_one(doc! {"_id":&operation})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.get_str("terminal_evidence"),
        Ok("provider_evidence_unavailable")
    );
    assert_eq!(receipt.get_bool("billing_finalized"), Ok(true));
    assert!(
        receipt
            .get("observed_parts")
            .is_none_or(|v| v == &bson::Bson::Null)
    );
    let op =
        t.f.state
            .db
            .collection::<bson::Document>(ManagedCallConfirmation::COLLECTION_NAME)
            .find_one(doc! {"_id":&operation})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        op.get_str("terminal_evidence"),
        Ok("provider_evidence_unavailable")
    );
    assert_eq!(op.get_str("status"), Ok("uncertain"));
    let meter = meter_row(&t, &request).await;
    assert_eq!(meter.get_bool("released"), Ok(true));
    assert_eq!(meter.get_i64("quantity"), Ok(0));
    let after = wallet(&t).await;
    assert_eq!(
        after.reserved_credits,
        before
            .reserved_credits
            .checked_sub(Credits::from_whole(1))
            .unwrap()
    );
    assert_eq!(after.pending_lago_debits, before.pending_lago_debits);
    // One metadata-only audit, however many passes ran.
    let audits: Vec<bson::Document> =
        t.f.state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type":"channel_adapter_sms_provider_evidence_unavailable"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(audits.len(), 1);
    let text = format!("{:?}", audits[0]);
    assert!(!text.contains("Booked") && !text.contains(&s.from));
    // Cost evidence keeps the supplier component unknown and fully absorbed.
    costs::capture(&t.f.state, &cost_row(&t, &operation).await.id)
        .await
        .unwrap();
    let cost = cost_row(&t, &operation).await;
    assert_eq!(
        cost.terminal_evidence.as_deref(),
        Some("provider_evidence_unavailable")
    );
    assert_eq!(cost.absorbed_sms_parts, 1);
    assert!(cost.capture_complete);
    assert!(
        cost.components
            .iter()
            .all(|c| c.source == CostSource::Unknown && c.amount.is_none())
    );
    // A delivery callback after the waiver records evidence and never charges.
    crate::services::call_channels::sms_delivery::record_correlated(
        &t.f.state,
        &sms,
        crate::services::call_channels::sms_delivery::Receipt {
            event: "late-delivery",
            message: "SMlate",
            from: &sms.phone_number,
            to: &s.from,
            status: "delivered",
            parts: Some(1),
            send: Some(&operation),
        },
    )
    .await
    .unwrap();
    let meter = meter_row(&t, &request).await;
    assert_eq!(meter.get_i64("quantity"), Ok(0));
    let late = wallet(&t).await;
    assert_eq!(late.reserved_credits, after.reserved_credits);
    assert_eq!(late.pending_lago_debits, after.pending_lago_debits);
    assert_eq!(late.balance_credits, after.balance_credits);
}
#[tokio::test]
async fn interrupted_waiver_finishes_at_zero_when_part_evidence_arrives() {
    let (t, _v, sms, s, chat) = confirmation_fixture().await;
    let operation = uncertain_confirmation(&t, &sms, &chat, "interrupted").await;
    let request = format!("call-confirmation:{operation}:0");
    let before = wallet(&t).await;
    // The waiver committed but its zero settlement did not run yet.
    let events = t.f.state.db.collection::<bson::Document>(EVENTS);
    events
        .update_one(
            doc! {"_id":&operation},
            doc! {"$set":{"terminal_evidence":"provider_evidence_unavailable"}},
        )
        .await
        .unwrap();
    crate::services::call_channels::sms_delivery::record_correlated(
        &t.f.state,
        &sms,
        crate::services::call_channels::sms_delivery::Receipt {
            event: "after-waiver",
            message: "SMafterwaiver",
            from: &sms.phone_number,
            to: &s.from,
            status: "delivered",
            parts: Some(1),
            send: Some(&operation),
        },
    )
    .await
    .unwrap();
    let receipt = events
        .find_one(doc! {"_id":&operation})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.get_bool("billing_finalized"), Ok(true));
    assert!(
        receipt
            .get("observed_parts")
            .is_none_or(|v| v == &bson::Bson::Null)
    );
    let meter = meter_row(&t, &request).await;
    assert_eq!(meter.get_bool("released"), Ok(true));
    assert_eq!(meter.get_i64("quantity"), Ok(0));
    let after = wallet(&t).await;
    assert_eq!(
        after.reserved_credits,
        before
            .reserved_credits
            .checked_sub(Credits::from_whole(1))
            .unwrap()
    );
    assert_eq!(after.pending_lago_debits, before.pending_lago_debits);
}
/// A managed Telnyx SMS binding beside the Telnyx voice fixture, and one
/// admitted call session on its line.
async fn telnyx_sms_fixture() -> (LifecycleFixture, Binding, CallSession) {
    let (mut t, voice, _signer) = setup_telnyx().await;
    let db = t.f.state.db.clone();
    let service = price_service(&t, ManagedCommsRole::Sms, BillingMetric::Requests).await;
    let mut sms = voice.clone();
    sms.id = id();
    sms.profile = Profile::TelnyxSms;
    t.f.line = db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":&t.f.line.id})
        .await
        .unwrap()
        .unwrap();
    t.f.line.sms = Some(Capability {
        application_id: sms.application_id.clone(),
        billing_service_id: service,
    });
    t.f.line.regulatory.sms_sender_registration = Some("approved fixture sender".into());
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .replace_one(doc! {"_id":&t.f.line.id}, &t.f.line)
        .await
        .unwrap();
    let CredentialBinding::ManagedLine {
        managed_claim_id, ..
    } = &sms.credential
    else {
        unreachable!()
    };
    db.collection::<bson::Document>(ManagedLineClaim::COLLECTION_NAME)
        .update_one(
            doc! {"_id":managed_claim_id},
            doc! {"$set":{"sms_enabled":true,"sms_binding_id":&sms.id},"$inc":{"fence":1_i64}},
        )
        .await
        .unwrap();
    db.collection::<Binding>(BINDINGS)
        .insert_one(&sms)
        .await
        .unwrap();
    binding::live(&t.f.state, &sms).await.unwrap();
    // SMS part reservations verify the owner's plan entitlements.
    Mock::given(method("GET"))
        .and(path("/api/v1/subscriptions/subscription/entitlements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entitlements":[{"code":"*"}]
        })))
        .mount(&t.server)
        .await;
    let s = prepare_telnyx(&t, &voice, "v2:telnyx-sms-evidence==").await;
    (t, sms, s)
}
/// One forwarded two-part managed reply whose send outcome is unknown but whose
/// Telnyx message ID is known.
async fn telnyx_uncertain_reply(
    t: &LifecycleFixture,
    sms: &Binding,
    session: &str,
    message: &str,
) -> (String, Vec<String>) {
    let live = binding::live(&t.f.state, sms).await.unwrap();
    let requests: Vec<String> = (0..2)
        .map(|part| format!("managed-sms-out:{session}:{part}"))
        .collect();
    for request in &requests {
        crate::services::call_channels::billing::reserve(&t.f.state, sms, &live, request)
            .await
            .unwrap();
    }
    forward(&t.f.state.db, &requests).await.unwrap();
    let CredentialBinding::ManagedLine {
        managed_claim_id,
        managed_claim_generation,
        ..
    } = &sms.credential
    else {
        unreachable!()
    };
    let sent = EventReceipt {
        id: id(),
        identity_key: id(),
        user_id: sms.user_id.clone(),
        binding_id: sms.id.clone(),
        binding_revision: sms.revision,
        managed_claim_id: Some(managed_claim_id.clone()),
        managed_claim_generation: Some(*managed_claim_generation),
        admitted_reference: Some(
            occurrence()
                .reference(&t.f.line, &t.f.account, None, Some(message))
                .unwrap(),
        ),
        session_id: Some(session.into()),
        event_kind: "sms_send".into(),
        status: "uncertain".into(),
        provider_message_id: Some(message.into()),
        delivery: "unknown".into(),
        billing_request_id: Some(requests[0].clone()),
        observed_parts: None,
        reserved_parts: Some(2),
        billing_finalized: false,
        from: Some(sms.phone_number.clone()),
        to: Some("+18005550999".into()),
        created_at: Utc::now(),
    };
    t.f.state
        .db
        .collection::<EventReceipt>(EVENTS)
        .insert_one(&sent)
        .await
        .unwrap();
    (sent.id, requests)
}
async fn allow_telnyx_message_read(t: &LifecycleFixture) {
    allow_fixture_operations(t, &[("GET", "/v2/messages/{message_id}".into())]).await;
}
#[tokio::test]
async fn managed_telnyx_part_with_a_known_message_settles_on_the_provider_read() {
    let (t, sms, s) = telnyx_sms_fixture().await;
    allow_telnyx_message_read(&t).await;
    let base = wallet(&t).await;
    let message = Uuid::new_v4().to_string();
    let (send, requests) = telnyx_uncertain_reply(&t, &sms, &s.id, &message).await;
    assert_eq!(
        wallet(&t).await.reserved_credits,
        base.reserved_credits
            .checked_add(Credits::from_whole(2))
            .unwrap()
    );
    Mock::given(method("GET"))
        .and(path(format!("/v2/messages/{message}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
            "id":&message,"record_type":"message","direction":"outbound",
            "messaging_profile_id":&sms.application_id,
            "from":{"phone_number":&sms.phone_number},
            "to":[{"phone_number":"+18005550999","status":"delivered"}],
            "parts":1
        }})))
        .expect(1)
        .mount(&t.server)
        .await;
    // The evidence sweep reads the exact message through the canonical account.
    crate::services::call_channels::sms_delivery::sweep(&t.f.state)
        .await
        .unwrap();
    let receipt =
        t.f.state
            .db
            .collection::<bson::Document>(EVENTS)
            .find_one(doc! {"_id":&send})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(receipt.get_bool("billing_finalized"), Ok(true));
    assert_eq!(receipt.get_i64("observed_parts"), Ok(1));
    assert_eq!(receipt.get_str("delivery"), Ok("delivered"));
    assert!(!receipt.contains_key("terminal_evidence"));
    let charged = meter_row(&t, &requests[0]).await;
    assert_eq!(charged.get_i64("quantity"), Ok(1));
    assert_eq!(charged.get_bool("released"), Ok(true));
    let unused = meter_row(&t, &requests[1]).await;
    assert_eq!(unused.get_i64("quantity"), Ok(0));
    assert_eq!(unused.get_bool("released"), Ok(true));
    assert_eq!(wallet(&t).await.reserved_credits, base.reserved_credits);
}
#[tokio::test]
async fn managed_telnyx_refused_or_failed_read_falls_through_to_the_evidence_window() {
    use crate::services::call_channels::managed::reservations;
    let (t, sms, _s) = telnyx_sms_fixture().await;
    let base = wallet(&t).await;
    // Without the policy rule the credential is never used for the read.
    let refused = Uuid::new_v4().to_string();
    Mock::given(method("GET"))
        .and(path(format!("/v2/messages/{refused}")))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&t.server)
        .await;
    let (refused_send, refused_parts) = telnyx_uncertain_reply(&t, &sms, &id(), &refused).await;
    dispatched_hours_ago(&t, &refused_send, 73).await;
    reservations::sweep(&t.f.state).await.unwrap();
    // An allowed read the provider fails keeps the part held inside the window.
    allow_telnyx_message_read(&t).await;
    let failed = Uuid::new_v4().to_string();
    Mock::given(method("GET"))
        .and(path(format!("/v2/messages/{failed}")))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&t.server)
        .await;
    let (failed_send, failed_parts) = telnyx_uncertain_reply(&t, &sms, &id(), &failed).await;
    // The waiver's last read happens only once the window has passed.
    dispatched_hours_ago(&t, &failed_send, 71).await;
    reservations::sweep(&t.f.state).await.unwrap();
    assert_eq!(
        meter_row(&t, &failed_parts[0]).await.get_bool("released"),
        Ok(false)
    );
    dispatched_hours_ago(&t, &failed_send, 73).await;
    reservations::sweep(&t.f.state).await.unwrap();
    let events = t.f.state.db.collection::<bson::Document>(EVENTS);
    for (send, parts) in [
        (&refused_send, &refused_parts),
        (&failed_send, &failed_parts),
    ] {
        let receipt = events.find_one(doc! {"_id":send}).await.unwrap().unwrap();
        assert_eq!(
            receipt.get_str("terminal_evidence"),
            Ok("provider_evidence_unavailable")
        );
        assert_eq!(receipt.get_bool("billing_finalized"), Ok(true));
        assert!(
            receipt
                .get("observed_parts")
                .is_none_or(|v| v == &bson::Bson::Null)
        );
        for part in parts {
            let meter = meter_row(&t, part).await;
            assert_eq!(meter.get_bool("released"), Ok(true));
            assert_eq!(meter.get_i64("quantity"), Ok(0));
        }
    }
    let after = wallet(&t).await;
    assert_eq!(after.reserved_credits, base.reserved_credits);
    assert_eq!(after.pending_lago_debits, base.pending_lago_debits);
}
