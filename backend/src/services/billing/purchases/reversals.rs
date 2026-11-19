use super::*;
use crate::models::billing_topup_session::{
    StripeFinanceCorrection, StripeReversal, StripeReversalAction, StripeReversalDecision,
    StripeReversalState,
};
use crate::services::audit_service;

const WON: &str = "won";
const WITHDRAWN: [&str; 3] = ["failed", "canceled", WON];

pub(super) struct Observations {
    pub reversals: Vec<StripeReversal>,
    pub succeeded_minor: i64,
    pub totals_agree: bool,
}

pub(super) async fn observe(
    stripe: &Stripe,
    purchase: &StripePurchase,
    payment: &crate::services::billing::stripe::PaymentIntent,
) -> AppResult<Observations> {
    let charge = &payment.latest_charge;
    let mut observations = Vec::new();
    let mut succeeded_minor = 0_i64;
    let mut pending_minor = 0_i64;
    {
        let refunds = stripe.refunds(&charge.id).await?;
        for refund in refunds {
            if refund.charge.as_deref() != Some(charge.id.as_str())
                || refund
                    .payment_intent
                    .as_ref()
                    .is_some_and(|intent| intent != &payment.id)
                || refund.currency != purchase.currency
                || !valid_id(&refund.id, "re_")
                || !(1..=payment.amount_received).contains(&refund.amount)
            {
                return Err(unavailable());
            }
            let total = match refund.status.as_deref() {
                Some("succeeded") => Some(&mut succeeded_minor),
                Some("failed" | "canceled") => None,
                _ => Some(&mut pending_minor),
            };
            if let Some(total) = total {
                *total = total.checked_add(refund.amount).ok_or_else(unavailable)?;
            }
            observations.push(StripeReversal {
                provider_id: refund.id,
                status: refund.status.unwrap_or_else(|| "pending".into()),
                amount_minor: refund.amount,
                decision: None,
                finance_correction: None,
            });
        }
    }
    {
        let disputes = stripe.disputes(&charge.id).await?;
        if charge.disputed && disputes.is_empty() {
            return Err(unavailable());
        }
        for dispute in disputes {
            if dispute.livemode != purchase.livemode
                || dispute.charge != charge.id
                || dispute
                    .payment_intent
                    .as_ref()
                    .is_some_and(|intent| intent != &payment.id)
                || dispute.currency != purchase.currency
                || !valid_id(&dispute.id, "dp_")
                || !(1..=payment.amount_received).contains(&dispute.amount)
            {
                return Err(unavailable());
            }
            observations.push(StripeReversal {
                provider_id: dispute.id,
                status: dispute.status,
                amount_minor: dispute.amount,
                decision: None,
                finance_correction: None,
            });
        }
    }
    // Stripe may count in-flight refunds in the charge aggregate. Only
    // succeeded refunds contribute to the customer history's paid-out total.
    let possible_minor = succeeded_minor
        .checked_add(pending_minor)
        .ok_or_else(unavailable)?;
    let totals_agree = possible_minor <= payment.amount_received
        && (succeeded_minor..=possible_minor).contains(&charge.amount_refunded);
    Ok(Observations {
        reversals: observations,
        succeeded_minor,
        totals_agree,
    })
}

pub(super) fn merge(
    current: &StripePurchase,
    mut observations: Vec<StripeReversal>,
) -> AppResult<Vec<StripeReversal>> {
    for existing in &current.reversals {
        let observed = observations
            .iter_mut()
            .find(|r| r.provider_id == existing.provider_id)
            .ok_or_else(unavailable)?;
        if observed.amount_minor != existing.amount_minor {
            return Err(unavailable());
        }
        observed.decision = existing.decision.clone();
        observed.finance_correction = existing.finance_correction.clone();
    }
    observations.sort_unstable_by(|a, b| a.provider_id.cmp(&b.provider_id));
    if observations
        .windows(2)
        .any(|pair| pair[0].provider_id == pair[1].provider_id)
    {
        return Err(unavailable());
    }
    Ok(observations)
}

pub(super) fn held(reversals: &[StripeReversal]) -> bool {
    reversals.iter().any(|r| {
        if r.decision
            .as_ref()
            .is_some_and(|d| d.state == StripeReversalState::Pending)
        {
            return true;
        }
        !(r.provider_id.starts_with("re_") && matches!(r.status.as_str(), "failed" | "canceled"))
            && r.status != WON
            && r.decision
                .as_ref()
                .is_none_or(|d| d.state != StripeReversalState::Applied)
    })
}

fn terminal(r: &StripeReversal) -> bool {
    if r.provider_id.starts_with("re_") {
        r.status == "succeeded"
    } else {
        matches!(
            r.status.as_str(),
            "lost" | "warning_closed" | "prevented" | "charge_refunded"
        )
    }
}

pub(crate) fn valid_id(value: &str, prefix: &str) -> bool {
    value.len() <= 128
        && value
            .strip_prefix(prefix)
            .is_some_and(|suffix| !suffix.is_empty())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub decision_id: uuid::Uuid,
    pub action: StripeReversalAction,
    pub credits: i64,
    pub reason: String,
}

pub(in crate::services::billing) struct ReversalAudit<'a> {
    pub actor: &'a audit_service::AuditActor,
    pub key: &'a [u8],
}

pub(in crate::services::billing) async fn list(
    db: &Database,
    order_id: &str,
) -> AppResult<Vec<StripeReversal>> {
    Ok(sessions(db)
        .find_one(doc! {"_id": order_id})
        .await?
        .and_then(|order| order.stripe_purchase)
        .ok_or_else(|| AppError::NotFound("Stripe purchase not found".into()))?
        .reversals)
}

pub(in crate::services::billing) async fn resolve(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    order_id: &str,
    provider_id: &str,
    resolution: Resolution,
    audit: ReversalAudit<'_>,
) -> AppResult<()> {
    let reason = resolution.reason.trim();
    let reverse = resolution.action == StripeReversalAction::ReverseCredits;
    if reason.is_empty()
        || reason.len() > 512
        || (reverse && resolution.credits <= 0)
        || (!reverse && resolution.credits != 0)
        || !(valid_id(provider_id, "re_") || valid_id(provider_id, "dp_"))
    {
        return Err(AppError::BadRequest(
            "A reversal requires a decision ID, action, reason and approved credit quantity".into(),
        ));
    }
    let rows = sessions(db);
    let order = rows
        .find_one(doc! {"_id": order_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Stripe purchase not found".into()))?;
    let purchase = order.stripe_purchase.as_ref().ok_or_else(unavailable)?;
    if purchase.account_id != stripe.settings.account_id
        || purchase.livemode != stripe.settings.mode.livemode()
    {
        return Err(unavailable());
    }
    // Refresh before accepting a decision. Stripe can change a pending refund
    // or dispute while an operator has the review open.
    reconcile_order(db, lago, stripe, &order).await?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let decision_id = resolution.decision_id.to_string();
    let result = async {
        let order = rows.find_one(doc! {"_id": order_id}).session(&mut session).await?
            .ok_or_else(unavailable)?;
        let purchase = order.stripe_purchase.ok_or_else(unavailable)?;
        let target = purchase.reversals.iter().find(|r| r.provider_id == provider_id)
            .ok_or_else(|| AppError::NotFound("Stripe reversal not found".into()))?;
        if let Some(existing) = target.decision.as_ref() {
            if existing.decision_id == decision_id {
                if existing.credits == resolution.credits && existing.action == resolution.action && existing.reason == reason {
                    return Ok(());
                }
                return Err(AppError::Conflict("This decision ID already has a different resolution".into()));
            }
            if existing.state != StripeReversalState::Refused {
                return Err(AppError::Conflict("This Stripe reversal already has a decision".into()));
            }
        }
        if purchase.used_reversal_decision_ids.contains(&decision_id) {
            return Err(AppError::Conflict("This decision ID was already used".into()));
        }
        if !terminal(target) {
            return Err(AppError::Conflict("Stripe has not confirmed a reversible outcome".into()));
        }
        let allocated = purchase.reversals.iter().filter_map(|r| r.decision.as_ref())
            .filter(|d| d.state != StripeReversalState::Refused)
            .try_fold(resolution.credits, |sum, d| sum.checked_add(d.credits).ok_or_else(unavailable))?;
        if allocated > order.amount_credits {
            return Err(AppError::Conflict("The cumulative approved credit reversal exceeds this purchase".into()));
        }
        let decision = StripeReversalDecision {
            decision_id: decision_id.clone(), action: resolution.action, credits: resolution.credits,
            reason: reason.into(), state: if reverse { StripeReversalState::Pending } else { StripeReversalState::Applied },
            dispatch_started: false,
            lago_void_transaction_id: None,
        };
        let encoded = bson::to_document(&decision).map_err(|_| unavailable())?;
        let update = rows.update_one(
            doc! {"_id": order_id, "stripe_purchase.reversals.provider_id": provider_id},
            doc! {"$set": {"stripe_purchase.reversals.$.decision": encoded},
                  "$addToSet": {"stripe_purchase.used_reversal_decision_ids": &decision_id}},
        ).session(&mut session).await;
        if let Err(error) = update {
            if super::super::ledger::is_duplicate_key_error(&error) {
                return Err(AppError::Conflict("This decision ID was already used on another purchase".into()));
            }
            return Err(error.into());
        }
        audit_service::log_actor_event_in_session(db, &mut session, audit.key, audit.actor, "billing_stripe_reversal_decided",
            serde_json::json!({"purchase_id": order_id, "stripe_reversal_id": provider_id, "amount_minor": target.amount_minor,
                "decision": decision, "replaces_decision_id": target.decision.as_ref().map(|d| &d.decision_id)})).await?;
        Ok(())
    }.await;
    match result {
        Ok(()) => session.commit_transaction().await?,
        Err(error) => {
            session.abort_transaction().await?;
            return Err(error);
        }
    }
    let current = rows
        .find_one(doc! {"_id": order_id})
        .await?
        .ok_or_else(unavailable)?;
    reconcile_order(db, lago, stripe, &current).await
}

pub(crate) fn requires_finance_correction(reversal: &StripeReversal) -> bool {
    WITHDRAWN.contains(&reversal.status.as_str())
        && reversal.finance_correction.is_none()
        && reversal.decision.as_ref().is_some_and(|decision| {
            decision.state == StripeReversalState::Applied
                && decision.lago_void_transaction_id.is_some()
        })
}

pub(super) fn finance_correction_filter() -> bson::Document {
    doc! {"status": {"$in": WITHDRAWN.to_vec()}, "finance_correction": null,
    "decision.state": "applied", "decision.lago_void_transaction_id": {"$type": "string"}}
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinanceCorrection {
    pub lago_void_transaction_id: uuid::Uuid,
    pub reference: String,
}

pub(in crate::services::billing) async fn acknowledge_finance_correction(
    db: &Database,
    purchase_id: &str,
    provider_id: &str,
    input: FinanceCorrection,
    actor: &audit_service::AuditActor,
    audit_key: &[u8],
) -> AppResult<()> {
    let reference = input.reference.trim();
    if reference.is_empty() || reference.len() > 128 || reference.chars().any(char::is_control) {
        return Err(AppError::BadRequest(
            "A Finance correction requires a valid reference".into(),
        ));
    }
    let void_id = input.lago_void_transaction_id.to_string();
    let rows = sessions(db);
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let result = async {
        let reversal = rows.find_one(doc! {"_id": purchase_id})
            .session(&mut session).await?
            .and_then(|row| row.stripe_purchase)
            .and_then(|purchase| purchase.reversals.into_iter().find(|r| r.provider_id == provider_id))
            .ok_or_else(|| AppError::NotFound("Stripe reversal not found".into()))?;
        let decision = reversal.decision.as_ref().filter(|d| d.lago_void_transaction_id.as_deref() == Some(void_id.as_str()))
            .ok_or_else(|| AppError::Conflict("Finance correction must match the recorded Lago void".into()))?;
        if let Some(existing) = reversal.finance_correction.as_ref() {
            return if existing.reference == reference { Ok(()) } else {
                Err(AppError::Conflict("A Finance correction was already acknowledged for this void".into()))
            };
        }
        if !requires_finance_correction(&reversal) {
            return Err(AppError::Conflict("This reversal does not require a Finance correction".into()));
        }
        let correction = StripeFinanceCorrection { reference: reference.into(), acknowledged_at: Utc::now() };
        let mut event_filter = finance_correction_filter();
        event_filter.insert("provider_id", provider_id);
        event_filter.insert("decision.lago_void_transaction_id", void_id.as_str());
        let encoded = bson::to_document(&correction).map_err(|_| unavailable())?;
        let updated = rows.update_one(
            doc! {"_id": purchase_id, "stripe_purchase.reversals": {"$elemMatch": event_filter}},
            doc! {"$set": {"stripe_purchase.reversals.$.finance_correction": encoded}},
        ).session(&mut session).await?;
        if updated.modified_count != 1 {
            return Err(AppError::Conflict("The Finance correction changed. Read the reversal and retry.".into()));
        }
        audit_service::log_actor_event_in_session(db, &mut session, audit_key, actor,
            "billing_stripe_finance_correction_acknowledged",
            serde_json::json!({"purchase_id": purchase_id, "stripe_reversal_id": provider_id,
                "decision_id": decision.decision_id, "lago_void_transaction_id": void_id,
                "stripe_status": reversal.status, "credits": decision.credits, "reference": reference}),
        ).await?;
        Ok(())
    }.await;
    match result {
        Ok(()) => {
            session.commit_transaction().await?;
            Ok(())
        }
        Err(error) => {
            session.abort_transaction().await?;
            Err(error)
        }
    }
}

pub(super) fn pending_terminal(reversals: &[StripeReversal]) -> bool {
    reversals
        .iter()
        .any(|reversal| terminal(reversal) && pending(std::slice::from_ref(reversal)))
}

pub(super) fn pending(reversals: &[StripeReversal]) -> bool {
    reversals.iter().any(|r| {
        r.decision
            .as_ref()
            .is_some_and(|d| d.state == StripeReversalState::Pending)
    })
}

pub(super) async fn apply(
    db: &Database,
    lago: &dyn LagoApi,
    stripe: &Stripe,
    order: &BillingTopUpSession,
) -> AppResult<()> {
    let rows = sessions(db);
    let order = rows
        .find_one(doc! {"_id": &order.id})
        .await?
        .ok_or_else(unavailable)?;
    let purchase = order.stripe_purchase.as_ref().ok_or_else(unavailable)?;
    let wallet = provisioning::get_wallet(db, &order.owner_id)
        .await?
        .ok_or_else(unavailable)?;
    if wallet.lago_wallet_id.as_deref() != Some(order.lago_wallet_id.as_str()) {
        return Err(unavailable());
    }
    for candidate in &purchase.reversals {
        let fresh = rows
            .find_one(doc! {"_id": &order.id})
            .await?
            .and_then(|entry| entry.stripe_purchase)
            .ok_or_else(unavailable)?;
        if !fresh.reversal_totals_agree {
            return Err(unavailable());
        }
        let Some(reversal) = fresh
            .reversals
            .iter()
            .find(|r| r.provider_id == candidate.provider_id)
        else {
            continue;
        };
        let Some(decision) = reversal
            .decision
            .as_ref()
            .filter(|d| d.state == StripeReversalState::Pending)
        else {
            continue;
        };
        let cancel_only = !terminal(reversal);
        let mut authority = doc! {
            "provider_id": &reversal.provider_id, "status": &reversal.status,
            "decision.decision_id": &decision.decision_id, "decision.state": "pending",
        };
        let (state, void) = if cancel_only && !decision.dispatch_started {
            // No provider write was authorized. The final update below
            // compares both the observed status and the dispatch marker.
            authority.insert("decision.dispatch_started", false);
            (StripeReversalState::Refused, None)
        } else {
            let protected_minor = if cancel_only {
                0
            } else {
                if fresh.credits_available_at.is_none() {
                    continue;
                }
                if !wallet
                    .stripe_enrollment
                    .as_ref()
                    .is_some_and(|e| e.reversal_holds.contains(&order.id))
                    || wallet.reserved_credits != Credits::ZERO
                    || wallet.pending_lago_debits != Credits::ZERO
                    || wallet.active_topup_expiry.is_some()
                {
                    return Err(AppError::Conflict(
                        "Wallet funding or credit expiry is still in flight".into(),
                    ));
                }
                let subscription = wallet
                    .lago_subscription_id
                    .as_deref()
                    .ok_or_else(unavailable)?;
                let usage = lago
                    .current_usage(&wallet.lago_customer_id, subscription)
                    .await?;
                let protected =
                    super::super::lago_client::extract_current_usage_credits(&usage.raw)?;
                let cent = crate::models::credits::SCALE / 100;
                i64::try_from((protected.pico().max(0) + cent - 1) / cent)
                    .map_err(|_| unavailable())?
            };
            let matched = rows
                .update_one(
                    doc! {"_id": &order.id, "stripe_purchase.reversal_totals_agree": true,
                    "stripe_purchase.reversals": {"$elemMatch": &authority}},
                    doc! {"$set": {"stripe_purchase.reversals.$.decision.dispatch_started": true}},
                )
                .await?;
            if matched.matched_count == 0 {
                continue;
            }
            let input = super::super::lago_client::external_purchases::ExternalReversal {
                provider_id: &reversal.provider_id,
                decision_id: &decision.decision_id,
                credits: decision.credits,
                amount_minor: reversal.amount_minor,
                protected_minor,
                cancel_only,
            };
            // A committed receipt wins over cancellation. A cancellation
            // reaching Lago first prevents any delayed dispatch at that ID.
            authority.remove("status");
            match lago
                .record_external_credit_refund(
                    &wallet,
                    &order.id,
                    &input,
                    stripe.lago_secret.as_str(),
                )
                .await
            {
                Ok(void) => (StripeReversalState::Applied, Some(void)),
                Err(AppError::Conflict(_)) => (StripeReversalState::Refused, None),
                Err(error) => return Err(error),
            }
        };
        if state == StripeReversalState::Applied {
            let current = provisioning::get_wallet(db, &order.owner_id)
                .await?
                .ok_or_else(unavailable)?;
            let balance = super::super::webhook::provider_effective_balance(lago, &current).await?;
            if super::super::webhook::refresh_wallet_balance(db, &current, balance)
                .await?
                .is_none()
            {
                return Err(unavailable());
            }
        }
        let mut txn = db.client().start_session().await?;
        txn.start_transaction().await?;
        let persisted = async {
            let updated = rows.find_one_and_update(doc! {"_id": &order.id, "stripe_purchase.reversals": {"$elemMatch": &authority}}, doc! {"$set": {
                "stripe_purchase.reversals.$.decision.state": bson::to_bson(&state).map_err(|_| unavailable())?,
                "stripe_purchase.reversals.$.decision.lago_void_transaction_id": &void,
            }}).return_document(mongodb::options::ReturnDocument::After).session(&mut txn).await?;
            if let Some(purchase) = updated.and_then(|row| row.stripe_purchase) {
                let withdrawn = purchase.reversals.iter().any(|event|
                    event.provider_id == reversal.provider_id && !terminal(event));
                let key = audit_service::audit_chain_hmac_key().ok_or_else(unavailable)?;
                audit_service::log_system_event_in_session(db, &mut txn, key, "billing_stripe_reversal_settled",
                    serde_json::json!({"purchase_id": order.id, "stripe_reversal_id": reversal.provider_id,
                        "decision_id": decision.decision_id, "amount_minor": reversal.amount_minor,
                        "credits": decision.credits, "state": state, "lago_void_transaction_id": void,
                        "stripe_outcome_withdrawn": withdrawn})).await?;
            }
            Ok::<(), AppError>(())
        }.await;
        match persisted {
            Ok(()) => txn.commit_transaction().await?,
            Err(error) => {
                txn.abort_transaction().await?;
                return Err(error);
            }
        }
    }
    let mut txn = db.client().start_session().await?;
    txn.start_transaction().await?;
    let release = async {
        let latest = rows
            .find_one(doc! {"_id": &order.id})
            .session(&mut txn)
            .await?
            .and_then(|entry| entry.stripe_purchase)
            .ok_or_else(unavailable)?;
        if latest.reversal_review_required
            && latest.reversal_totals_agree
            && !held(&latest.reversals)
        {
            rows.update_one(
                doc! {"_id": &order.id},
                doc! {"$set": {"stripe_purchase.reversal_review_required": false}},
            )
            .session(&mut txn)
            .await?;
            let matched = db
                .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &wallet.id, "lago_wallet_id": &order.lago_wallet_id},
                    doc! {"$pull": {"stripe_enrollment.reversal_holds": &order.id}},
                )
                .session(&mut txn)
                .await?;
            if matched.matched_count != 1 {
                return Err(unavailable());
            }
        }
        Ok(())
    }
    .await;
    match release {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withdrawn_stripe_outcome_keeps_hold_through_pending_receipt_recovery() {
        for (provider_id, status) in [
            ("re_refund", "failed"),
            ("re_refund", "canceled"),
            ("dp_dispute", "won"),
        ] {
            let mut reversal = StripeReversal {
                provider_id: provider_id.into(),
                status: status.into(),
                amount_minor: 1_000,
                finance_correction: None,
                decision: Some(StripeReversalDecision {
                    decision_id: uuid::Uuid::new_v4().to_string(),
                    action: StripeReversalAction::ReverseCredits,
                    credits: 10,
                    reason: "fixture".into(),
                    state: StripeReversalState::Pending,
                    dispatch_started: true,
                    lago_void_transaction_id: None,
                }),
            };
            assert!(held(std::slice::from_ref(&reversal)));
            assert!(!pending_terminal(std::slice::from_ref(&reversal)));
            assert!(pending(std::slice::from_ref(&reversal)));
            reversal.decision.as_mut().unwrap().state = StripeReversalState::Refused;
            assert!(!held(std::slice::from_ref(&reversal)));
            assert!(!requires_finance_correction(&reversal));
            let decision = reversal.decision.as_mut().unwrap();
            decision.state = StripeReversalState::Applied;
            decision.lago_void_transaction_id = Some("fixture-void".into());
            assert!(!held(std::slice::from_ref(&reversal)));
            assert!(requires_finance_correction(&reversal));
        }
    }
}
