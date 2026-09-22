use super::*;
use crate::errors::AppResult;
use crate::models::billing_wallet::BillingWallet;
use crate::models::service_billing::{
    BillingMetric, LanePricing, PricingSyncStatus, ServiceBilling,
};
use crate::models::usage_meter::{CredentialClass, UsageMeterRow, UsageStatus};
use crate::services::billing::BillingService;
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoUsage, LagoWallet, OwnerProvisionInput,
};
use crate::services::channel_billing_service::ChannelBilling;
use crate::services::channel_platform::{BotCredentials, OutboundReply};
use futures::TryStreamExt;
use std::sync::Arc;

struct FakeLago;
#[async_trait::async_trait]
impl LagoApi for FakeLago {
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        Ok(owner.external_customer_id.clone())
    }

    async fn ensure_subscription(&self, customer_id: &str, plan_code: &str) -> AppResult<String> {
        Ok(format!("{customer_id}:{plan_code}"))
    }

    async fn ensure_wallet(&self, customer_id: &str) -> AppResult<LagoWallet> {
        Ok(LagoWallet {
            id: format!("{customer_id}:wallet"),
            balance_credits: 100,
        })
    }

    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }

    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        Ok(events
            .iter()
            .map(|event| LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
            .collect())
    }

    async fn current_usage(
        &self,
        customer_id: &str,
        subscription_id: &str,
    ) -> AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer_id.to_string(),
            subscription_id: subscription_id.to_string(),
            raw: serde_json::json!({}),
        })
    }

    async fn wallet_balance(&self, _customer_id: &str) -> AppResult<i64> {
        Ok(100)
    }

    async fn entitlements(&self, _subscription_id: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "api-twitter".to_string(),
            raw: serde_json::json!({}),
        }])
    }
}

async fn enable_billing(state: &mut AppState, owner: &str) -> String {
    state.config.billing_enabled = true;
    state.billing = Arc::new(BillingService::new_with_lago(
        state.db.clone(),
        Arc::new(state.config.clone()),
        Arc::new(FakeLago),
    ));
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    let now = bson::DateTime::now();
    state
        .db
        .collection::<bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "owner_id": owner,
            "lago_customer_id": owner, "lago_wallet_id": "wallet", "lago_subscription_id": "plan",
            "plan_kind": "prepaid", "balance_credits": 100_i64, "reserved_credits": 0_i64,
            "pending_lago_debits": 0_i64, "has_payment_instrument": false,
            "overdraft_cap_credits": 0_i64, "suspended": false, "collection_state": "good",
            "balance_synced_at": now, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    let mut service = crate::test_utils::test_auto_connected_catalog_service();
    service.slug = "api-twitter".into();
    service.billing = Some(ServiceBilling {
        byok_pricing: Some(LanePricing {
            metric: BillingMetric::Requests,
            credits_per_unit: "2".into(),
            lago_metric_code: "platform_svc_api-twitter_byok".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: vec![],
        }),
        // A very different master-key price proves shared OAuth picks its own lane.
        platform_key_pricing: Some(LanePricing {
            metric: BillingMetric::Requests,
            credits_per_unit: "99".into(),
            lago_metric_code: "platform_svc_api-twitter_pk".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: vec![],
        }),
        ..Default::default()
    });
    let id = service.id.clone();
    state
        .db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    state.db.collection::<bson::Document>(crate::models::billing_rate_cache::COLLECTION_NAME).insert_one(doc! {
        "_id": "platform_svc_api-twitter_byok:*", "lago_metric_code": "platform_svc_api-twitter_byok",
        "credits_per_unit_micros": 2_000_000_i64, "synced_at": now,
    }).await.unwrap();
    id
}

async fn registered(state: &AppState, owner: &str, key: &str) -> ChannelBot {
    let mut bot = insert_bot(state, owner, key).await;
    bot.webhook_registered = true;
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .replace_one(doc! {"_id": &bot.id}, &bot)
        .await
        .unwrap();
    bot
}

async fn wallet(state: &AppState, owner: &str) -> BillingWallet {
    state
        .db
        .collection(crate::models::billing_wallet::COLLECTION_NAME)
        .find_one(doc! {"owner_id": owner})
        .await
        .unwrap()
        .unwrap()
}

async fn rows(state: &AppState) -> Vec<UsageMeterRow> {
    state
        .db
        .collection(crate::models::usage_meter::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn settled(state: &AppState) -> Vec<UsageMeterRow> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let rows = rows(state).await;
            if rows.iter().all(|r| r.released) {
                return rows;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("durable usage settlement")
}

async fn balance(state: &AppState, owner: &str, amount: i64) {
    state
        .db
        .collection::<bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
        .update_one(
            doc! {"owner_id": owner},
            doc! {"$set": {"balance_credits": amount}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn x_billing_incoming_duplicate_uses_owner_shared_oauth_price_and_one_ledger_entry() {
    for org in [false, true] {
        let (mut state, _, _, owner, key) = fixture().await;
        enable_billing(&mut state, &owner).await;
        if org {
            state
                .db
                .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                .update_one(doc! {"_id": &owner}, doc! {"$set": {"user_type": "org"}})
                .await
                .unwrap();
            crate::services::feature_flag_service::set_platform_override(
                &state.db,
                crate::services::feature_flag_service::BILLING_FLAG_KEY,
                &crate::services::feature_flag_service::FlagTarget::Global,
                false,
                &owner,
            )
            .await
            .unwrap();
            crate::services::feature_flag_service::set_platform_org_override(
                &state.db,
                &owner,
                crate::services::feature_flag_service::BILLING_FLAG_KEY,
                true,
                &owner,
            )
            .await
            .unwrap();
        }
        let bot = registered(&state, &owner, &key).await;
        let billing = ChannelBilling::for_bot(&state.db, &state.billing, &bot, None).unwrap();
        let (a, b) = tokio::join!(billing.received("501"), billing.received("501"));
        assert!(a.is_ok() || b.is_ok());
        for result in [a, b] {
            assert!(result.is_ok() || matches!(result, Err(AppError::Conflict(_))));
        }
        let charged = settled(&state).await;
        assert_eq!(charged.len(), 1);
        assert_eq!(charged[0].quantity, Some(1));
        assert_eq!(charged[0].billing_owner_id, owner);
        assert_eq!(
            charged[0].credential_class,
            CredentialClass::NyxidPlatformOauthApp
        );
        assert_eq!(charged[0].lago_metric_code, "platform_svc_api-twitter_byok");
        assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 2);
        assert_eq!(wallet(&state, &owner).await.reserved_credits, 0);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while state
                .db
                .collection::<bson::Document>(crate::models::billing_ledger::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap()
                != 1
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("ledger append completes after wallet settlement");
        let report = crate::services::billing::ledger::verify_chain(
            &state.db,
            &crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(report.break_info.is_none());
        balance(&state, &owner, 0).await;
        // Even deleting the catalog cannot make redelivery request new funding.
        state
            .db
            .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
            .delete_many(doc! {})
            .await
            .unwrap();
        billing.received("501").await.unwrap();
        assert_eq!(rows(&state).await.len(), 1);
    }
}

#[tokio::test]
async fn x_billing_split_reply_charges_each_success_and_releases_rejected_chunk() {
    let (mut state, adapter, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    let billing =
        ChannelBilling::for_bot(&state.db, &state.billing, &bot, Some("reply-agent")).unwrap();
    let n = std::sync::atomic::AtomicUsize::new(0);
    Mock::given(method("POST"))
        .and(path("/2/dm_conversations/10-20/messages"))
        .respond_with(move |_: &wiremock::Request| {
            if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                ResponseTemplate::new(201).set_body_json(json!({"data": {"dm_event_id": "600"}}))
            } else {
                ResponseTemplate::new(403)
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let result = adapter
        .send_reply(
            &state.http_client,
            &BotCredentials {
                token: "token",
                platform_bot_id: Some("10"),
                platform_secrets: None,
                billing: Some(&billing),
            },
            "10-20",
            &OutboundReply {
                text: Some("a".repeat(10001)),
                attachments: vec![],
                metadata: None,
                reply_to_platform_message_id: None,
            },
        )
        .await;
    assert!(result.is_err());
    let rows = settled(&state).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter_map(|r| r.quantity).sum::<i64>(), 1);
    assert!(
        rows.iter()
            .all(|r| r.api_key_id.as_deref() == Some("reply-agent"))
    );
    assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 2);
    assert_eq!(wallet(&state, &owner).await.reserved_credits, 0);
}

#[tokio::test]
async fn x_billing_insufficient_funds_and_missing_config_block_before_send() {
    let (mut state, _, server, owner, key) = fixture().await;
    let service = enable_billing(&mut state, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    balance(&state, &owner, 0).await;
    let billing = ChannelBilling::for_bot(&state.db, &state.billing, &bot, None).unwrap();
    assert!(matches!(
        billing.send(state.http_client.post(server.uri())).await,
        Err(AppError::InsufficientCredits)
    ));
    balance(&state, &owner, 100).await;
    state
        .db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(doc! {"_id": &service}, doc! {"$unset": {"billing": ""}})
        .await
        .unwrap();
    assert!(matches!(
        billing.send(state.http_client.post(server.uri())).await,
        Err(AppError::BillingNotConfigured(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(rows(&state).await.is_empty());
}

#[tokio::test]
async fn x_billing_unknown_send_keeps_hold_without_charging_or_retrying() {
    let (mut state, _, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    let billing = ChannelBilling::for_bot(&state.db, &state.billing, &bot, None).unwrap();
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        billing
            .send(state.http_client.post(server.uri()))
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE
    );
    let rows = rows(&state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, UsageStatus::Forwarded);
    assert_eq!(rows[0].quantity, None);
    assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 0);
    assert_eq!(wallet(&state, &owner).await.reserved_credits, 2);
}

#[tokio::test]
async fn x_billing_allowance_then_grant_then_wallet() {
    let (mut state, _, _, owner, key) = fixture().await;
    let service = enable_billing(&mut state, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    let now = bson::DateTime::now();
    state.db.collection::<bson::Document>(crate::models::usage_allowance::COLLECTION_NAME).insert_one(doc! {
        "_id": "allowance", "service_id": &service, "service_slug": "api-twitter", "metric": "requests",
        "quantity": 1_i64, "recurrence": "daily", "target_kind": "all_users", "is_active": true,
        "created_by": "admin", "created_at": now, "updated_at": now,
    }).await.unwrap();
    state.db.collection::<bson::Document>(crate::models::credit_grant::COLLECTION_NAME).insert_one(doc! {
        "_id": "grant", "batch_id": "batch", "recipient_user_id": &owner, "target_kind": "selected_users",
        "amount_credits": 2_i64, "amount_micros": 2_000_000_i64, "remaining_micros": 2_000_000_i64, "reserved_micros": 0_i64,
        "scope": {"all_services": true}, "granted_by": "admin", "status": "active", "issued_ledgered_at": now,
        "created_at": now, "updated_at": now,
    }).await.unwrap();
    let billing = ChannelBilling::for_bot(&state.db, &state.billing, &bot, None).unwrap();
    for event in ["701", "702", "703"] {
        billing.received(event).await.unwrap();
        settled(&state).await;
    }
    let rows = rows(&state).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 2);
    let grant = state
        .db
        .collection::<bson::Document>(crate::models::credit_grant::COLLECTION_NAME)
        .find_one(doc! {"_id": "grant"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.get_i64("remaining_micros").unwrap(), 0);
}

#[tokio::test]
async fn x_billing_no_agent_route_still_accounts_for_received_event() {
    let (mut state, adapter, _, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    let inbound = adapter.parse_inbound(&json!({"data": {"event_type": "dm.received", "filter": {"user_id": "10"},
        "payload": {"direct_message_events": [{"id": "801", "type": "message_create", "message_create": {
            "sender_id": "20", "target": {"recipient_id": "10"}, "message_data": {"text": "private content"}
        }}]}}}).to_string().into_bytes()).await.unwrap();
    crate::services::channel_inbound_service::process_inbound_messages(
        crate::services::channel_inbound_service::InboundDeps::from(&state),
        &bot,
        &adapter,
        &inbound,
    )
    .await
    .unwrap();
    let rows = settled(&state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 2);
    assert!(!format!("{rows:?}").contains("private content"));
}

#[tokio::test]
async fn x_billing_subscription_admission_checks_funding_before_x_setup() {
    let (mut state, adapter, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    super::webhooks::credentials(&state, &adapter, &owner).await;
    let bot = insert_bot(&state, &owner, &key).await;
    balance(&state, &owner, 0).await;
    let result = crate::services::channel_connection_webhook_service::configure(
        &state.db,
        &state.billing,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &bot,
        "https://nyx.example",
    )
    .await;
    assert!(matches!(result, Err(AppError::InsufficientCredits)));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .status,
        "failed"
    );
}

#[tokio::test]
async fn x_billing_setup_lease_conflict_records_a_recoverable_channel_failure() {
    use crate::services::coordination_service::{LeaseStore, cluster_lease_runtime};
    let (mut state, adapter, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    let bot = insert_bot(&state, &owner, &key).await;
    let lease = cluster_lease_runtime()
        .acquire(&state.db, "channel-webhook:x")
        .await
        .unwrap()
        .unwrap();
    let result = crate::services::channel_connection_webhook_service::configure(
        &state.db,
        &state.billing,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &bot,
        "https://nyx.example",
    )
    .await;
    LeaseStore::release(&state.db, &lease).await.unwrap();
    assert!(matches!(result, Err(AppError::Conflict(_))));
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(current.status, "failed");
    assert!(!current.webhook_registered);
    assert!(current.error.as_deref().unwrap().contains("Verify"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn x_billing_failure_unsubscribes_and_cleanup_rechecks_recovery() {
    let (mut state, adapter, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    super::webhooks::credentials(&state, &adapter, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    super::webhooks::provider_setup(&server, &bot).await;
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/200"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(
                if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    503
                } else {
                    204
                },
            )
        })
        .expect(2)
        .mount(&server)
        .await;
    balance(&state, &owner, 0).await;
    let inbound = adapter.parse_inbound(&json!({"data": {"event_type": "dm.received", "filter": {"user_id": "10"},
        "payload": {"direct_message_events": [{"id": "901", "type": "message_create", "message_create": {
            "sender_id": "20", "target": {"recipient_id": "10"}, "message_data": {"text": "private content"}
        }}]}}}).to_string().into_bytes()).await.unwrap();
    assert!(
        crate::services::channel_inbound_service::process_inbound_messages(
            crate::services::channel_inbound_service::InboundDeps::from(&state),
            &bot,
            &adapter,
            &inbound
        )
        .await
        .is_err()
    );
    let failed = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert!(
        failed.webhook_registered,
        "retain cleanup retry marker on provider failure"
    );
    crate::services::channel_connection_webhook_service::remove_stopped(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &failed,
    )
    .await
    .unwrap();
    assert!(
        !channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .webhook_registered
    );
    balance(&state, &owner, 100).await;
    crate::services::channel_connection_webhook_service::configure(
        &state.db,
        &state.billing,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &failed,
        "https://nyx.example",
    )
    .await
    .unwrap();
    // A cleanup task holding the old failed snapshot cannot remove the recovered subscription.
    crate::services::channel_connection_webhook_service::remove_stopped(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &failed,
    )
    .await
    .unwrap();
    let active = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(active.status, "active");
    assert!(active.webhook_registered);
    assert_eq!(wallet(&state, &owner).await.reserved_credits, 0);
}

#[tokio::test]
async fn x_billing_polling_is_blocked_on_create_sweep_and_reconnect() {
    let (mut state, adapter, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    identity(&server, "10").await;
    Mock::given(path("/2/dm_events"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    assert!(matches!(
        create(&state, &adapter, &owner, &key).await,
        Err(AppError::BillingNotConfigured(_))
    ));
    let mut bot = insert_bot(&state, &owner, &key).await;
    channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
        .await
        .unwrap();
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .status,
        "failed"
    );
    bot = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    bot.poll_cursor = None;
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .replace_one(doc! {"_id": &bot.id}, &bot)
        .await
        .unwrap();
    channel_bot_service::reconnect_bot(
        &state.db,
        &state.billing,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &bot,
        &key,
    )
    .await
    .unwrap();
    assert!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .poll_cursor
            .is_none()
    );
}

#[tokio::test]
async fn x_billing_requires_provider_but_allows_explicit_zero_price() {
    let (mut state, _, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    let no_provider = BillingService::new(state.db.clone(), Arc::new(state.config.clone()));
    let billing = ChannelBilling::for_bot(&state.db, &no_provider, &bot, None).unwrap();
    assert!(matches!(
        billing.send(state.http_client.post(server.uri())).await,
        Err(AppError::BillingNotConfigured(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
    balance(&state, &owner, 0).await;
    state
        .db
        .collection::<bson::Document>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .update_one(doc! {}, doc! {"$set": {"credits_per_unit_micros": 0_i64}})
        .await
        .unwrap();
    let billing = ChannelBilling::for_bot(&state.db, &state.billing, &bot, None).unwrap();
    billing.received("1001").await.unwrap();
    let rows = settled(&state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].quantity, Some(1));
    assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 0);
    assert_eq!(wallet(&state, &owner).await.reserved_credits, 0);
}

#[tokio::test]
async fn x_billing_account_verification_reserves_before_lookup() {
    let (mut state, adapter, server, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    let bot = insert_bot(&state, &owner, &key).await;
    identity(&server, "10").await;
    let billing = ChannelBilling::for_bot(&state.db, &state.billing, &bot, None).unwrap();
    let credentials = BotCredentials {
        token: "token",
        platform_bot_id: None,
        platform_secrets: None,
        billing: Some(&billing),
    };
    adapter
        .verify_bot_token(&state.http_client, &credentials)
        .await
        .unwrap();
    let rows = settled(&state).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].billing_request_id.starts_with("x-account-verify:"));
    assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 2);
    balance(&state, &owner, 0).await;
    assert!(matches!(
        adapter
            .verify_bot_token(&state.http_client, &credentials)
            .await,
        Err(AppError::InsufficientCredits)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn x_billing_rollout_disabled_meters_without_wallet_charge() {
    let (mut state, _, _, owner, key) = fixture().await;
    enable_billing(&mut state, &owner).await;
    let bot = registered(&state, &owner, &key).await;
    crate::services::feature_flag_service::set_platform_override(
        &state.db,
        crate::services::feature_flag_service::BILLING_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        false,
        &owner,
    )
    .await
    .unwrap();
    balance(&state, &owner, 0).await;
    let billing = ChannelBilling::for_bot(&state.db, &state.billing, &bot, None).unwrap();
    billing.received("1101").await.unwrap();
    let rows = settled(&state).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].wallet_id.is_none());
    assert_eq!(wallet(&state, &owner).await.pending_lago_debits, 0);
}
