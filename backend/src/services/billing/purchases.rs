use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Serialize;

use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_topup_session::{
            BillingTopUpSession, BillingTopUpStatus, COLLECTION_NAME, StripePurchase,
        },
        billing_wallet::{BillingWallet, PlanKind},
        credits::Credits,
    },
};

use super::{
    lago_client::{
        LagoApi,
        external_purchases::{CreditPeriodConfig, ExternalPayment},
    },
    provisioning::{self, TopUpCheckout},
    stripe::{Stripe, hosted_url},
};

pub mod reversals;

const MAX_STRIPE_USD_MINOR: i64 = 99_999_999;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PurchaseOffer {
    pub unit_amount_minor: i64,
    pub maximum_credits: i64,
    pub minimum_credits: i64,
}

pub(super) fn offer(stripe: &Stripe) -> PurchaseOffer {
    PurchaseOffer {
        unit_amount_minor: stripe.settings.unit_amount_minor,
        minimum_credits: 999 / stripe.settings.unit_amount_minor + 1,
        maximum_credits: (stripe.settings.maximum_subtotal_minor
            / stripe.settings.unit_amount_minor)
            .min(10_000_000),
    }
}

fn sessions(db: &Database) -> mongodb::Collection<BillingTopUpSession> {
    db.collection(COLLECTION_NAME)
}

pub(super) async fn verify_wallet(
    lago: &dyn LagoApi,
    stripe: &Stripe,
    wallet: &BillingWallet,
) -> AppResult<String> {
    verify_wallet_config(lago, &stripe.credit_period_config(), wallet).await
}

async fn verify_wallet_config(
    lago: &dyn LagoApi,
    config: &CreditPeriodConfig,
    wallet: &BillingWallet,
) -> AppResult<String> {
    let enrollment = wallet.stripe_enrollment.as_ref().ok_or_else(unavailable)?;
    if wallet.is_suspended()
        || wallet.plan_kind != PlanKind::Prepaid
        || wallet.has_payment_instrument
        || wallet.overdraft_cap_credits != Credits::ZERO
        || wallet.lago_wallet_id.as_deref() != Some(enrollment.lago_wallet_id.as_str())
        || wallet.lago_subscription_id.as_deref() != Some(enrollment.lago_subscription_id.as_str())
        || enrollment.stripe_account_id != config.account_id
        || enrollment.livemode != config.livemode
    {
        return Err(AppError::Conflict(
            "Lago Stripe-funded period settings changed".into(),
        ));
    }
    lago.verify_external_credit_wallet(
        wallet,
        &enrollment.stripe_account_id,
        config.secret.as_str(),
    )
    .await?;
    let tax = lago
        .tax_config(&wallet.lago_customer_id, &enrollment.lago_subscription_id)
        .await?;
    // All charges use the same non-collecting provider period. A zero plan base
    // keeps credit consumption separate from charges that were never admitted.
    if tax
        .plan
        .get("amount_cents")
        .and_then(serde_json::Value::as_i64)
        != Some(0)
        || tax
            .plan
            .get("amount_currency")
            .and_then(serde_json::Value::as_str)
            != Some("USD")
        || !tax
            .plan
            .get("fixed_charges")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty)
    {
        return Err(AppError::Conflict(
            "Lago Stripe-funded period settings changed".into(),
        ));
    }
    if !enrollment.plan_code.is_empty()
        && tax.plan.get("code").and_then(serde_json::Value::as_str)
            != Some(enrollment.plan_code.as_str())
    {
        return Err(AppError::Conflict(
            "Lago Stripe-funded period settings changed".into(),
        ));
    }
    let profile =
        super::taxes::resolve_config(&wallet.owner_id, &enrollment.lago_subscription_id, &tax)?;
    super::taxes::require_zero_usage_tax(&profile)
        .map_err(|_| AppError::Conflict("Lago Stripe-funded usage has tax configured".into()))?;
    Ok(enrollment.stripe_customer_id.clone())
}

pub(super) async fn enroll_wallet(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    mut wallet: BillingWallet,
    customer_id: &str,
    actor: &crate::services::audit_service::AuditActor,
    audit_key: &[u8],
) -> AppResult<()> {
    super::exact_migration::require_ready(db).await?;
    if customer_id.len() > 128
        || !customer_id.starts_with("cus_")
        || !customer_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(AppError::BadRequest("Stripe customer ID is invalid".into()));
    }
    if let Some(enrollment) = &wallet.stripe_enrollment {
        if enrollment.stripe_customer_id == customer_id
            && enrollment.stripe_account_id == stripe.settings.account_id
            && enrollment.livemode == stripe.settings.mode.livemode()
        {
            verify_and_publish_wallet(db, lago, stripe, &wallet).await?;
            return Ok(());
        }
        return Err(AppError::Conflict(
            "A Stripe billing enrollment already owns this wallet".into(),
        ));
    }
    let enrollment = crate::models::billing_wallet::StripeWalletEnrollment {
        stripe_account_id: stripe.settings.account_id.clone(),
        stripe_customer_id: customer_id.into(),
        livemode: stripe.settings.mode.livemode(),
        lago_wallet_id: wallet.lago_wallet_id.clone().ok_or_else(unavailable)?,
        lago_subscription_id: wallet
            .lago_subscription_id
            .clone()
            .ok_or_else(unavailable)?,
        plan_code: String::new(),
        activated_at: None,
        usage_verified_until: None,
        reversal_holds: vec![],
    };
    wallet.stripe_enrollment = Some(enrollment.clone());
    verify_wallet(lago, stripe, &wallet).await?;
    stripe
        .verify_customer(&wallet.owner_id, customer_id)
        .await?;
    if !lago
        .wallet_transactions(&enrollment.lago_wallet_id)
        .await?
        .is_empty()
        || lago.wallet_balance_exact(&wallet.lago_customer_id).await? != Credits::ZERO
        || super::webhook::provider_effective_balance(lago, &wallet).await? != Credits::ZERO
    {
        return Err(existing_activity());
    }
    let now = Utc::now();
    let mut enrollment = enrollment;
    let provider_plan = lago
        .tax_config(&wallet.lago_customer_id, &enrollment.lago_subscription_id)
        .await?;
    enrollment.plan_code = provider_plan
        .plan
        .get("code")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(unavailable)?
        .into();
    let profile = super::taxes::resolve_config(
        &wallet.owner_id,
        &enrollment.lago_subscription_id,
        &provider_plan,
    )?;
    super::taxes::require_zero_usage_tax(&profile)?;
    enrollment.activated_at = Some(now);
    enrollment.usage_verified_until = Some(now + Duration::minutes(10));
    let fields = bson::to_document(&enrollment).map_err(|_| unavailable())?;
    let mut txn = db.client().start_session().await?;
    txn.start_transaction().await?;
    let result = async {
        let rows = db.collection::<crate::models::usage_meter::UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        if sessions(db).find_one(doc! {"owner_id": &wallet.owner_id}).session(&mut txn).await?.is_some()
            || rows.find_one(doc! {"billing_owner_id": &wallet.owner_id}).session(&mut txn).await?.is_some()
        {
            return Err(existing_activity());
        }
        let changed = db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .update_one(doc! {
                "_id": &wallet.id, "stripe_enrollment": null,
                "snapshot_generation": wallet.snapshot_generation, "tax_generation": wallet.tax_generation,
                "lago_wallet_id": &enrollment.lago_wallet_id, "lago_subscription_id": &enrollment.lago_subscription_id,
                "balance_credits": Credits::ZERO, "reserved_credits": Credits::ZERO, "pending_lago_debits": Credits::ZERO,
            }, doc! {"$set": {"stripe_enrollment": fields}})
            .session(&mut txn).await?;
        if changed.modified_count != 1 { return Err(existing_activity()); }
        crate::services::audit_service::log_actor_event_in_session(
            db, &mut txn, audit_key, actor, "billing_stripe_wallet_enrolled",
            serde_json::json!({"wallet_id": wallet.id, "owner_id": wallet.owner_id, "livemode": enrollment.livemode}),
        ).await?;
        Ok(())
    }.await;
    match result {
        Ok(()) => {
            txn.commit_transaction().await?;
            Ok(())
        }
        Err(error) => {
            txn.abort_transaction().await?;
            Err(error)
        }
    }
}

fn existing_activity() -> AppError {
    AppError::Conflict(
        "Stripe billing requires a new wallet with no purchase or usage history".into(),
    )
}

async fn verify_and_publish_wallet(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    wallet: &BillingWallet,
) -> AppResult<String> {
    let enrollment = wallet.stripe_enrollment.as_ref().ok_or_else(unavailable)?;
    let wallets = db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME);
    let filter = doc! {
        "_id": &wallet.id,
        "tax_generation": {"$in": [bson::to_bson(&wallet.tax_generation).map_err(|_| unavailable())?, bson::Bson::Null]},
        "stripe_enrollment.stripe_account_id": &enrollment.stripe_account_id,
        "stripe_enrollment.stripe_customer_id": &enrollment.stripe_customer_id,
        "lago_subscription_id": &enrollment.lago_subscription_id,
        "lago_wallet_id": &enrollment.lago_wallet_id,
    };
    // The first verification is only for a dedicated, empty prepaid wallet.
    // Never mix established balances or historical checkouts into this period.
    let result = async {
        if enrollment.activated_at.is_none()
            && (wallet.balance_credits != Credits::ZERO
                || wallet.reserved_credits != Credits::ZERO
                || wallet.pending_lago_debits != Credits::ZERO
                || wallet.pending_topup_expiry_credits != Credits::ZERO
                || sessions(db)
                    .find_one(doc! {"owner_id": &wallet.owner_id})
                    .await?
                    .is_some()
                || !lago
                    .wallet_transactions(&enrollment.lago_wallet_id)
                    .await?
                    .is_empty())
        {
            return Err(AppError::Conflict(
                "Stripe billing requires a newly provisioned wallet and period".into(),
            ));
        }
        let customer = verify_wallet(lago, stripe, wallet).await?;
        if enrollment.activated_at.is_none() {
            stripe.verify_customer(&wallet.owner_id, &customer).await?;
        }
        Ok(customer)
    }
    .await;
    let verified_at = Utc::now();
    match result {
        Ok(customer) => {
            let matched = wallets.update_one(filter, doc! {"$set": {
                "stripe_enrollment.activated_at": bson::DateTime::from_chrono(enrollment.activated_at.unwrap_or(verified_at)),
                "stripe_enrollment.usage_verified_until": bson::DateTime::from_chrono(verified_at + Duration::minutes(10)),
            }}).await?;
            if matched.matched_count != 1 {
                return Err(unavailable());
            }
            Ok(customer)
        }
        Err(error) => {
            if matches!(error, AppError::Conflict(_)) {
                wallets
                    .update_one(
                        filter,
                        doc! {"$unset": {"stripe_enrollment.usage_verified_until": ""}},
                    )
                    .await?;
            }
            Err(error)
        }
    }
}

pub(super) async fn ensure_usage_period(
    db: &Database,
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
) -> AppResult<()> {
    let Some(enrollment) = &wallet.stripe_enrollment else {
        return Ok(());
    };
    if !enrollment.reversal_holds.is_empty() {
        return Err(AppError::WalletPaymentReview);
    }
    let now = Utc::now();
    if enrollment
        .usage_verified_until
        .is_some_and(|until| until > now)
    {
        return Ok(());
    }
    let config = lago.credit_period_config().ok_or_else(unavailable)?;
    let wallets = db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME);
    let filter = doc! {"_id": &wallet.id, "tax_generation": wallet.tax_generation,
        "stripe_enrollment.stripe_account_id": &config.account_id,
        "stripe_enrollment.livemode": config.livemode,
        "stripe_enrollment.lago_subscription_id": &enrollment.lago_subscription_id,
        "lago_wallet_id": &enrollment.lago_wallet_id,
    };
    let mut claim = filter.clone();
    claim.insert("$and", vec![
        doc! {"$or": [{"stripe_enrollment.usage_verified_until": null},
            {"stripe_enrollment.usage_verified_until": {"$lte": bson::DateTime::from_chrono(now)}}]},
        doc! {"$or": [{"stripe_enrollment.retry_verification_after": null},
            {"stripe_enrollment.retry_verification_after": {"$lte": bson::DateTime::from_chrono(now)}}]},
    ]);
    if wallets.update_one(claim, doc! {"$set": {
        "stripe_enrollment.retry_verification_after": bson::DateTime::from_chrono(now + Duration::seconds(15)),
    }}).await?.matched_count == 0 {
        let fresh = wallets.find_one(filter).await?.and_then(|w| w.stripe_enrollment)
            .and_then(|e| e.usage_verified_until).is_some_and(|until| until > Utc::now());
        return if fresh { Ok(()) } else { Err(unavailable()) };
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(12),
        verify_wallet_config(lago, config, wallet),
    )
    .await
    .map_err(|_| unavailable())??;
    if wallets.update_one(filter, doc! {"$set": {
        "stripe_enrollment.usage_verified_until": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(10)),
    }}).await?.matched_count != 1 {
        return Err(unavailable());
    }
    Ok(())
}

pub(super) async fn reconcile_wallets(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
) -> AppResult<u64> {
    let wallets = db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME);
    let mut cursor = wallets.find(doc! {
        "stripe_enrollment.stripe_account_id": &stripe.settings.account_id,
        "stripe_enrollment.livemode": stripe.settings.mode.livemode(),
        "stripe_enrollment.activated_at": {"$type": "date"},
        "$or": [
            {"stripe_enrollment.usage_verified_until": null},
            {"stripe_enrollment.usage_verified_until": {"$lte": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(5))}},
        ]
    }).sort(doc! {"stripe_enrollment.last_verification_attempt": 1, "_id": 1}).limit(20).await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut count = 0;
    while tokio::time::Instant::now() < deadline && super::reconcile::continue_reconcile() {
        let Some(wallet) = cursor.try_next().await? else {
            break;
        };
        wallets.update_one(doc! {"_id": &wallet.id, "stripe_enrollment": {"$type": "object"}},
            doc! {"$set": {"stripe_enrollment.last_verification_attempt": bson::DateTime::now()}}).await?;
        match verify_and_publish_wallet(db, lago, stripe, &wallet).await {
            Ok(_) => count += 1,
            Err(error) => {
                tracing::warn!(owner_id = %wallet.owner_id, %error, "Stripe-funded usage period verification failed")
            }
        }
    }
    Ok(count)
}

pub(super) async fn create(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    wallet: &BillingWallet,
    amount_credits: i64,
    idempotency_key: &str,
) -> AppResult<TopUpCheckout> {
    super::exact_migration::require_ready(db).await?;
    let key = provisioning::normalize_idempotency_key(idempotency_key)?;
    let id = provisioning::deterministic_topup_session_id(&wallet.owner_id, &key);
    let rows = sessions(db);
    let existing = rows
        .find_one(doc! {"_id": &id, "owner_id": &wallet.owner_id})
        .await?;
    if existing.is_none() {
        if !(1..=10_000_000).contains(&amount_credits) {
            return Err(AppError::BadRequest(
                "Purchase quantity must be 1 to 10000000 credits".into(),
            ));
        }
        quote_subtotal(stripe, amount_credits)?;
    }
    let customer_id = match wallet
        .stripe_enrollment
        .as_ref()
        .and_then(|e| e.activated_at)
    {
        Some(_) => verify_wallet(lago, stripe, wallet).await?,
        None => verify_and_publish_wallet(db, lago, stripe, wallet).await?,
    };
    if wallet
        .stripe_enrollment
        .as_ref()
        .is_some_and(|e| !e.reversal_holds.is_empty())
    {
        return Err(AppError::WalletPaymentReview);
    }
    let mut reused = existing.is_some();
    let order = match existing {
        Some(order) => order,
        None => {
            let subtotal = quote_subtotal(stripe, amount_credits)?;
            let now = Utc::now();
            let order = BillingTopUpSession {
                id: id.clone(),
                owner_id: wallet.owner_id.clone(),
                idempotency_key: key,
                amount_credits,
                lago_wallet_id: wallet.lago_wallet_id.clone().ok_or_else(unavailable)?,
                stripe_purchase: Some(StripePurchase {
                    account_id: stripe.settings.account_id.clone(),
                    livemode: stripe.settings.mode.livemode(),
                    customer_id: customer_id.clone(),
                    currency: "usd".into(),
                    subtotal_minor: subtotal,
                    unit_amount_minor: stripe.settings.unit_amount_minor,
                    offer_version: stripe.settings.offer_version.clone(),
                    product_tax_code: stripe.settings.product_tax_code.clone(),
                    return_url: stripe.return_url(&wallet.owner_id),
                    checkout_expires_at: now + Duration::hours(2),
                    checkout_session_id: None,
                    payment_intent_id: None,
                    stripe_invoice_id: None,
                    tax_minor: None,
                    total_minor: None,
                    expired: false,
                    async_failed: false,
                    reversal_review_required: false,
                    reversal_totals_agree: false,
                    stripe_charge_id: None,
                    refunded_minor: 0,
                    reversals: vec![],
                    used_reversal_decision_ids: vec![],
                    payment_observed_at: None,
                    credits_available_at: None,
                }),
                lago_wallet_transaction_id: None,
                lago_invoice_id: None,
                payment_url: None,
                payment_provider: Some("stripe".into()),
                status: BillingTopUpStatus::Pending,
                paid_at: None,
                credits_expire_at: None,
                expired_credits: Credits::ZERO,
                credits_expired_at: None,
                expiry_void_transaction_id: None,
                created_at: now,
                updated_at: now,
            };
            match rows.insert_one(&order).await {
                Ok(_) => order,
                Err(error) if provisioning::is_duplicate_key_error(&error) => {
                    reused = true;
                    rows.find_one(doc! {"_id": &id, "owner_id": &wallet.owner_id})
                        .await?
                        .ok_or_else(unavailable)?
                }
                Err(error) => return Err(error.into()),
            }
        }
    };
    let purchase = order.stripe_purchase.as_ref().ok_or_else(|| {
        AppError::Conflict("An existing Lago checkout already uses this key".into())
    })?;
    if order.amount_credits != amount_credits {
        return Err(AppError::Conflict(
            "idempotency_key was already used with a different amount_credits".into(),
        ));
    }
    if purchase.account_id != stripe.settings.account_id
        || purchase.livemode != stripe.settings.mode.livemode()
        || purchase.customer_id != customer_id
        || wallet.lago_wallet_id.as_deref() != Some(order.lago_wallet_id.as_str())
    {
        return Err(unavailable());
    }
    if purchase.expired
        || purchase.async_failed
        || order.paid_at.is_some()
        || Utc::now() >= purchase.checkout_expires_at
    {
        return Err(AppError::Conflict(
            "This Stripe checkout is no longer open. Check purchase history for its status.".into(),
        ));
    }
    if purchase.checkout_session_id.is_some() {
        stripe.verify_account().await?;
        return Ok(TopUpCheckout {
            session: order,
            reused: true,
        });
    }
    let session = recover_checkout(db, stripe, &order).await?.ok_or_else(|| {
        AppError::Conflict("This checkout attempt expired. Start a new purchase.".into())
    })?;
    Ok(TopUpCheckout { session, reused })
}

async fn recover_checkout(
    db: &Database,
    stripe: &Stripe,
    order: &BillingTopUpSession,
) -> AppResult<Option<BillingTopUpSession>> {
    let purchase = order.stripe_purchase.as_ref().ok_or_else(unavailable)?;
    let id = &order.id;
    let rows = sessions(db);
    let pending = doc! {"_id": id, "stripe_purchase.checkout_session_id": null};
    // This fixed lifetime stays within Stripe's idempotency retention. A
    // locally unbound Checkout URL has never been returned to a customer.
    if Utc::now() >= purchase.checkout_expires_at - Duration::minutes(31) {
        rows.update_one(pending, doc! {"$set": {"stripe_purchase.expired": true}})
            .await?;
        return Ok(None);
    }
    let checkout = stripe
        .create_checkout(id, &order.owner_id, purchase, order.amount_credits)
        .await?;
    checkout.verify_identity(id, purchase)?;
    if checkout.status != "open"
        || !checkout
            .id
            .starts_with(stripe.settings.mode.checkout_prefix())
    {
        return Err(unavailable());
    }
    let url = hosted_url(
        checkout.url.as_deref().ok_or_else(unavailable)?,
        "checkout.stripe.com",
    )?;
    rows.update_one(
        pending,
        doc! {"$set": {"stripe_purchase.checkout_session_id": &checkout.id,
        "stripe_purchase.expired": false,
        "payment_url": url, "status": "checkout_created",
        "updated_at": bson::DateTime::now()}},
    )
    .await?;
    rows.find_one(doc! {"_id": id, "stripe_purchase.checkout_session_id": checkout.id})
        .await
        .map_err(Into::into)
}

fn quote_subtotal(stripe: &Stripe, quantity: i64) -> AppResult<i64> {
    let subtotal = stripe
        .settings
        .unit_amount_minor
        .checked_mul(quantity)
        .filter(|subtotal| *subtotal <= stripe.settings.maximum_subtotal_minor)
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "The maximum for this purchase offer is {} credits",
                offer(stripe).maximum_credits
            ))
        })?;
    if subtotal < 1_000 {
        return Err(AppError::BadRequest(
            "Minimum purchase subtotal is USD 10".into(),
        ));
    }
    Ok(subtotal)
}

pub(super) async fn handle_event(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    signature: &str,
    body: &[u8],
) -> AppResult<()> {
    let event = stripe.verify_event(signature, body)?;
    let object = event.data.object;
    let filter = match event.kind.as_str() {
        "checkout.session.completed"
        | "checkout.session.async_payment_succeeded"
        | "checkout.session.async_payment_failed"
        | "checkout.session.expired" => {
            let Some(id) = object.client_reference_id else {
                return Ok(());
            };
            doc! {"_id": id, "stripe_purchase.checkout_session_id": object.id}
        }
        "charge.refunded"
        | "refund.created"
        | "refund.updated"
        | "refund.failed"
        | "charge.dispute.created"
        | "charge.dispute.updated"
        | "charge.dispute.closed" => {
            let Some(id) = object.payment_intent else {
                return Ok(());
            };
            doc! {"stripe_purchase.payment_intent_id": id}
        }
        _ => return Ok(()),
    };
    if let Some(order) = sessions(db).find_one(filter).await? {
        reconcile_order(db, lago, stripe, &order).await?;
        if event.kind == "checkout.session.async_payment_failed" {
            sessions(db).update_one(
                doc! {"_id": &order.id, "paid_at": null, "stripe_purchase.payment_intent_id": null},
                doc! {"$set": {"stripe_purchase.async_failed": true}},
            ).await?;
        }
    }
    Ok(())
}

pub(super) async fn reconcile(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
) -> AppResult<u64> {
    let rows = sessions(db);
    let permitted = doc! {
        "stripe_purchase.account_id": &stripe.settings.account_id,
        "stripe_purchase.livemode": stripe.settings.mode.livemode(),
        "stripe_purchase.expired": false,
        "stripe_purchase.async_failed": {"$ne": true},
    };
    let open_or_recovery = [
        doc! {"stripe_purchase.credits_available_at": null},
        doc! {"stripe_purchase.stripe_invoice_id": null, "paid_at": {"$type": "date"}},
        doc! {"stripe_purchase.reversal_review_required": true},
    ];
    let mut pending = permitted.clone();
    pending.insert("$or", open_or_recovery.to_vec());
    let mut recent = permitted;
    recent.insert("$nor", open_or_recovery.to_vec());
    recent.insert(
        "paid_at",
        doc! {"$gte": bson::DateTime::from_chrono(Utc::now() - Duration::days(180))},
    );
    recent.insert(
        "updated_at",
        doc! {"$lte": bson::DateTime::from_chrono(Utc::now() - Duration::hours(1))},
    );

    // A larger recent-payment backlog must not delay a newly paid order.
    let mut count = 0;
    for (filter, limit) in [(pending, 64), (recent, 64)] {
        if !super::reconcile::continue_reconcile() {
            break;
        }
        let mut cursor = rows
            .find(filter)
            .sort(doc! {"updated_at": 1, "_id": 1})
            .limit(limit)
            .await?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        while tokio::time::Instant::now() < deadline && super::reconcile::continue_reconcile() {
            let mut batch = Vec::with_capacity(8);
            for _ in 0..8 {
                let Some(order) = cursor.try_next().await? else {
                    break;
                };
                batch.push(order);
            }
            if batch.is_empty() {
                break;
            }
            use futures::StreamExt;
            let completed = futures::stream::iter(batch)
                .map(|order| async move {
                    let result = reconcile_candidate(db, lago, stripe, &order).await;
                    match result {
                        Ok(()) => 1_u64,
                        Err(error) => {
                            tracing::warn!(order_id = %order.id, %error, "Stripe purchase will be retried");
                            0
                        }
                    }
                })
                .buffer_unordered(8)
                .fold(0, |total, value| async move { total + value })
                .await;
            count += completed;
        }
    }
    Ok(count)
}

async fn reconcile_candidate(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    order: &BillingTopUpSession,
) -> AppResult<()> {
    sessions(db)
        .update_one(
            doc! {"_id": &order.id},
            doc! {"$set": {"updated_at": bson::DateTime::now()}},
        )
        .await?;
    let purchase = order.stripe_purchase.as_ref().ok_or_else(unavailable)?;
    if purchase.checkout_session_id.is_some() {
        return reconcile_order(db, lago, stripe, order).await;
    }
    if Utc::now() >= purchase.checkout_expires_at - Duration::minutes(31) {
        return recover_checkout(db, stripe, order).await.map(|_| ());
    }
    let wallet = provisioning::get_wallet(db, &order.owner_id)
        .await?
        .ok_or_else(unavailable)?;
    if wallet.lago_wallet_id.as_deref() != Some(order.lago_wallet_id.as_str())
        || verify_wallet(lago, stripe, &wallet).await? != purchase.customer_id
    {
        return Err(unavailable());
    }
    if wallet
        .stripe_enrollment
        .as_ref()
        .is_some_and(|e| !e.reversal_holds.is_empty())
    {
        return Err(AppError::WalletPaymentReview);
    }
    recover_checkout(db, stripe, order).await.map(|_| ())
}

async fn reconcile_order(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    order: &BillingTopUpSession,
) -> AppResult<()> {
    let rows = sessions(db);
    let purchase = order.stripe_purchase.as_ref().ok_or_else(unavailable)?;
    let checkout_id = purchase
        .checkout_session_id
        .as_deref()
        .ok_or_else(unavailable)?;
    if purchase.account_id != stripe.settings.account_id
        || purchase.livemode != stripe.settings.mode.livemode()
    {
        return Err(unavailable());
    }
    let observed_at = bson::DateTime::now();
    stripe.verify_account().await?;
    let checkout = stripe.checkout(checkout_id).await?;
    checkout.verify_identity(&order.id, purchase)?;
    if checkout.id != checkout_id {
        return Err(unavailable());
    }
    if checkout.status == "expired" {
        rows.update_one(
            doc! {"_id": &order.id, "paid_at": null},
            doc! {"$set": {"stripe_purchase.expired": true}},
        )
        .await?;
        return Ok(());
    }
    // Checkout completion with an asynchronous payment still pending can be
    // replayed. The authoritative paid state is required on every attempt.
    if checkout.payment_status != "paid" {
        return Ok(());
    }
    checkout.verify_paid()?;
    if checkout.amount_total > MAX_STRIPE_USD_MINOR {
        return Err(unavailable());
    }
    let intent_id = checkout.payment_intent.as_deref().ok_or_else(unavailable)?;
    let payment = stripe.payment(intent_id).await?;
    if payment.id != intent_id
        || payment.livemode != purchase.livemode
        || payment.customer != purchase.customer_id
        || payment.currency != purchase.currency
        || payment.status != "succeeded"
        || payment.amount_received != checkout.amount_total
    {
        return Err(unavailable());
    }
    if !payment.latest_charge.id.starts_with("ch_") {
        return Err(unavailable());
    }
    let observations = reversals::observe(stripe, purchase, &payment).await?;
    let totals_agree = observations.totals_agree;
    let mut update = doc! {
        "stripe_purchase.async_failed": false,
        "stripe_purchase.payment_intent_id": intent_id,
        "stripe_purchase.tax_minor": checkout.total_details.amount_tax,
        "stripe_purchase.total_minor": checkout.amount_total,
        "stripe_purchase.stripe_charge_id": &payment.latest_charge.id,
        "stripe_purchase.refunded_minor": observations.succeeded_minor,
        "stripe_purchase.payment_observed_at": observed_at,
        "updated_at": bson::DateTime::now(),
    };
    if let Some(invoice_id) = &checkout.invoice {
        update.insert("stripe_purchase.stripe_invoice_id", invoice_id);
    }
    let mut txn = db.client().start_session().await?;
    let purchase_filter = doc! {"_id": &order.id, "stripe_purchase.checkout_session_id": checkout_id,
        "$and": [
            {"$or": [{"stripe_purchase.payment_intent_id": null}, {"stripe_purchase.payment_intent_id": intent_id}]},
            {"$or": [{"stripe_purchase.payment_observed_at": null}, {"stripe_purchase.payment_observed_at": {"$lt": observed_at}}]},
        ],
    };
    txn.start_transaction().await?;
    let recorded: AppResult<Option<(bool, bool, bool)>> = async {
        let found = rows
            .find_one(purchase_filter.clone())
            .session(&mut txn)
            .await?;
        let Some(current) = found else {
            return Ok(None);
        };
        let current_purchase = current.stripe_purchase.as_ref().ok_or_else(unavailable)?;
        let merged = reversals::merge(current_purchase, observations.reversals)?;
        let hold = !totals_agree || reversals::held(&merged);
        let authorized_reversal = reversals::pending_terminal(&merged);
        let pending_reversal = reversals::pending(&merged);
        update.insert("stripe_purchase.reversals", bson::to_bson(&merged).map_err(|_| unavailable())?);
        update.insert("stripe_purchase.reversal_review_required", hold);
        update.insert("stripe_purchase.reversal_totals_agree", totals_agree);
        if current.paid_at.is_none() {
            update.insert("paid_at", observed_at);
            let key = crate::services::audit_service::audit_chain_hmac_key().ok_or_else(unavailable)?;
            crate::services::audit_service::log_system_event_in_session(
                db, &mut txn, key, "billing_stripe_payment_observed",
                serde_json::json!({"purchase_id": order.id, "account_id": purchase.account_id,
                    "stripe_payment_intent_id": intent_id, "stripe_checkout_id": checkout_id,
                    "currency": purchase.currency, "subtotal_minor": purchase.subtotal_minor,
                    "tax_minor": checkout.total_details.amount_tax, "total_minor": checkout.amount_total,
                    "credits": order.amount_credits, "livemode": purchase.livemode}),
            ).await?;
        }
        rows.update_one(purchase_filter, doc! {"$set": update})
            .session(&mut txn)
            .await?;
        // Mark the order and freeze wallet admission in one transaction.
        // A verified won dispute releases its hold only if no refund remains.
        if hold
            || current
                .stripe_purchase
                .is_some_and(|p| p.reversal_review_required)
        {
            let holds = if hold {
                doc! {"$addToSet": {"stripe_enrollment.reversal_holds": &order.id}}
            } else {
                doc! {"$pull": {"stripe_enrollment.reversal_holds": &order.id}}
            };
            let changed = db
                .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
                .update_one(
                    doc! {"owner_id": &order.owner_id, "lago_wallet_id": &order.lago_wallet_id,
                    "stripe_enrollment.stripe_account_id": &purchase.account_id},
                    holds,
                )
                .session(&mut txn)
                .await?;
            if changed.matched_count != 1 {
                return Err(unavailable());
            }
        }
        Ok(Some((hold, pending_reversal, authorized_reversal)))
    }
    .await;
    let accepted = match recorded {
        Ok(found) => {
            txn.commit_transaction().await?;
            found
        }
        Err(error) => {
            txn.abort_transaction().await?;
            return Err(error);
        }
    };
    let Some((hold, pending_reversal, authorized_reversal)) = accepted else {
        return Ok(());
    };
    if !totals_agree {
        return Err(AppError::BillingProviderUnavailable(
            "Stripe charge and refund totals are temporarily inconsistent; payment review hold is active".into(),
        ));
    }
    if hold && !authorized_reversal {
        if pending_reversal {
            reversals::apply(db, lago, stripe, order).await?;
        }
        return Ok(());
    }
    if purchase.credits_available_at.is_some() {
        if pending_reversal {
            reversals::apply(db, lago, stripe, order).await?;
        }
        return Ok(());
    }
    let wallet = provisioning::get_wallet(db, &order.owner_id)
        .await?
        .ok_or_else(unavailable)?;
    if wallet.lago_wallet_id.as_deref() != Some(order.lago_wallet_id.as_str())
        || verify_and_publish_wallet(db, lago, stripe, &wallet).await? != purchase.customer_id
    {
        return Err(unavailable());
    }
    let provider_payment = ExternalPayment {
        purchase_id: order.id.clone(),
        stripe_account_id: purchase.account_id.clone(),
        stripe_customer_id: purchase.customer_id.clone(),
        stripe_checkout_id: checkout.id,
        stripe_payment_intent_id: intent_id.into(),
        currency: purchase.currency.clone(),
        credits: order.amount_credits,
        subtotal_minor: purchase.subtotal_minor,
        tax_minor: checkout.total_details.amount_tax,
        total_minor: checkout.amount_total,
    };
    let receipt = lago
        .record_external_credit_purchase(&wallet, &provider_payment, stripe.lago_secret.as_str())
        .await?;
    if receipt.payment != provider_payment
        || receipt.wallet_id != order.lago_wallet_id
        || receipt.status != "settled"
        || receipt.transaction_type != "inbound"
        || receipt.transaction_status != "purchased"
    {
        return Err(unavailable());
    }
    rows.update_one(
        doc! {"_id": &order.id},
        doc! {"$set": {"lago_wallet_transaction_id": &receipt.wallet_transaction_id,
            "credits_expire_at": bson::DateTime::from_chrono(receipt.settled_at + Duration::days(365))}},
    ).await?;
    let balance = super::webhook::provider_effective_balance(lago, &wallet).await?;
    let Some(outcome) = super::webhook::refresh_wallet_balance(db, &wallet, balance).await? else {
        // An intervening wallet write deferred this refresh. Keep this order
        // pending until a later attempt reads back the available balance.
        return Ok(());
    };
    super::ledger::record_stripe_purchase(
        db,
        &order.owner_id,
        &order.lago_wallet_id,
        &provider_payment,
        outcome.balance_credits,
    )
    .await?;
    rows.update_one(
        doc! {"_id": &order.id, "stripe_purchase.credits_available_at": null},
        doc! {"$set": {"stripe_purchase.credits_available_at": bson::DateTime::now()}},
    )
    .await?;
    if authorized_reversal {
        let current = rows
            .find_one(doc! {"_id": &order.id})
            .await?
            .ok_or_else(unavailable)?;
        reversals::apply(db, lago, stripe, &current).await?;
    }
    Ok(())
}

pub(super) async fn document(
    db: &Database,
    stripe: &Stripe,
    owner_id: &str,
    order_id: &str,
) -> AppResult<String> {
    let not_found = || AppError::NotFound("Purchase document not found".into());
    let order = sessions(db)
        .find_one(doc! {"_id": order_id, "owner_id": owner_id})
        .await?
        .ok_or_else(not_found)?;
    let purchase = order.stripe_purchase.as_ref().ok_or_else(not_found)?;
    let total = purchase.total_minor.ok_or_else(not_found)?;
    if purchase.account_id != stripe.settings.account_id
        || purchase.livemode != stripe.settings.mode.livemode()
        || order.paid_at.is_none()
    {
        return Err(not_found());
    }
    let invoice_id = purchase.stripe_invoice_id.as_deref().ok_or_else(|| {
        AppError::Conflict("Stripe is still generating the purchase document".into())
    })?;
    stripe.verify_account().await?;
    let invoice = stripe.invoice(invoice_id).await?;
    if invoice.id != invoice_id
        || invoice.livemode != purchase.livemode
        || invoice.customer != purchase.customer_id
        || invoice.status != "paid"
        || invoice.currency != purchase.currency
        || invoice.subtotal != purchase.subtotal_minor
        || invoice.amount_paid != total
        || invoice.amount_remaining != 0
    {
        return Err(unavailable());
    }
    hosted_url(
        invoice
            .hosted_invoice_url
            .as_deref()
            .ok_or_else(unavailable)?,
        "invoice.stripe.com",
    )
}

pub(super) async fn diagnostics(db: &Database) -> AppResult<Vec<super::BillingStartupDiagnostic>> {
    let now = Utc::now();
    let mut items = vec![];
    let wallets = db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME);
    let expired_wallets = wallets.count_documents(doc! {
        "stripe_enrollment.activated_at": {"$type": "date"},
        "$or": [{"stripe_enrollment.usage_verified_until": null},
            {"stripe_enrollment.usage_verified_until": {"$lte": bson::DateTime::from_chrono(now)}}],
    }).await?;
    if expired_wallets > 0 {
        items.push(super::BillingStartupDiagnostic {
            code: "stripe_usage_period_verification".into(),
            severity: super::DiagnosticSeverity::Action,
            technical_details: None,
            summary: "Stripe-funded usage periods need verification".into(),
            detail: format!("{expired_wallets} enrolled wallet(s) have an expired or missing Lago verification."),
            remediation: "Check the Lago connector, payment provider settings and effective usage tax for the enrolled wallets.".into(),
            detected_at: now,
        });
    }
    let rows = sessions(db);
    let filter = doc! {"stripe_purchase.reversal_review_required": true};
    let pending_reviews = rows.count_documents(filter.clone()).await?;
    if pending_reviews > 0 {
        let sample: Vec<bson::Document> = rows
            .clone_with_type::<bson::Document>()
            .find(filter)
            .projection(doc! {"_id": 1})
            .sort(doc! {"updated_at": 1, "_id": 1})
            .limit(5)
            .await?
            .try_collect()
            .await?;
        let ids = sample
            .iter()
            .filter_map(|row| row.get_str("_id").ok())
            .collect::<Vec<_>>()
            .join(", ");
        items.push(super::BillingStartupDiagnostic {
            code: "stripe_purchase_reversal_review".into(),
            severity: super::DiagnosticSeverity::Action,
            technical_details: Some(ids),
            summary: "Stripe payment reversals need review".into(),
            detail: format!("{pending_reviews} purchase(s) have a refund or unresolved dispute that holds wallet admission."),
            remediation: "Review the Stripe payments and approve only independently verified unused-credit refunds.".into(),
            detected_at: now,
        });
    }
    let filter =
        doc! {"stripe_purchase.reversals": {"$elemMatch": reversals::finance_correction_filter()}};
    let pending_corrections = rows.count_documents(filter.clone()).await?;
    if pending_corrections > 0 {
        let sample: Vec<BillingTopUpSession> = rows
            .find(filter)
            .sort(doc! {"updated_at": 1, "_id": 1})
            .limit(5)
            .await?
            .try_collect()
            .await?;
        let ids = sample
            .iter()
            .flat_map(|order| {
                order
                    .stripe_purchase
                    .iter()
                    .flat_map(|purchase| purchase.reversals.iter())
                    .filter(|event| reversals::requires_finance_correction(event))
                    .map(|event| format!("{}/{}", order.id, event.provider_id))
            })
            .take(5)
            .collect::<Vec<_>>()
            .join(", ");
        items.push(super::BillingStartupDiagnostic {
            code: "stripe_purchase_finance_correction".into(),
            severity: super::DiagnosticSeverity::Action,
            technical_details: Some(ids),
            summary: "Stripe credit reversals require Finance correction".into(),
            detail: format!("{pending_corrections} purchase(s) have a committed Lago void for a withdrawn Stripe outcome."),
            remediation: "Review the purchase and reversal IDs. After Finance records the correction, acknowledge it with its accounting reference and Lago void ID.".into(),
            detected_at: now,
        });
    }
    Ok(items)
}

fn unavailable() -> AppError {
    AppError::BillingProviderUnavailable(
        "Stripe credit purchase could not be verified for this wallet".into(),
    )
}
