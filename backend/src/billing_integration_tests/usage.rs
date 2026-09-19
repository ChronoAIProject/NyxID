use super::*;
use crate::handlers::billing::{self, BillingUsageResponse, UsageQuery};
use crate::models::usage_meter::{BillingLayer, UsageFunding};
use crate::test_utils::test_auth_user;
use axum::extract::{Query, State};

fn meter(owner: &str, quantity: i64) -> UsageMeterRow {
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    UsageMeterRow {
        id: id.clone(),
        transaction_id: id.clone(),
        billing_request_id: id,
        layer: BillingLayer::Platform,
        flush_seq: None,
        billing_owner_id: owner.to_string(),
        wallet_id: Some("wallet".into()),
        actor_user_id: owner.to_string(),
        api_key_id: None,
        service_id: Some("service".into()),
        service_slug: Some("llm-test".into()),
        metric: BillingMetric::Tokens,
        lago_metric_code: "platform_tokens".into(),
        credential_class: CredentialClass::NyxidManagedMaster,
        model: Some("test-model".into()),
        token_breakdown: None,
        reserved_credits: 0,
        funding: None,
        quantity: Some(quantity),
        pending_resale_quantity: None,
        pending_platform_usage: None,
        status: UsageStatus::Finalized,
        forwarded: true,
        released: false,
        lago_acked: false,
        attempt: 0,
        settlement_attempts: 0,
        settlement_next_retry_at: None,
        created_at: now,
        updated_at: now,
        finalized_at: Some(now),
        expires_at: None,
        last_error: None,
    }
}

async fn read_usage(state: &crate::AppState, actor: &str) -> BillingUsageResponse {
    billing::get_usage(
        State(state.clone()),
        test_auth_user(actor),
        Query(UsageQuery { period: None }),
    )
    .await
    .expect("usage read")
    .0
}

async fn rate(db: &mongodb::Database, model: Option<&str>, micros: i64) {
    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .insert_one(BillingRateCache {
            id: BillingRateCache::cache_id("platform_tokens", model),
            lago_metric_code: "platform_tokens".into(),
            model: model.map(str::to_string),
            credits_per_unit_micros: micros,
            credits_per_unit_pico: None,
            synced_at: Utc::now(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn free_usage_is_visible_with_zero_cost_and_status_filters() {
    let Some(db) = connect_test_database("billing_usage_free").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    rate(&db, None, 50).await;
    for (status, forwarded, quantity) in [
        (UsageStatus::Finalized, true, Some(7)),
        (UsageStatus::DeadLetter, true, Some(3)),
        (UsageStatus::DeadLetter, false, Some(100)),
        (UsageStatus::Forwarded, true, Some(100)),
        (UsageStatus::Finalized, true, None),
    ] {
        let mut row = meter(&owner, 1);
        row.wallet_id = None;
        row.status = status;
        row.forwarded = forwarded;
        row.quantity = quantity;
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(row)
            .await
            .unwrap();
    }
    let result = read_usage(&state, &owner).await;
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert!(!row.billable);
    assert_eq!(row.quantity, 10);
    assert_eq!(row.events, 2);
    assert_eq!(row.estimated_credits_micros, Some(0));
    assert_eq!(row.wallet_credits_micros, Some(0));
    assert_eq!(row.grant_credits_micros, Some(0));
    assert_eq!(row.allowance_credits_micros, Some(0));
    assert_eq!(row.allowance_quantity, 0);
    assert_eq!(result.totals.estimated_credits_micros, Some(0));
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! {"lago_acked": true})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_funding_costs_survive_retries_repricing_and_missing_rates() {
    use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
    use crate::models::credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant, CreditGrantStatus};
    use crate::models::usage_allowance::{
        AllowanceRecurrence, COLLECTION_NAME as ALLOWANCES, UsageAllowance,
    };
    use crate::services::billing::funding::settle_usage_funding;
    let Some(db) = connect_test_database("billing_usage_exact_funding").await else {
        return;
    };
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    rate(&db, Some("test-model"), 1).await;
    // Fractional wallet cost must never be replaced by the whole-credit debit.
    for (name, allowance_units, grant_micros, wallet_micros) in [
        ("grant", 0, 2440, 0),
        ("allowance", 2440, 0, 0),
        ("mixed", 1200, 1000, 240),
        ("wallet", 0, 0, 2440),
    ] {
        let owner = insert_owner(&db).await;
        let now = Utc::now();
        if allowance_units > 0 {
            db.collection::<UsageAllowance>(ALLOWANCES)
                .insert_one(UsageAllowance {
                    id: Uuid::new_v4().to_string(),
                    service_id: "service".into(),
                    service_slug: "llm-test".into(),
                    metric: BillingMetric::Tokens,
                    quantity: allowance_units,
                    recurrence: AllowanceRecurrence::OneTime,
                    target_kind: BillingTargetKind::SelectedUsers,
                    target_user_ids: vec![owner.clone()],
                    is_active: true,
                    created_by: owner.clone(),
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
        }
        let grant_id = Uuid::new_v4().to_string();
        if grant_micros > 0 {
            db.collection::<CreditGrant>(GRANTS)
                .insert_one(CreditGrant {
                    id: grant_id.clone(),
                    batch_id: Uuid::new_v4().to_string(),
                    schedule_origin: None,
                    recipient_user_id: owner.clone(),
                    target_kind: BillingTargetKind::SelectedUsers,
                    amount_credits: 1,
                    amount_micros: grant_micros,
                    remaining_micros: grant_micros,
                    reserved_micros: 0,
                    scope: BillingServiceScope {
                        all_services: true,
                        service_ids: vec![],
                        service_slugs: vec![],
                    },
                    expires_at: None,
                    reason: None,
                    granted_by: owner.clone(),
                    status: CreditGrantStatus::Active,
                    issued_ledgered_at: Some(now),
                    terminal_ledgered_at: None,
                    terminal_amount_micros: 0,
                    active_settlement: None,
                    created_at: now,
                    updated_at: now,
                    consumed_at: None,
                    expired_at: None,
                    revoked_at: None,
                })
                .await
                .unwrap();
        }
        let mut row = meter(&owner, 2440);
        row.funding = Some(UsageFunding {
            credits_per_unit_micros: 90,
            credits_per_unit_pico: None,
            ..Default::default()
        });
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let settlement = settle_usage_funding(&db, &row).await.unwrap();
        assert_eq!(
            settlement.wallet_charge_credits,
            i64::from(wallet_micros > 0),
            "{name}"
        );
        assert_eq!(
            settlement.lago_billable_quantity_micros,
            wallet_micros * 1_000_000,
            "{name}"
        );
        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        let funding = saved.funding.as_ref().unwrap();
        assert_eq!(funding.total_charge_micros, Some(2440), "{name}");
        assert_eq!(funding.allowance_funded_quantity, Some(allowance_units));
        assert_eq!(funding.allowance_funded_micros, Some(allowance_units));
        assert_eq!(funding.grant_funded_micros, Some(grant_micros));
        assert_eq!(funding.wallet_funded_micros, Some(wallet_micros));
        db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .update_many(
                doc! {},
                doc! { "$set": { "credits_per_unit_micros": 7_i64 } },
            )
            .await
            .unwrap();
        // Both the already-settled path and the stale-caller crash-recovery path
        // return the original debit/quantity without consuming benefits twice.
        assert_eq!(settle_usage_funding(&db, &saved).await.unwrap(), settlement);
        assert_eq!(settle_usage_funding(&db, &row).await.unwrap(), settlement);
        let after_retry = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_retry.funding, saved.funding);
        let result = read_usage(&state, &owner).await;
        assert_eq!(result.rows[0].estimated_credits_micros, Some(2440));
        assert_eq!(result.rows[0].wallet_credits_micros, Some(wallet_micros));
        assert_eq!(result.rows[0].grant_credits_micros, Some(grant_micros));
        assert_eq!(
            result.rows[0].allowance_credits_micros,
            Some(allowance_units)
        );
        assert_eq!(result.rows[0].allowance_quantity, allowance_units);
        assert_eq!(result.totals.estimated_credits_micros, Some(2440));
        db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .update_many(
                doc! {},
                doc! { "$set": { "credits_per_unit_micros": 1_i64 } },
            )
            .await
            .unwrap();
    }
    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .delete_many(doc! {})
        .await
        .unwrap();
    let owners: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! {})
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    for owner in owners {
        let result = read_usage(&state, &owner.id).await;
        assert_eq!(result.rows[0].estimated_credits_micros, Some(2440));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn historical_funding_uses_model_rate_and_sums_with_exact_and_free_rows() {
    use crate::models::usage_meter::{AllowanceConsumptionAllocation, GrantConsumptionAllocation};
    let Some(db) = connect_test_database("billing_usage_historical").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    rate(&db, None, 90).await;
    rate(&db, Some("test-model"), 2).await;
    let mut old = meter(&owner, 100);
    old.funding = Some(UsageFunding {
        settled: true,
        wallet_charge_credits: Some(1),
        allowance_consumptions: vec![AllowanceConsumptionAllocation {
            operation_id: "a".into(),
            allowance_id: "a".into(),
            period_id: "a".into(),
            quantity: 20,
        }],
        grant_consumptions: vec![GrantConsumptionAllocation {
            operation_id: "g".into(),
            grant_id: "g".into(),
            amount_micros: 50,
        }],
        ..Default::default()
    });
    let legacy = meter(&owner, 10);
    let legacy_settlement = crate::services::billing::funding::settle_usage_funding(&db, &legacy)
        .await
        .unwrap();
    assert_eq!(legacy_settlement.wallet_charge_credits, 1);
    assert_eq!(legacy_settlement.lago_billable_quantity_micros, 10_000_000);
    let mut exact = meter(&owner, 30);
    exact.funding = Some(UsageFunding {
        settled: true,
        total_charge_micros: Some(30),
        wallet_funded_micros: Some(20),
        grant_funded_micros: Some(10),
        allowance_funded_micros: Some(0),
        allowance_funded_quantity: Some(0),
        ..Default::default()
    });
    let mut free = meter(&owner, 1000);
    free.wallet_id = None;
    let mut unknown = meter(&owner, 99);
    unknown.lago_metric_code = "missing".into();
    let mut mixed_exact = meter(&owner, 30);
    mixed_exact.lago_metric_code = "missing_mixed".into();
    mixed_exact.funding = exact.funding.clone();
    let mut mixed_historical = meter(&owner, 100);
    mixed_historical.lago_metric_code = mixed_exact.lago_metric_code.clone();
    mixed_historical.funding = old.funding.clone();
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_many([
            old,
            legacy,
            exact,
            free,
            unknown,
            mixed_exact,
            mixed_historical,
        ])
        .await
        .unwrap();
    let result = read_usage(&state, &owner).await;
    let charged = result
        .rows
        .iter()
        .find(|row| row.lago_metric_code == "platform_tokens" && row.billable)
        .unwrap();
    assert_eq!(charged.estimated_credits_micros, Some(250));
    assert_eq!(charged.wallet_credits_micros, Some(150));
    assert_eq!(charged.grant_credits_micros, Some(60));
    assert_eq!(charged.allowance_credits_micros, Some(40));
    assert_eq!(charged.allowance_quantity, 20);
    let missing = result
        .rows
        .iter()
        .find(|row| row.lago_metric_code == "missing")
        .unwrap();
    assert_eq!(missing.estimated_credits_micros, None);
    assert_eq!(missing.wallet_credits_micros, None);
    let mixed = result
        .rows
        .iter()
        .find(|row| row.lago_metric_code == "missing_mixed")
        .unwrap();
    assert_eq!(mixed.events, 2);
    assert_eq!(mixed.estimated_credits_micros, None);
    assert_eq!(mixed.wallet_credits_micros, None);
    assert_eq!(mixed.allowance_credits_micros, None);
    assert_eq!(mixed.grant_credits_micros, Some(60));
    assert_eq!(mixed.allowance_quantity, 20);
    assert_eq!(result.totals.estimated_credits_micros, Some(250));
    assert_eq!(
        result.totals.estimated_credits_micros,
        Some(
            result
                .rows
                .iter()
                .filter_map(|r| r.estimated_credits_micros)
                .sum()
        )
    );
    assert_eq!(result.totals.wallet_credits_micros, Some(150));
    assert_eq!(result.totals.grant_credits_micros, Some(120));
    assert_eq!(result.totals.allowance_credits_micros, Some(40));
    assert_eq!(result.totals.allowance_quantity, 40);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_platform_key_request_charges_personal_wallet_and_personal_rollout() {
    Box::pin(org_credential_request(true)).await;
}

#[tokio::test]
async fn org_byok_request_keeps_org_wallet_and_org_rollout() {
    Box::pin(org_credential_request(false)).await;
}

async fn org_credential_request(platform_key: bool) {
    use crate::models::downstream_service::{PlatformKeyAudience, PlatformKeyConfig};
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    use crate::services::feature_flag_service as flags;
    use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
    let Some(db) = connect_test_database("billing_org_credential_payer").await else {
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let actor = insert_owner(&db).await;
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            &actor,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    flags::set_platform_override(
        &db,
        flags::BILLING_FLAG_KEY,
        &flags::FlagTarget::Global,
        false,
        &actor,
    )
    .await
    .unwrap();
    if platform_key {
        flags::set_platform_override(
            &db,
            flags::BILLING_FLAG_KEY,
            &flags::FlagTarget::User(actor.clone()),
            true,
            &actor,
        )
        .await
        .unwrap();
    } else {
        flags::set_platform_org_override(&db, &org, flags::BILLING_FLAG_KEY, true, &actor)
            .await
            .unwrap();
    }
    assert_eq!(
        flags::billing_rollout_enabled(&db, &org, &actor)
            .await
            .unwrap(),
        !platform_key
    );
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let (downstream_url, downstream) = start_billing_downstream().await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "llm-org-payer".into();
    catalog.base_url = downstream_url.clone();
    catalog.auth_method = "bearer".into();
    catalog.credential_encrypted = state
        .encryption_keys
        .encrypt(b"platform-test-key")
        .await
        .unwrap();
    catalog.platform_key = Some(PlatformKeyConfig {
        enabled: true,
        audience: PlatformKeyAudience::Restricted,
        allowed_owner_ids: vec![org.clone()],
    });
    catalog.billing = Some(ServiceBilling {
        platform_billable: true,
        platform_metric: Some(BillingMetric::Tokens),
        ..Default::default()
    });
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = insert_route_service(
        &db,
        &org,
        &catalog.slug,
        &downstream_url,
        Some(&catalog.id),
        None,
    )
    .await;
    let mut update = doc! { "credential_binding": if platform_key { "platform" } else { "user" }, "auth_method": "bearer" };
    if !platform_key {
        let key = create_api_key(
            &db,
            &state.encryption_keys,
            &org,
            CreateApiKeyParams {
                label: "Org key",
                credential_type: "bearer",
                credential: "org-test-key",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        update.insert("api_key_id", key.id);
    }
    db.collection::<UserService>(USER_SERVICES)
        .update_one(doc! {"_id": &connection.id}, doc! {"$set": update})
        .await
        .unwrap();
    // There is no personal connection: the request must resolve this org row.
    let token = route_access_token(&state, &actor);
    let (_, private) = crate::routes::build_router();
    let app = private.with_state(state.clone());
    let request = route_request(
        Method::POST,
        &format!("/api/v1/proxy/s/{}/chat/completions", catalog.slug),
        &token,
        Body::from(r#"{"model":"test-model","messages":[]}"#),
    );
    call_mounted_route(&app, request).await;
    assert_route_settled(&db, &catalog.slug, BillingMetric::Tokens).await;
    let row = usage_row_for_service(&db, &catalog.slug).await;
    let (payer, other) = if platform_key {
        (&actor, &org)
    } else {
        (&org, &actor)
    };
    assert_eq!(&row.billing_owner_id, payer);
    assert_eq!(row.actor_user_id, actor);
    assert_eq!(
        row.credential_class,
        if platform_key {
            CredentialClass::NyxidManagedMaster
        } else {
            CredentialClass::UserOwned
        }
    );
    assert_eq!(row.quantity, Some(5));
    assert!(row.wallet_id.is_some());
    assert_eq!(wallet(&db, payer).await.pending_lago_debits, 5);
    assert!(state.billing.get_wallet(other).await.unwrap().is_none());
    let personal_usage = read_usage(&state, &actor).await;
    assert_eq!(personal_usage.rows.len(), usize::from(platform_key));
    if platform_key {
        assert_eq!(
            personal_usage.totals.estimated_credits_micros,
            Some(5_000_000)
        );
    }
    downstream.abort();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn execution_owner_preserves_org_acl_for_non_master_credentials() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    use crate::services::billing::owner_resolver::{BillingOwnerResolver, PaysFrom};
    let Some(db) = connect_test_database("billing_execution_owner_acl").await else {
        return;
    };
    let actor = insert_owner(&db).await;
    let outsider = insert_owner(&db).await;
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            &actor,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    let resolver = BillingOwnerResolver::new(db.clone());
    let personal = resolver
        .resolve_for_execution(&actor, &org, CredentialClass::NyxidManagedMaster)
        .await
        .unwrap();
    assert_eq!(personal.owner_id, actor);
    assert_eq!(personal.pays, PaysFrom::Personal);
    for class in [
        CredentialClass::UserOwned,
        CredentialClass::AgentOverrideUserOwned,
        CredentialClass::NodeManaged,
        CredentialClass::NyxidPlatformOauthApp,
        CredentialClass::NoAuth,
    ] {
        let resolved = resolver
            .resolve_for_execution(&actor, &org, class)
            .await
            .unwrap();
        assert_eq!(resolved.owner_id, org);
        assert_eq!(
            resolved.pays,
            PaysFrom::OrgWallet {
                org_id: org.clone()
            }
        );
        assert!(matches!(
            resolver.resolve_for_execution(&outsider, &org, class).await,
            Err(AppError::Forbidden(_))
        ));
        assert_eq!(
            resolver
                .resolve_for_execution(&actor, &actor, class)
                .await
                .unwrap(),
            personal
        );
    }
    assert!(matches!(
        resolver.resolve_for_resource(&outsider, &org).await,
        Err(AppError::Forbidden(_))
    ));
    db.drop().await.unwrap();
}
