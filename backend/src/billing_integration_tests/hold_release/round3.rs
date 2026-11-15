use super::*;
use futures::TryStreamExt;

const OBS: &str = "billing_hold_observations";
const GRANTS: &str = "credit_grants";
const PERIODS: &str = "usage_allowance_periods";

async fn observe(f: &Fixture) -> bson::Document {
    f.db.collection::<bson::Document>(OBS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
}

pub(super) async fn expire_pass(f: &Fixture) {
    f.db.collection::<bson::Document>("billing_account_verify_lease")
        .update_one(
            doc! {},
            doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
}

pub(super) async fn confirmed_before_repair(f: &Fixture) {
    f.accounts_clean().await;
    let gate = Gate::new(Phase::BeforeHoldRepair);
    let mut pass = Box::pin(GATE.scope(
        gate.clone(),
        account_reconciliation::hold_pass_for_test(&f.db),
    ));
    tokio::select! {
        result = &mut pass => panic!("missing repair barrier: {result:?}"),
        () = gate.entered() => {},
    }
    assert!(observe(f).await.get_bool("persistent").unwrap());
    drop(pass);
    expire_pass(f).await;
}

async fn idle_pages(f: &Fixture) {
    let (ctx, _) = f.mixed_request().await;
    let opened = f.service.open(&ctx).await.unwrap();
    f.service.fail(&opened, "fixture release").await.unwrap();
    let mut templates = Vec::new();
    for collection in [BILLING_WALLET, GRANTS, PERIODS] {
        templates.push(
            f.db.collection::<bson::Document>(collection)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap(),
        );
        f.db.collection::<bson::Document>(collection)
            .delete_many(doc! {})
            .await
            .unwrap();
    }
    for collection in [USAGE_METER, "billing_ledger", "billing_account_balances"] {
        f.db.collection::<bson::Document>(collection)
            .delete_many(doc! {})
            .await
            .unwrap();
    }
    for (collection, template) in [BILLING_WALLET, GRANTS, PERIODS].into_iter().zip(templates) {
        let rows: Vec<_> = (0..1000)
            .map(|i| {
                let mut row = template.clone();
                row.insert("_id", format!("{collection}-{i:04}"));
                if collection == BILLING_WALLET {
                    row.insert("owner_id", format!("idle-{i:04}"));
                    row.insert("balance_credits", Credits::ZERO);
                    row.insert("pending_lago_debits", Credits::ZERO);
                } else if collection == GRANTS {
                    row.insert("remaining", Credits::ZERO);
                }
                row
            })
            .collect();
        f.db.collection::<bson::Document>(collection)
            .insert_many(rows)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn hold_round3_idle_pages_have_constant_command_cost_and_active_rows_are_checked() {
    let f = Fixture::new().await;
    idle_pages(&f).await;
    f.commands.lock().unwrap().clear();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    let commands = std::mem::take(&mut *f.commands.lock().unwrap());
    println!(
        "HOLD_R3_IDLE_COMMANDS count={} {commands:?}",
        commands.len()
    );
    assert_eq!(
        commands
            .iter()
            .filter(|c| *c == "find:billing_hold_observations")
            .count(),
        3,
        "one observation read per nonempty page"
    );
    assert_eq!(
        commands
            .iter()
            .filter(|c| *c == "aggregate:usage_meter")
            .count(),
        3,
        "one presence query per nonempty page"
    );
    assert!(!commands.iter().any(|c| c == "find:usage_meter"));
    assert_eq!(commands.len(), 33, "constant page budget: {commands:?}");
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"_id":"billing_wallet-0000"},
            doc! {"$set":{"reserved_credits":Credits::from_whole(1)}},
        )
        .await
        .unwrap();
    for (i, hold) in [(1, Credits::ZERO), (2, Credits::from_whole(1))] {
        let wallet_id = format!("billing_wallet-{i:04}");
        f.db.collection::<bson::Document>(USAGE_METER).insert_one(doc! {"_id":format!("live-{i}"),"transaction_id":format!("live-{i}"),"wallet_id":&wallet_id,"billing_owner_id":format!("idle-{i:04}"),"released":false,"reserved_credits":hold}).await.unwrap();
        f.db.collection::<bson::Document>(BILLING_WALLET)
            .update_one(
                doc! {"_id":wallet_id},
                doc! {"$set":{"reserved_credits":hold}},
            )
            .await
            .unwrap();
    }
    f.commands.lock().unwrap().clear();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    let active = std::mem::take(&mut *f.commands.lock().unwrap());
    println!("HOLD_R3_ACTIVE_COMMANDS count={} {active:?}", active.len());
    assert_eq!(
        active.iter().filter(|c| *c == "find:usage_meter").count(),
        3,
        "counter, zero-share presence, nonzero presence"
    );
    assert_eq!(
        f.db.collection::<bson::Document>(OBS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_round3_pre_cutover_page_has_no_hold_commands() {
    let f = Fixture::new().await;
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .delete_many(doc! {})
        .await
        .unwrap();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .insert_many((0..1000).map(|i| doc! {"_id":format!("legacy-{i:04}")}))
        .await
        .unwrap();
    f.commands.lock().unwrap().clear();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    let commands = std::mem::take(&mut *f.commands.lock().unwrap());
    assert!(
        !commands
            .iter()
            .any(|c| c == "find:billing_hold_observations"
                || c == "find:usage_meter"
                || c == "aggregate:usage_meter"),
        "pre-cutover hold reads: {commands:?}"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_round3_partial_indexes_track_live_and_benefit_rows() {
    let f = Fixture::new().await;
    let meters = f.db.collection::<bson::Document>(USAGE_METER);
    let indexes = meters
        .list_indexes()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    for (keys, partial) in [
        (
            doc! {"wallet_id":1,"released":1,"_id":1},
            doc! {"released":false},
        ),
        (
            doc! {"funding.grant_reservations.grant_id":1,"funding.settled":1,"_id":1},
            doc! {"funding.grant_reservations.grant_id":{"$exists":true}},
        ),
        (
            doc! {"funding.allowance_reservations.period_id":1,"funding.settled":1,"_id":1},
            doc! {"funding.allowance_reservations.period_id":{"$exists":true}},
        ),
    ] {
        let index = indexes.iter().find(|i| i.keys == keys).unwrap();
        assert_eq!(
            index
                .options
                .as_ref()
                .and_then(|o| o.partial_filter_expression.as_ref()),
            Some(&partial)
        );
    }
    let mut rows:Vec<_>=(0..3000).map(|i|doc! {"_id":format!("history-{i}"),"transaction_id":format!("history-{i}"),"wallet_id":"history-wallet","released":true}).collect();
    for i in 0..100 {
        rows.push(doc! {"_id":format!("benefit-history-{i}"),"transaction_id":format!("benefit-history-{i}"),"wallet_id":"history-wallet","released":true,"funding":{"settled":true,"grant_reservations":[{"grant_id":"g"}],"allowance_reservations":[{"period_id":"p"}]}});
    }
    for i in 0..3 {
        rows.push(doc! {"_id":format!("live-{i}"),"transaction_id":format!("live-{i}"),"wallet_id":"live-wallet","released":false,"reserved_credits":Credits::from_whole(1)});
    }
    // Legacy funding has no settled field and must remain discoverable.
    rows.push(doc! {"_id":"legacy-benefit","transaction_id":"legacy-benefit","wallet_id":"live-wallet","released":false,"reserved_credits":Credits::ZERO,"funding":{"grant_reservations":[{"grant_id":"g"}],"allowance_reservations":[{"period_id":"p"}]}});
    meters.insert_many(rows).await.unwrap();
    let validation =
        f.db.run_command(doc! {"validate":USAGE_METER,"full":true})
            .await
            .unwrap();
    let counts = validation.get_document("keysPerIndex").unwrap();
    println!("HOLD_R3_INDEX_KEYS {counts:?}");
    for (name, count) in [
        ("wallet_id_1_released_1__id_1", 4),
        (
            "funding.grant_reservations.grant_id_1_funding.settled_1__id_1",
            101,
        ),
        (
            "funding.allowance_reservations.period_id_1_funding.settled_1__id_1",
            101,
        ),
    ] {
        assert_eq!(
            counts
                .get(name)
                .and_then(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from))),
            Some(count)
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_round3_same_pass_revisit_preserves_confirmation_until_assets_recover() {
    let f = Fixture::new().await;
    leak_old_release(&f).await;
    confirmed_before_repair(&f).await;
    let mut other =
        f.db.collection::<bson::Document>(BILLING_WALLET)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    other.insert("_id", "other-wallet");
    other.insert("owner_id", "other-owner");
    other.insert("balance_credits", Credits::from_whole(1));
    other.insert("reserved_credits", Credits::ZERO);
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .insert_one(other)
        .await
        .unwrap();
    account_reconciliation::hold_pass_for_test(&f.db)
        .await
        .unwrap();
    let before = observe(&f).await;
    assert!(before.get_bool("persistent").unwrap());
    // Preserve database time; this call simulates a revisit of the held cursor.
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    let after = observe(&f).await;
    assert_eq!(
        after, before,
        "same-pass revisit must preserve the observation byte-for-byte"
    );
    assert_eq!(repair_audits(&f).await, 0);
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"_id":"other-wallet"},
            doc! {"$set":{"balance_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    assert_eq!(repair_audits(&f).await, 1);
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        Credits::ZERO
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_round3_incomplete_hold_does_not_change_asset_certificate() {
    let f = Fixture::new().await;
    let current = wallet(&f.db, &f.owner).await;
    f.db.collection::<bson::Document>(USAGE_METER).insert_many((0..10001).map(|i|doc! {"_id":format!("bound-{i}"),"transaction_id":format!("bound-{i}"),"wallet_id":&current.id,"billing_owner_id":&f.owner,"released":false,"reserved_credits":Credits::from_pico(1).unwrap()})).await.unwrap();
    f.accounts_clean().await;
    let status = f
        .db
        .collection::<crate::models::chain_verify_status::ChainVerifyStatus>("chain_verify_status")
        .find_one(doc! {"_id":"billing_accounts"})
        .await
        .unwrap()
        .unwrap();
    assert!(
        status.last_full_pass_at.is_some(),
        "hold bound must not block asset certification"
    );
    assert!(status.break_detail.is_none());
    assert_eq!(repair_audits(&f).await, 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_round3_malformed_resources_are_isolated_and_accounts_stay_clean() {
    let f = Fixture::new().await;
    let (_, grant_id) = f.mixed_request().await;
    f.service.open(&f.request()).await.unwrap();
    f.db.collection::<bson::Document>(GRANTS)
        .update_one(
            doc! {"_id":&grant_id},
            doc! {"$set":{"scope":"invalid","reserved":Credits::from_whole(1)}},
        )
        .await
        .unwrap();
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_one(doc! {}, doc! {"$set":{"reserved_credits":"not-a-credit"}})
        .await
        .unwrap();
    let result = account_reconciliation::hold_pass_for_test(&f.db).await;
    assert!(
        result.is_ok(),
        "one malformed resource cannot abort the pass: {result:?}"
    );
    let incomplete =
        f.db.collection::<bson::Document>(OBS)
            .find(doc! {"exact":false})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
    assert_eq!(incomplete.len(), 2);
    let status = f
        .db
        .collection::<crate::models::chain_verify_status::ChainVerifyStatus>("chain_verify_status")
        .find_one(doc! {"_id":"billing_accounts"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.outcome, ChainVerifyOutcome::Ok);
    assert!(status.last_full_pass_at.is_some());
    assert!(status.break_detail.is_none());
    assert_eq!(repair_audits(&f).await, 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_round3_missing_owner_benefits_and_signed_wallet_deficits_are_reported() {
    let f = Fixture::new().await;
    let (ctx, grant_id) = f.mixed_request().await;
    let opened = f.service.open(&ctx).await.unwrap();
    f.service.fail(&opened, "fixture").await.unwrap();
    let mut grant =
        f.db.collection::<bson::Document>(GRANTS)
            .find_one(doc! {"_id":grant_id})
            .await
            .unwrap()
            .unwrap();
    grant.insert("_id", "orphan-grant");
    grant.insert("recipient_user_id", "no-wallet");
    grant.insert("remaining", Credits::ZERO);
    grant.insert("reserved", Credits::from_whole(2));
    f.db.collection::<bson::Document>(GRANTS)
        .insert_one(grant)
        .await
        .unwrap();
    let mut period =
        f.db.collection::<bson::Document>(PERIODS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    period.insert("_id", "orphan-period");
    period.insert("owner_user_id", "no-wallet");
    period.insert("reserved_quantity", 3_i64);
    f.db.collection::<bson::Document>(PERIODS)
        .insert_one(period)
        .await
        .unwrap();
    f.db.collection::<bson::Document>(GRANTS)
        .update_one(
            doc! {"recipient_user_id":&f.owner},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    f.service.open(&f.request()).await.unwrap();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"reserved_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    f.accounts_clean().await;
    f.accounts_clean().await;
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    let detail = diagnostics
        .iter()
        .filter(|d| d.code.starts_with("billing_hold_conservation"))
        .map(|d| format!("{} {}", d.code, d.detail))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        detail.contains("orphan-grant") && detail.contains("orphan-period"),
        "benefits without a wallet must be visible: {detail}"
    );
    for (kind, difference) in [
        ("grant", "unknown"),
        ("allowance", "unknown"),
        ("wallet_deficit", "-1.09"),
    ] {
        assert!(
            detail.contains(kind) && detail.contains(difference),
            "kind/difference missing: {detail}"
        );
        let row =
            f.db.collection::<bson::Document>(OBS)
                .find_one(if difference == "unknown" {
                    doc! {"kind":kind,"exact":false}
                } else {
                    doc! {"kind":kind,"difference":difference}
                })
                .await
                .unwrap();
        assert!(row.is_some(), "observation missing {kind}/{difference}");
    }
    assert_eq!(repair_audits(&f).await, 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_round3_repair_removes_diagnostic_atomically() {
    for phase in [
        Phase::AfterHoldRepairWallet,
        Phase::AfterHoldRepairAudit,
        Phase::AfterHoldObservationDelete,
    ] {
        let f = Fixture::new().await;
        leak_old_release(&f).await;
        f.accounts_clean().await;
        let before = observe(&f).await;
        let gate = Gate::new(phase);
        let mut pass = Box::pin(GATE.scope(
            gate.clone(),
            account_reconciliation::hold_pass_for_test(&f.db),
        ));
        tokio::select! { result=&mut pass=>panic!("missing {phase:?}: {result:?}"), ()=gate.entered()=>{} }
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            "1.09".parse().unwrap()
        );
        assert_eq!(repair_audits(&f).await, 0);
        assert!(observe(&f).await.get_bool("persistent").unwrap());
        drop(pass);
        expire_pass(&f).await;
        f.accounts_clean().await;
        assert_eq!(repair_audits(&f).await, 1);
        assert_eq!(
            f.db.collection::<bson::Document>(OBS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "repair and observation deletion must commit together (initial pass {:?})",
            before.get_str("pass")
        );
        assert!(
            crate::services::billing::startup_diagnostics(&f.db)
                .await
                .unwrap()
                .iter()
                .all(|d| !d.code.starts_with("billing_hold_conservation"))
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_round3_wrapping_pass_deletes_removed_resource_observations() {
    let f = Fixture::new().await;
    f.service.open(&f.request()).await.unwrap();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"reserved_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    f.accounts_clean().await;
    f.accounts_clean().await;
    assert!(observe(&f).await.get_bool("persistent").unwrap());
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .delete_many(doc! {})
        .await
        .unwrap();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    assert_eq!(
        f.db.collection::<bson::Document>(OBS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "completed pass removes observations for absent resources"
    );
    assert!(
        crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap()
            .iter()
            .all(|d| !d.code.starts_with("billing_hold_conservation"))
    );
    f.db.drop().await.unwrap();
}
