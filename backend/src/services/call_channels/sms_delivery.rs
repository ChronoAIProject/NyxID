//! Authenticated receipt evidence survives send/receipt ordering and revocation.
use super::{billing, session};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_adapter::*,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;

pub struct Receipt<'a> {
    pub event: &'a str,
    pub message: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub status: &'a str,
    pub parts: Option<i64>,
    pub send: Option<&'a str>,
}

pub async fn record_correlated(
    state: &AppState,
    b: &Binding,
    evidence: Receipt<'_>,
) -> AppResult<()> {
    let Receipt {
        event,
        message,
        from,
        to,
        status,
        parts,
        send,
    } = evidence;
    if (!from.is_empty() && from != b.phone_number)
        || (!to.is_empty() && !super::protocol::phone(to))
    {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "SMS resource mismatch".into(),
        ));
    }
    let mut row = EventReceipt {
        managed_claim_id: None,
        managed_claim_generation: None,
        admitted_reference: None,
        id: uuid::Uuid::new_v4().to_string(),
        identity_key: session::identity(&[&b.id, "sms_receipt", event]),
        user_id: b.user_id.clone(),
        binding_id: b.id.clone(),
        binding_revision: b.revision,
        session_id: None,
        event_kind: "sms_receipt".into(),
        status: status.into(),
        provider_message_id: Some(message.into()),
        delivery: delivery(status).into(),
        billing_request_id: None,
        observed_parts: parts,
        reserved_parts: None,
        billing_finalized: false,
        from: (!from.is_empty()).then(|| from.into()),
        to: (!to.is_empty()).then(|| to.into()),
        created_at: Utc::now(),
    };
    if matches!(b.credential, CredentialBinding::ManagedLine { .. }) {
        let mut filter = doc! {"binding_id":&b.id,"user_id":&b.user_id,"event_kind":"sms_send"};
        if let Some(send) = send {
            filter.insert("_id", send);
        } else {
            filter.insert("provider_message_id", message);
        }
        let sent = state
            .db
            .collection::<EventReceipt>(EVENTS)
            .find_one(filter)
            .await?
            .ok_or(AppError::AssistantTurnRequired)?;
        row.managed_claim_id = sent.managed_claim_id;
        row.managed_claim_generation = sent.managed_claim_generation;
        row.admitted_reference = sent.admitted_reference;
    }
    let db = state.db.clone();
    let binding = b.clone();
    let correlation = send.map(str::to_owned);
    let message_id = message.to_owned();
    let sender = from.to_owned();
    let recipient = to.to_owned();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction().and_run2(async move |tx| {
        let result: AppResult<()> = async {
            super::binding::retain_receipt(&db, tx, &binding).await?;
            let rows = db.collection::<EventReceipt>(EVENTS);
            if let Some(existing) = rows.find_one(doc! {"identity_key":&row.identity_key}).session(&mut *tx).await? {
                if existing.provider_message_id != row.provider_message_id || existing.from != row.from || existing.to != row.to {
                    return Err(AppError::AssistantTurnRequired);
                }
            } else {
                rows.insert_one(&row).session(&mut *tx).await?;
            }
            if let Some(send) = &correlation {
                let target = rows.find_one(doc! {"_id":send,"binding_id":&binding.id,"user_id":&binding.user_id,"event_kind":"sms_send"}).session(&mut *tx).await?.ok_or(AppError::AssistantTurnRequired)?;
                if target.provider_message_id.as_deref().is_some_and(|id| id != message_id)
                    || (!sender.is_empty() && target.from.as_deref() != Some(sender.as_str()))
                    || (!recipient.is_empty() && target.to.as_deref() != Some(recipient.as_str())) {
                    return Err(AppError::AssistantTurnRequired);
                }
                rows.update_one(doc! {"_id":send,"provider_message_id":mongodb::bson::Bson::Null}, doc! {"$set":{"provider_message_id":&message_id,"status":"receipt_confirmed"}}).session(&mut *tx).await?;
            }
            Ok(())
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    if let Some(sent) = state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"binding_id":&b.id,"event_kind":"sms_send","provider_message_id":message})
        .await?
    {
        reconcile(state, &sent.id).await?;
    }
    Ok(())
}
fn delivery(status: &str) -> &'static str {
    match status {
        "delivered" | "delivery_confirmed" => "delivered",
        "failed" | "undelivered" | "delivery_failed" => "failed",
        _ => "accepted",
    }
}
pub async fn reconcile(state: &AppState, id: &str) -> AppResult<()> {
    let rows = state.db.collection::<EventReceipt>(EVENTS);
    let Some(sent) = rows
        .find_one(doc! {"_id":id,"event_kind":"sms_send"})
        .await?
    else {
        return Ok(());
    };
    if let (Some(message), Some(reference)) = (&sent.provider_message_id, &sent.admitted_reference)
    {
        use crate::models::managed_comms::*;
        let mut reference = reference.clone();
        reference.provider_message_id = Some(message.clone());
        state.db.collection::<EventReceipt>(EVENTS).update_one(doc!{"_id":&sent.id},doc!{"$set":{"admitted_reference":mongodb::bson::to_bson(&reference).map_err(|_|AppError::AssistantTurnRequired)?}}).await?;
        let route = ManagedEffectRoute {
            id: uuid::Uuid::new_v4().to_string(),
            trusted_account_id: reference.trusted_account_id.clone(),
            provider: reference.provider.clone(),
            provider_effect_id: message.clone(),
            line_id: reference.line_id.clone(),
            managed_claim_id: sent.managed_claim_id.clone(),
            managed_claim_generation: sent.managed_claim_generation,
            binding_id: Some(sent.binding_id.clone()),
            effect_id: sent.id.clone(),
            kind: EffectKind::Sms,
            financially_finalized: sent.billing_finalized,
            admitted: reference,
            covered_from: sent.created_at,
            covered_until: None,
            created_at: Utc::now(),
        };
        state.db.collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME).update_one(doc!{"trusted_account_id":&route.trusted_account_id,"provider":&route.provider,"provider_effect_id":message},doc!{"$setOnInsert":mongodb::bson::to_document(&route).map_err(|_|AppError::AssistantTurnRequired)?}).upsert(true).await?;
    }
    let evidence: Vec<EventReceipt> = if let Some(message) = sent.provider_message_id.as_ref() {
        rows.find(doc!{"binding_id":&sent.binding_id,"event_kind":"sms_receipt","provider_message_id":message,"$and":[{"$or":[{"from":&sent.from},{"from":mongodb::bson::Bson::Null}]},{"$or":[{"to":&sent.to},{"to":mongodb::bson::Bson::Null}]}]}).sort(doc!{"created_at":1}).limit(32).await?.try_collect().await?
    } else {
        Vec::new()
    };
    // Prefer terminal evidence. A later queued receipt cannot overwrite delivery.
    let terminal = evidence
        .iter()
        .find(|r| matches!(r.delivery.as_str(), "delivered" | "failed"));
    if let Some(evidence) = terminal {
        rows.update_one(
            doc! {"_id":id,"delivery":{"$nin":["delivered","failed"]}},
            doc! {"$set":{"delivery":&evidence.delivery}},
        )
        .await?;
    }
    if let Some(sid) = sent.session_id.as_deref()
        && let Ok(session) = session::load(state, sid).await
        && let Some(turn) = session.reply_turn_id.as_deref()
    {
        let current = rows
            .find_one(doc! {"_id":id})
            .await?
            .ok_or(AppError::AssistantTurnRequired)?;
        let metadata = crate::models::assistant_message::ChannelDelivery {
            status: current.delivery,
            heard_prefix_chars: None,
            heard_prefix_matches_sent: false,
        };
        state.db.collection::<mongodb::bson::Document>(crate::models::assistant_message::COLLECTION_NAME).update_many(doc!{"conversation_id":&session.conversation_id,"turn_id":turn,"role":"assistant"},doc!{"$set":{"turn_metadata.channel_delivery":mongodb::bson::to_bson(&metadata).map_err(|_|AppError::AssistantTurnRequired)?}}).await?;
    }
    if sent.billing_finalized {
        if sent.admitted_reference.is_some() {
            super::managed::sms::recognize_send(state, &sent).await?;
        }
        return Ok(());
    }
    // A part waived for missing provider evidence never settles on later
    // evidence: it only finishes its zero settlement.
    if super::managed::reservations::waived(&state.db, &sent.id).await? {
        return super::managed::reservations::finish_waived(state, &sent).await;
    }
    let mut actual = terminal
        .and_then(|r| r.observed_parts)
        .or(sent.observed_parts)
        .or_else(|| evidence.iter().find_map(|r| r.observed_parts));
    let binding = super::binding::load(state, &sent.binding_id).await?;
    if sent.observed_parts.is_none()
        && binding.profile == Profile::TelnyxSms
        && matches!(binding.credential, CredentialBinding::Connection { .. })
        && sent.status != "rejected"
    {
        let r = super::binding::resolve(
            state,
            &binding.user_id,
            &binding.credential,
            binding.profile,
        )
        .await?;
        actual = Some(
            super::carrier::message_parts(
                state,
                &binding,
                &r,
                sent.provider_message_id
                    .as_deref()
                    .ok_or(AppError::AssistantTurnRequired)?,
                sent.from
                    .as_deref()
                    .ok_or(AppError::AssistantTurnRequired)?,
                sent.to.as_deref().ok_or(AppError::AssistantTurnRequired)?,
                false,
            )
            .await?,
        );
    }
    if actual.is_none() {
        let b = super::binding::load(state, &sent.binding_id).await?;
        if b.profile == Profile::TwilioSms {
            let message = sent
                .provider_message_id
                .as_deref()
                .ok_or(AppError::AssistantTurnRequired)?;
            if !super::protocol::handle(message) {
                return Err(AppError::AssistantTurnRequired);
            }
            let value = if matches!(b.credential, CredentialBinding::ManagedLine { .. }) {
                // Read the exact message just correlated, not the stored
                // reference captured before its provider ID was known.
                let mut reference = sent
                    .admitted_reference
                    .clone()
                    .ok_or(AppError::AssistantTurnRequired)?;
                reference.provider_message_id = Some(message.to_owned());
                super::managed::resolver::request_existing(
                    state,
                    &b,
                    &reference,
                    super::managed::resolver::CleanupOperation::ReadMessage,
                    Some(&super::profiles::binding_origin(&b)?),
                )
                .await?
            } else {
                let r =
                    super::binding::resolve(state, &b.user_id, &b.credential, b.profile).await?;
                super::carrier::request(
                    state,
                    &b,
                    &r,
                    "GET",
                    &format!(
                        "/2010-04-01/Accounts/{}/Messages/{message}.json",
                        b.account_id
                    ),
                    None,
                )
                .await?
            };
            if value["sid"] != message
                || value["account_sid"] != b.account_id
                || value["from"] != sent.from.as_deref().unwrap_or_default()
                || value["to"] != sent.to.as_deref().unwrap_or_default()
            {
                return Err(AppError::AssistantTurnRequired);
            }
            actual = value["num_segments"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|s| (1..=16).contains(s));
            if let Some(status) = value["status"]
                .as_str()
                .filter(|s| matches!(*s, "delivered" | "failed" | "undelivered"))
            {
                rows.update_one(
                    doc! {"_id":id,"delivery":{"$nin":["delivered","failed"]}},
                    doc! {"$set":{"delivery":delivery(status)}},
                )
                .await?;
                project_delivery(state, &sent).await?;
            }
        } else if b.profile == Profile::TelnyxSms
            && matches!(b.credential, CredentialBinding::ManagedLine { .. })
        {
            let message = sent
                .provider_message_id
                .as_deref()
                .filter(|m| super::protocol::handle(m))
                .ok_or(AppError::AssistantTurnRequired)?;
            let mut reference = sent
                .admitted_reference
                .clone()
                .ok_or(AppError::AssistantTurnRequired)?;
            reference.provider_message_id = Some(message.to_owned());
            let value = super::managed::resolver::read_telnyx_message(
                state,
                &b,
                &reference,
                Some(&super::profiles::binding_origin(&b)?),
            )
            .await?;
            // The same identity, part and per-recipient status fields the
            // signed `message.finalized` webhook carries.
            let v = &value["data"];
            let to = v["to"]
                .as_array()
                .filter(|to| to.len() == 1)
                .map(|to| &to[0]);
            if v["id"] != message
                || v["messaging_profile_id"] != b.application_id
                || v["direction"] != "outbound"
                || v.pointer("/from/phone_number")
                    .and_then(serde_json::Value::as_str)
                    != sent.from.as_deref()
                || to.and_then(|to| to["phone_number"].as_str()) != sent.to.as_deref()
            {
                return Err(AppError::AssistantTurnRequired);
            }
            actual = v["parts"].as_i64().filter(|n| (1..=16).contains(n));
            if let Some(status) = to
                .and_then(|to| to["status"].as_str())
                .filter(|s| delivery(s) != "accepted")
            {
                rows.update_one(
                    doc! {"_id":id,"delivery":{"$nin":["delivered","failed"]}},
                    doc! {"$set":{"delivery":delivery(status)}},
                )
                .await?;
                project_delivery(state, &sent).await?;
            }
        }
    }
    let (Some(actual), Some(reserved), Some(_sid)) =
        (actual, sent.reserved_parts, sent.session_id.as_ref())
    else {
        return Ok(());
    };
    if actual < 0 || actual > reserved || (actual == 0 && sent.status != "rejected") {
        return Err(AppError::Conflict(
            "Provider SMS parts exceeded the admitted bound; reconcile manually".into(),
        ));
    }
    // Freeze the exact part count before any money write. All retries reuse it.
    // The freeze and an evidence waiver exclude each other on this row.
    rows.update_one(
        doc! {"_id":id,"observed_parts":mongodb::bson::Bson::Null,"terminal_evidence":{"$exists":false}},
        doc! {"$set":{"observed_parts":actual}},
    )
    .await?;
    if super::managed::reservations::waived(&state.db, id).await? {
        return super::managed::reservations::finish_waived(state, &sent).await;
    }
    let frozen = rows
        .find_one(doc! {"_id":id})
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let actual = frozen
        .observed_parts
        .ok_or(AppError::AssistantTurnRequired)?;
    for part in 0..reserved {
        billing::settle_admitted(
            state,
            &outbound_request_id(&sent, part)?,
            &sent.user_id,
            0,
            i64::from(part < actual),
        )
        .await?;
    }
    if sent.admitted_reference.is_some() {
        super::managed::sms::recognize_send(state, &frozen).await?;
    }
    rows.update_one(doc! {"_id":id}, doc! {"$set":{"billing_finalized":true}})
        .await?;
    if sent.admitted_reference.is_some() {
        use crate::models::managed_comms::ManagedEffectRoute;
        state
            .db
            .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
            .update_one(
                doc! {"effect_id":id},
                doc! {"$set":{"financially_finalized":true}},
            )
            .await?;
    }
    Ok(())
}
async fn project_delivery(state: &AppState, sent: &EventReceipt) -> AppResult<()> {
    let Some(sid) = sent.session_id.as_deref() else {
        return Ok(());
    };
    let session = session::load(state, sid).await?;
    let Some(turn) = session.reply_turn_id.as_deref() else {
        return Ok(());
    };
    let current = state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id":&sent.id})
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let metadata = crate::models::assistant_message::ChannelDelivery {
        status: current.delivery,
        heard_prefix_chars: None,
        heard_prefix_matches_sent: false,
    };
    state.db.collection::<mongodb::bson::Document>(crate::models::assistant_message::COLLECTION_NAME).update_many(doc!{"conversation_id":&session.conversation_id,"turn_id":turn,"role":"assistant"},doc!{"$set":{"turn_metadata.channel_delivery":mongodb::bson::to_bson(&metadata).map_err(|_|AppError::AssistantTurnRequired)?}}).await?;
    Ok(())
}
pub async fn sweep(state: &AppState) -> AppResult<()> {
    let rows:Vec<EventReceipt>=state.db.collection::<EventReceipt>(EVENTS).find(doc!{"event_kind":"sms_send","$or":[{"billing_finalized":false,"provider_message_id":{"$ne":mongodb::bson::Bson::Null}},{"billing_finalized":false,"status":"rejected"},{"status":"rejected_before_dispatch","managed_spend_hold_id":{"$type":"string"},"managed_spend_recognized":{"$ne":true}}]}).sort(doc!{"recovery_checked_at":1,"_id":1}).limit(100).await?.try_collect().await?;
    for row in rows {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        state
            .db
            .collection::<mongodb::bson::Document>(EVENTS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"recovery_checked_at":mongodb::bson::DateTime::now()}},
            )
            .await?;
        let _ = reconcile(state, &row.id).await;
    }
    Ok(())
}

/// One persisted prefix carries owner-funded replies, managed-line replies and
/// call-confirmation sends through the same receipt/funding recovery path.
pub(crate) fn outbound_request_id(row: &EventReceipt, part: i64) -> AppResult<String> {
    let prefix = if let Some(first) = &row.billing_request_id {
        first
            .strip_suffix(":0")
            .ok_or(AppError::AssistantTurnRequired)?
            .to_owned()
    } else {
        // Legacy ordinary SMS receipts derived their billing identity from the
        // session. Preserve settlement after revocation and on retained rows.
        format!(
            "adapter-sms-out:{}",
            row.session_id
                .as_deref()
                .ok_or(AppError::AssistantTurnRequired)?
        )
    };
    if !(prefix.starts_with("adapter-sms-out:")
        || prefix.starts_with("managed-sms-out:")
        || prefix.starts_with("call-confirmation:"))
        || !(0..16).contains(&part)
    {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(format!("{prefix}:{part}"))
}
