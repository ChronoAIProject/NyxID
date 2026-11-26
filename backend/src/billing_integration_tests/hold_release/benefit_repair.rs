use super::*;

const GRANTS: &str = "credit_grants";
const PERIODS: &str = "usage_allowance_periods";
const OBS: &str = "billing_hold_observations";

async fn seeded() -> (Fixture, BillingRouteContext, String) {
    let f = Fixture::new().await;
    let (ctx, grant) = f.mixed_request().await;
    let opened = f.service.open(&ctx).await.unwrap();
    f.service.fail(&opened, "fixture release").await.unwrap();
    (f, ctx, grant)
}

async fn resource(f: &Fixture, collection: &str) -> bson::Document {
    f.db.collection::<bson::Document>(collection)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
}

async fn leak(f: &Fixture) {
    let dust: Credits = "0.0000008".parse().unwrap();
    // The historical admission committed these increments before meter insertion.
    f.db.collection::<bson::Document>(GRANTS)
        .update_one(
            doc! {},
            doc! {"$inc":{
                "reserved":dust,"service_limits.0.reserved":dust,
            }},
        )
        .await
        .unwrap();
    f.db.collection::<bson::Document>(PERIODS)
        .update_one(doc! {}, doc! {"$inc":{"reserved_quantity":1_i64}})
        .await
        .unwrap();
}

pub(super) async fn audits(f: &Fixture) -> Vec<bson::Document> {
    use futures::TryStreamExt;
    f.db.collection::<bson::Document>("audit_log")
        .find(doc! {"event_type":{"$in":[
            "billing_grant_hold_excess_repaired","billing_allowance_hold_excess_repaired",
        ]}})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test]
async fn benefit_hold_repair_historical_orphans_return_exact_counters_once() {
    let (f, _, _) = seeded().await;
    leak(&f).await;
    f.accounts_clean().await;
    assert!(audits(&f).await.is_empty(), "one observation never repairs");
    f.accounts_clean().await;
    let grant = resource(&f, GRANTS).await;
    let period = resource(&f, PERIODS).await;
    let recorded = audits(&f).await;
    for audit in &recorded {
        let data = audit.get_document("event_data").unwrap();
        assert_ne!(
            data.get_str("previous_pass").unwrap(),
            data.get_str("pass").unwrap()
        );
        assert!(!data.get_str("resource_id").unwrap().is_empty());
        let amounts = data.get_document("amounts").unwrap();
        if audit.get_str("event_type").unwrap() == "billing_grant_hold_excess_repaired" {
            assert_eq!(
                amounts,
                &doc! {"rule:release-test":"0.0000008", "total":"0.0000008"}
            );
        } else {
            assert_eq!(amounts, &doc! {"total":"1"});
        }
    }
    f.accounts_clean().await;
    let replay = audits(&f).await;
    f.db.drop().await.unwrap();
    assert_eq!(
        grant.get("reserved"),
        Some(&bson::Bson::from(Credits::ZERO))
    );
    assert_eq!(
        grant.get_array("service_limits").unwrap()[0]
            .as_document()
            .unwrap()
            .get("reserved"),
        Some(&bson::Bson::from(Credits::ZERO))
    );
    assert_eq!(period.get_i64("reserved_quantity").unwrap(), 0);
    assert_eq!(recorded.len(), 2);
    assert_eq!(replay, recorded, "the same excess cannot be returned twice");
}

async fn leak_one(f: &Fixture, collection: &str) {
    let update = if collection == GRANTS {
        doc! {"$inc":{"reserved":Credits::from_whole(1),"service_limits.0.reserved":Credits::from_whole(1)}}
    } else {
        doc! {"$inc":{"reserved_quantity":1_i64}}
    };
    f.db.collection::<bson::Document>(collection)
        .update_one(doc! {}, update)
        .await
        .unwrap();
}
fn fresh(ctx: &BillingRouteContext) -> BillingRouteContext {
    let mut ctx = ctx.clone();
    ctx.billing_request_id = Uuid::new_v4().to_string();
    ctx
}
async fn immediate(f: &Fixture) {
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
}
async fn clear_lease(f: &Fixture) {
    round3::expire_pass(f).await;
}

#[tokio::test]
async fn benefit_hold_repair_age_per_counter_and_cost() {
    let (f, _, _) = seeded().await;
    // Rule-only excess is independent of a correct total.
    f.db.collection::<bson::Document>(GRANTS)
        .update_one(
            doc! {},
            doc! {"$inc":{"service_limits.0.reserved":Credits::from_whole(1)}},
        )
        .await
        .unwrap();
    leak_one(&f, PERIODS).await;
    immediate(&f).await;
    immediate(&f).await;
    assert!(
        audits(&f).await.is_empty(),
        "distinct rapid passes are insufficient"
    );
    f.commands.lock().unwrap().clear();
    f.accounts_clean().await;
    let commands = std::mem::take(&mut *f.commands.lock().unwrap());
    for segment in commands.split_inclusive(|s| s == "commitTransaction:") {
        if segment
            .iter()
            .any(|s| s == "delete:billing_hold_observations")
        {
            println!(
                "BENEFIT_HOLD_REPAIR_COMMANDS count={} {segment:?}",
                segment.len()
            );
        }
    }
    assert_eq!(audits(&f).await.len(), 2);
    f.holds_clean().await;
    let verified = crate::services::audit_chain_service::verify_chain(
        &f.db,
        crate::services::audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        Some(100),
    )
    .await
    .unwrap();
    assert_eq!(
        verified.status,
        crate::services::audit_chain_service::AuditChainStatus::Ok
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn benefit_hold_repair_changed_identity_counter_rule_or_receipt_restarts_confirmation() {
    for case in ["identity", "counter", "rule", "receipt", "row"] {
        let (f, ctx, _) = seeded().await;
        let live = f.service.open(&fresh(&ctx)).await.unwrap();
        leak_one(&f, GRANTS).await;
        f.accounts_clean().await;
        match case {
            "identity" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$set":{"reason":"changed"}})
                    .await
                    .unwrap();
            }
            "counter" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$inc":{"reserved":Credits::from_whole(1)}})
                    .await
                    .unwrap();
            }
            "rule" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$set":{"service_limits.0.label":"changed"}})
                    .await
                    .unwrap();
            }
            "receipt" => {
                let row = usage_row(&f.db, &live.route.as_ref().unwrap().billing_request_id).await;
                let gid = resource(&f, GRANTS)
                    .await
                    .get_str("_id")
                    .unwrap()
                    .to_owned();
                f.db.collection::<bson::Document>(USAGE_METER).update_one(doc!{"_id":&row.id},doc!{"$push":{"funding.grant_consumptions":{"grant_id":&gid,"operation_id":format!("{}:grant-extra:{}",row.id,gid),"service_limit_key":"release-test","amount":Credits::ZERO}}}).await.unwrap();
            }
            "row" => {
                f.service.fail(&live, "concurrent release").await.unwrap();
            }
            _ => unreachable!(),
        }
        f.accounts_clean().await;
        assert!(
            audits(&f).await.is_empty(),
            "{case}: changed snapshot cannot repair"
        );
        immediate(&f).await;
        assert!(
            audits(&f).await.is_empty(),
            "{case}: financial change resets database-time age"
        );
        f.accounts_clean().await;
        assert_eq!(audits(&f).await.len(), 1, "{case}");
        f.holds_clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn benefit_hold_repair_rejects_deficits_incomplete_owners_keys_allocations_and_receipts() {
    for case in [
        "deficit",
        "mixed_sign",
        "negative",
        "bad_amount",
        "empty_key",
        "duplicate_key",
        "missing_owner",
        "deleted_owner",
        "wrong_row_owner",
        "missing_amount",
        "unknown_key",
        "unknown_receipt",
        "wrong_receipt_id",
        "duplicate_receipt",
        "both_receipts",
        "allowance_deficit",
        "allowance_missing_allocation",
        "allowance_receipt",
    ] {
        let (f, ctx, gid) = seeded().await;
        let request = fresh(&ctx);
        f.service.open(&request).await.unwrap();
        let row = usage_row(&f.db, &request.billing_request_id).await;
        let allowance = case.starts_with("allowance");
        let collection = if allowance { PERIODS } else { GRANTS };
        leak_one(&f, collection).await;
        match case {
            "deficit" => {
                f.db.collection::<bson::Document>(GRANTS).update_one(doc!{},doc!{"$set":{"reserved":Credits::ZERO,"service_limits.0.reserved":Credits::ZERO}}).await.unwrap();
            }
            "mixed_sign" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(
                        doc! {},
                        doc! {"$set":{"service_limits.0.reserved":Credits::ZERO}},
                    )
                    .await
                    .unwrap();
            }
            "negative" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$set":{"reserved":-Credits::from_whole(1)}})
                    .await
                    .unwrap();
            }
            "bad_amount" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$set":{"reserved":"malformed"}})
                    .await
                    .unwrap();
            }
            "empty_key" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$set":{"service_limits.0.key":""}})
                    .await
                    .unwrap();
            }
            "duplicate_key" => {
                let r = resource(&f, GRANTS)
                    .await
                    .get_array("service_limits")
                    .unwrap()[0]
                    .clone();
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$push":{"service_limits":r}})
                    .await
                    .unwrap();
            }
            "missing_owner" => {
                f.db.collection::<bson::Document>(GRANTS)
                    .update_one(doc! {}, doc! {"$unset":{"recipient_user_id":""}})
                    .await
                    .unwrap();
            }
            "deleted_owner" => {
                f.db.collection::<bson::Document>("users")
                    .delete_one(doc! {"_id":&f.owner})
                    .await
                    .unwrap();
            }
            "wrong_row_owner" => {
                f.db.collection::<bson::Document>(USAGE_METER)
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$set":{"billing_owner_id":"someone-else"}},
                    )
                    .await
                    .unwrap();
            }
            "missing_amount" => {
                f.db.collection::<bson::Document>(USAGE_METER)
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$unset":{"funding.grant_reservations.0.amount":""}},
                    )
                    .await
                    .unwrap();
            }
            "unknown_key" => {
                f.db.collection::<bson::Document>(USAGE_METER)
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$set":{"funding.grant_reservations.0.service_limit_key":"unknown"}},
                    )
                    .await
                    .unwrap();
            }
            "allowance_deficit" => {
                f.db.collection::<bson::Document>(PERIODS)
                    .update_one(doc! {}, doc! {"$set":{"reserved_quantity":0_i64}})
                    .await
                    .unwrap();
            }
            "allowance_missing_allocation" => {
                f.db.collection::<bson::Document>(USAGE_METER)
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$unset":{"funding.allowance_reservations.0.quantity":""}},
                    )
                    .await
                    .unwrap();
            }
            "allowance_receipt" => {
                let period = resource(&f, PERIODS).await;
                f.db.collection::<bson::Document>(USAGE_METER).update_one(doc!{"_id":&row.id},doc!{"$set":{"funding.allowance_consumptions":[{"period_id":period.get_str("_id").unwrap(),"allowance_id":"wrong","operation_id":"unknown","quantity":0_i64}]}}).await.unwrap();
            }
            _ => {
                let mut receipt = doc! {"grant_id":&gid,"operation_id":format!("{}:grant:{}",row.id,gid),"service_limit_key":"release-test","amount":Credits::ZERO};
                if case == "unknown_receipt" {
                    receipt.insert("operation_id", "unknown");
                }
                if case == "wrong_receipt_id" {
                    receipt.insert("grant_id", "wrong");
                }
                let mut receipts = vec![receipt.clone()];
                if case == "duplicate_receipt" {
                    receipts.push(receipt.clone());
                }
                if case == "both_receipts" {
                    receipt.insert("operation_id", format!("{}:grant-release:{}", row.id, gid));
                    receipts.push(receipt);
                }
                f.db.collection::<bson::Document>(USAGE_METER)
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$set":{"funding.grant_consumptions":receipts}},
                    )
                    .await
                    .unwrap();
            }
        }
        let before = resource(&f, collection).await;
        f.accounts_clean().await;
        f.accounts_clean().await;
        assert_eq!(
            resource(&f, collection).await,
            before,
            "{case}: no correction"
        );
        assert!(audits(&f).await.is_empty(), "{case}");
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn benefit_hold_repair_loses_to_real_admission_or_release() {
    for collection in [GRANTS, PERIODS] {
        for admission in [false, true] {
            let (f, ctx, _) = seeded().await;
            if collection == PERIODS {
                f.db.collection::<bson::Document>("usage_allowances")
                    .update_one(doc! {}, doc! {"$set":{"quantity":10_i64}})
                    .await
                    .unwrap();
                f.db.collection::<bson::Document>(PERIODS)
                    .update_one(doc! {}, doc! {"$set":{"total_quantity":10_i64}})
                    .await
                    .unwrap();
            }
            let live = if admission {
                None
            } else {
                Some(f.service.open(&fresh(&ctx)).await.unwrap())
            };
            leak_one(&f, collection).await;
            // Leave spendable grant capacity for the concurrent admission.
            if admission && collection == GRANTS {
                f.db.collection::<bson::Document>(GRANTS).update_one(doc!{},doc!{"$set":{"reserved":"0.0000008".parse::<Credits>().unwrap(),"service_limits.0.reserved":"0.0000008".parse::<Credits>().unwrap()}}).await.unwrap();
            }
            f.accounts_clean().await;
            let gate = Gate::new(Phase::BeforeHoldRepair);
            let mut repair = Box::pin(GATE.scope(
                gate.clone(),
                account_reconciliation::hold_pass_for_test(&f.db),
            ));
            tokio::select! {result=&mut repair=>panic!("missing barrier: {result:?}"),()=gate.entered()=>{}}
            if admission {
                f.service.open(&fresh(&ctx)).await.unwrap();
            } else {
                f.service
                    .fail(live.as_ref().unwrap(), "racing release")
                    .await
                    .unwrap();
            }
            gate.resume.notify_one();
            repair.await.unwrap();
            assert!(
                audits(&f).await.is_empty(),
                "{collection}/{admission}: racing write wins"
            );
            f.accounts_clean().await;
            assert!(audits(&f).await.is_empty());
            f.accounts_clean().await;
            assert_eq!(audits(&f).await.len(), 1);
            f.holds_clean().await;
            f.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn benefit_hold_repair_crashes_and_audit_failure_are_atomic() {
    for collection in [GRANTS, PERIODS] {
        for phase in [
            Phase::AfterHoldRepairWallet,
            Phase::AfterHoldRepairAudit,
            Phase::AfterHoldObservationDelete,
        ] {
            let (f, _, _) = seeded().await;
            leak_one(&f, collection).await;
            f.accounts_clean().await;
            let before = resource(&f, collection).await;
            let gate = Gate::new(phase);
            let mut pass = Box::pin(GATE.scope(
                gate.clone(),
                account_reconciliation::hold_pass_for_test(&f.db),
            ));
            tokio::select! {result=&mut pass=>panic!("missing barrier {phase:?}: {result:?}"),()=gate.entered()=>{}}
            assert_eq!(resource(&f, collection).await, before);
            assert!(audits(&f).await.is_empty());
            assert_eq!(
                f.db.collection::<bson::Document>(OBS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1
            );
            drop(pass);
            clear_lease(&f).await;
            f.accounts_clean().await;
            assert_eq!(audits(&f).await.len(), 1);
            assert_eq!(
                f.db.collection::<bson::Document>(OBS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            f.holds_clean().await;
            f.db.drop().await.unwrap();
        }
    }
    let (f, _, _) = seeded().await;
    leak_one(&f, GRANTS).await;
    f.accounts_clean().await;
    let before = resource(&f, GRANTS).await;
    f.db.run_command(doc!{"collMod":"audit_log","validator":{"event_type":{"$ne":"billing_grant_hold_excess_repaired"}}}).await.unwrap();
    assert!(
        account_reconciliation::hold_pass_for_test(&f.db)
            .await
            .is_err()
    );
    assert_eq!(resource(&f, GRANTS).await, before);
    assert!(audits(&f).await.is_empty());
    f.db.run_command(doc! {"collMod":"audit_log","validator":{}})
        .await
        .unwrap();
    clear_lease(&f).await;
    f.accounts_clean().await;
    assert_eq!(audits(&f).await.len(), 1);
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn benefit_hold_repair_expired_grant_finishes_and_owner_needs_no_wallet() {
    let f = Fixture::new().await;
    let owner = insert_owner(&f.db).await;
    crate::services::billing::grants::issue_grants(
        &f.db,
        crate::services::billing::grants::IssueCreditGrantInput {
            amount_credits: 1,
            target_kind: crate::models::billing_target::BillingTargetKind::SelectedUsers,
            target_user_ids: vec![owner],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            expires_at: None,
            reason: None,
            granted_by: f.owner.clone(),
            amount: None,
            service_limits: vec![],
            request_id: None,
        },
    )
    .await
    .unwrap();
    f.db.collection::<bson::Document>(GRANTS)
        .update_one(doc! {}, doc! {"$set":{"reserved":Credits::from_whole(1)}})
        .await
        .unwrap();
    f.db.collection::<bson::Document>(GRANTS)
        .update_one(
            doc! {},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    f.accounts_clean().await;
    f.accounts_clean().await;
    assert_eq!(audits(&f).await.len(), 1);
    let audit = audits(&f).await.remove(0);
    assert_eq!(
        audit.get_document("event_data").unwrap().get("wallet_id"),
        Some(&bson::Bson::Null)
    );
    assert_eq!(
        crate::services::billing::grants::expire_due_grants(&f.db, Utc::now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(resource(&f, GRANTS).await.get_str("status"), Ok("expired"));
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn benefit_hold_repair_lock_handoffs_never_authorize_correction() {
    for phase in [
        Phase::GrantLocked,
        Phase::GrantApplied,
        Phase::GrantRecorded,
        Phase::AllowanceLocked,
        Phase::AllowanceApplied,
        Phase::AllowanceRecorded,
    ] {
        let (f, ctx, _) = seeded().await;
        let opened = f.service.open(&fresh(&ctx)).await.unwrap();
        f.service.mark_forwarded(&opened).await.unwrap();
        let gate = Gate::new(phase);
        let usage = PlatformUsage {
            images: 3,
            ..PlatformUsage::single_request(0)
        };
        let mut settle =
            Box::pin(GATE.scope(gate.clone(), f.service.settle(&opened, usage, None, None)));
        tokio::select! {result=&mut settle=>panic!("missing handoff {phase:?}: {result:?}"),()=gate.entered()=>{}}
        let collection = if matches!(
            phase,
            Phase::GrantLocked | Phase::GrantApplied | Phase::GrantRecorded
        ) {
            GRANTS
        } else {
            PERIODS
        };
        leak_one(&f, collection).await;
        let before = resource(&f, collection).await;
        f.accounts_clean().await;
        f.accounts_clean().await;
        assert_eq!(resource(&f, collection).await, before, "{phase:?}");
        assert!(audits(&f).await.is_empty());
        drop(settle);
        f.recover().await;
        f.accounts_clean().await;
        f.accounts_clean().await;
        assert_eq!(audits(&f).await.len(), 1, "{phase:?}");
        f.holds_clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn benefit_hold_repair_same_pass_confirmation_waits_for_assets() {
    let (f, _, _) = seeded().await;
    leak_one(&f, GRANTS).await;
    round3::confirmed_before_repair(&f).await;
    let mut other = resource(&f, BILLING_WALLET).await;
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
    let before = resource(&f, OBS).await;
    assert!(before.get_bool("persistent").unwrap());
    immediate(&f).await;
    assert_eq!(resource(&f, OBS).await, before);
    assert!(audits(&f).await.is_empty());
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"_id":"other-wallet"},
            doc! {"$set":{"balance_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    immediate(&f).await;
    assert_eq!(audits(&f).await.len(), 1);
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn benefit_hold_repair_canonical_fields_and_bounded_rows() {
    use futures::TryStreamExt;
    let (f, ctx, _) = seeded().await;
    let request = fresh(&ctx);
    f.service.open(&request).await.unwrap();
    leak_one(&f, GRANTS).await;
    f.accounts_clean().await;
    let original = resource(&f, GRANTS).await;
    let reversed = original
        .into_iter()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<bson::Document>();
    f.db.collection::<bson::Document>(GRANTS)
        .replace_one(doc! {}, reversed)
        .await
        .unwrap();
    let row = usage_row(&f.db, &request.billing_request_id).await;
    let mut raw =
        f.db.collection::<bson::Document>(USAGE_METER)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
    let funding = raw
        .get_document("funding")
        .unwrap()
        .clone()
        .into_iter()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<bson::Document>();
    raw.insert("funding", funding);
    f.db.collection::<bson::Document>(USAGE_METER)
        .replace_one(doc! {"_id":&row.id}, raw)
        .await
        .unwrap();
    f.accounts_clean().await;
    assert_eq!(
        audits(&f).await.len(),
        1,
        "field insertion order is not new evidence"
    );
    f.db.drop().await.unwrap();

    let (f, ctx, _) = seeded().await;
    let request = fresh(&ctx);
    f.service.open(&request).await.unwrap();
    let template =
        f.db.collection::<bson::Document>(USAGE_METER)
            .find(doc! {"released":false})
            .await
            .unwrap()
            .try_next()
            .await
            .unwrap()
            .unwrap();
    let rows = (0..10_001)
        .map(|i| {
            let mut row = template.clone();
            row.insert("_id", format!("bound-{i}"));
            row.insert("transaction_id", format!("bound-{i}"));
            row
        })
        .collect::<Vec<_>>();
    f.db.collection::<bson::Document>(USAGE_METER)
        .insert_many(rows)
        .await
        .unwrap();
    f.db.collection::<bson::Document>(GRANTS).update_one(doc!{},doc!{"$set":{"reserved":Credits::from_whole(20_000),"service_limits.0.reserved":Credits::from_whole(20_000)}}).await.unwrap();
    f.db.collection::<bson::Document>(PERIODS)
        .update_one(doc! {}, doc! {"$set":{"reserved_quantity":20_000_i64}})
        .await
        .unwrap();
    let before = resource(&f, GRANTS).await;
    f.accounts_clean().await;
    f.accounts_clean().await;
    assert_eq!(resource(&f, GRANTS).await, before);
    assert!(audits(&f).await.is_empty());
    assert!(
        !f.db
            .collection::<bson::Document>(OBS)
            .find_one(doc! {"kind":"grant"})
            .await
            .unwrap()
            .unwrap()
            .get_bool("exact")
            .unwrap()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn benefit_hold_repair_pre_october_writer_can_steal_a_new_hold() {
    for collection in [GRANTS, PERIODS] {
        let (f, ctx, _) = seeded().await;
        if collection == PERIODS {
            f.db.collection::<bson::Document>("usage_allowances")
                .update_one(doc! {}, doc! {"$set":{"quantity":10_i64}})
                .await
                .unwrap();
            f.db.collection::<bson::Document>(PERIODS)
                .update_one(doc! {}, doc! {"$set":{"total_quantity":10_i64}})
                .await
                .unwrap();
        }
        leak_one(&f, collection).await;
        f.accounts_clean().await;
        f.accounts_clean().await;
        assert_eq!(audits(&f).await.len(), 1);
        f.service.open(&fresh(&ctx)).await.unwrap();
        // A pre-20-October compensating writer had no meter/claim fence. Its
        // late subtraction steals a successor's hold; this is the excluded fleet.
        let update = if collection == GRANTS {
            doc! {"$inc":{"reserved":-Credits::from_whole(1),"service_limits.0.reserved":-Credits::from_whole(1)}}
        } else {
            doc! {"$inc":{"reserved_quantity":-1_i64}}
        };
        f.db.collection::<bson::Document>(collection)
            .update_one(doc! {}, update)
            .await
            .unwrap();
        let before = resource(&f, collection).await;
        f.accounts_clean().await;
        f.accounts_clean().await;
        assert_eq!(
            resource(&f, collection).await,
            before,
            "deficit is never repaired"
        );
        assert_eq!(audits(&f).await.len(), 1);
        f.db.drop().await.unwrap();
    }
}
