//! Bounded metadata for authenticated X ingress. Observation is not delivery.
use crate::models::channel_bot::{COLLECTION_NAME, ChannelBot};
use bson::{Document, doc};

/// Only this closed vocabulary can become a database key. Never pass provider prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    EventNotSelected,
    UnsupportedEvent,
    SelfEvent,
    RecipientMismatch,
    EmptyBatch,
    BotInactive,
    Unregistered,
    ConnectionUnavailable,
    InvalidPayload,
    BillingRefused,
    Duplicate,
    NoRoute,
    RouteUnavailable,
    AuthorityChanged,
    AdmissionRefused,
    StorageUnavailable,
    CallbackKeyUnavailable,
    ReplyTokenUnavailable,
    CallbackPending,
    CallbackAccepted,
    CallbackRefused,
    CallbackFailed,
    EncryptedChatUnsupported,
    NotificationOnly,
    EmptyContent,
    OrgAccessLost,
    OrgAccessUnavailable,
    SenderRefused,
    PolicyRefused,
    OwnerAbsent,
    BudgetRefused,
    Busy,
    Queued,
    TurnStarted,
    ProcessingFailed,
    DeliveryDeferred,
    AgentSilent,
    ReplySent,
    ReplyUnconfirmed,
}

impl Outcome {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::EventNotSelected => "x_event_not_selected",
            Self::UnsupportedEvent => "x_event_unsupported",
            Self::SelfEvent => "x_self_event",
            Self::RecipientMismatch => "x_recipient_mismatch",
            Self::EmptyBatch => "x_empty_batch",
            Self::BotInactive => "x_bot_inactive",
            Self::Unregistered => "x_subscription_unregistered",
            Self::ConnectionUnavailable => "x_connection_unavailable",
            Self::InvalidPayload => "x_payload_invalid",
            Self::BillingRefused => "x_billing_refused",
            Self::Duplicate => "x_inbound_duplicate",
            Self::NoRoute => "x_route_missing",
            Self::RouteUnavailable => "x_route_unavailable",
            Self::AuthorityChanged => "x_authority_changed",
            Self::AdmissionRefused => "x_admission_refused",
            Self::StorageUnavailable => "x_storage_unavailable",
            Self::CallbackKeyUnavailable => "x_callback_key_unavailable",
            Self::ReplyTokenUnavailable => "x_reply_token_unavailable",
            Self::CallbackPending => "x_callback_pending",
            Self::CallbackAccepted => "x_callback_accepted",
            Self::CallbackRefused => "x_callback_refused",
            Self::CallbackFailed => "x_callback_failed",
            Self::EncryptedChatUnsupported => "x_encrypted_chat_unsupported",
            Self::NotificationOnly => "x_notification_only",
            Self::EmptyContent => "x_content_unavailable",
            Self::OrgAccessLost => "x_org_access_lost",
            Self::OrgAccessUnavailable => "x_org_access_unavailable",
            Self::SenderRefused => "x_sender_refused",
            Self::PolicyRefused => "x_chat_policy_refused",
            Self::OwnerAbsent => "x_owner_absent",
            Self::BudgetRefused => "x_bot_budget_refused",
            Self::Busy => "x_turn_busy",
            Self::Queued => "x_turn_queued",
            Self::TurnStarted => "x_turn_started",
            Self::ProcessingFailed => "x_processing_failed",
            Self::DeliveryDeferred => "x_delivery_deferred",
            Self::AgentSilent => "x_agent_silent",
            Self::ReplySent => "x_reply_sent",
            Self::ReplyUnconfirmed => "x_reply_unconfirmed",
        }
    }

    fn updates_latest(self) -> bool {
        // A callback can finish after its asynchronous turn. Nor does a duplicate
        // or an echo prove that an earlier failure has recovered.
        !matches!(
            self,
            Self::CallbackAccepted | Self::Duplicate | Self::SelfEvent
        )
    }

    pub(crate) fn admission(code: &str) -> Self {
        match code {
            "refused_sender" | "refused_self_bot" => Self::SenderRefused,
            "refused_empty_content" => Self::EmptyContent,
            "refused_owner_absent" => Self::OwnerAbsent,
            "refused_bot_budget" => Self::BudgetRefused,
            "refused_busy" => Self::Busy,
            "queued" => Self::Queued,
            "refused_processing_error" => Self::ProcessingFailed,
            _ => Self::PolicyRefused,
        }
    }
}

/// Authentication proves receipt, even when parsing or billing later refuses.
pub(crate) async fn authenticated(db: &mongodb::Database, bot: &ChannelBot) {
    if bot.platform != "x" {
        return;
    }
    if db.collection::<Document>(COLLECTION_NAME).update_one(
        doc! {"_id":&bot.id,"user_id":&bot.user_id},
        doc! {"$set":{"diagnostics.last_inbound_at":bson::DateTime::now(),
            "diagnostics.last_inbound_source":"webhook","diagnostics.last_webhook_at":bson::DateTime::now()}},
    ).await.is_err() {
        tracing::warn!(bot_id=%bot.id, code="x_receipt_diagnostic_unavailable", "X receipt diagnostic write unavailable");
    }
}

pub(crate) async fn checked<T, E>(
    db: &mongodb::Database,
    bot: &ChannelBot,
    outcome: Outcome,
    result: Result<T, E>,
) -> Result<T, E> {
    if result.is_err() {
        record(db, bot, outcome).await;
    }
    result
}

/// Persist the reason even when no ChannelMessage was admitted. Counts are
/// bounded by the vocabulary, not event IDs. Existing workers preserve these
/// additive paths. A lost diagnostic write is logged without database prose.
pub(crate) async fn record(db: &mongodb::Database, bot: &ChannelBot, outcome: Outcome) {
    if bot.platform != "x" {
        return;
    }
    let code = outcome.code();
    let now = bson::DateTime::now();
    let mut set = doc! {format!("diagnostics.inbound_outcomes.{code}.at"): now};
    if outcome.updates_latest() {
        set.insert(
            "diagnostics.last_inbound_outcome",
            doc! {"code":code,"at":now},
        );
    }
    if outcome == Outcome::CallbackFailed {
        // The internal callback may already have stored a more precise refusal.
        set.insert("diagnostics.last_inbound_outcome", doc! {"$cond":[
            {"$in":["$diagnostics.last_inbound_outcome.code",["x_org_access_lost","x_org_access_unavailable","x_authority_changed"]]},
            "$diagnostics.last_inbound_outcome", {"code":code,"at":now}
        ]});
    }
    if outcome == Outcome::ProcessingFailed {
        // The generic async cleanup must not obscure an uncertain send.
        set.insert(
            "diagnostics.last_inbound_outcome",
            doc! {"$cond":[
                {"$eq":["$diagnostics.last_inbound_outcome.code","x_reply_unconfirmed"]},
                "$diagnostics.last_inbound_outcome", {"code":code,"at":now}
            ]},
        );
    }
    set.insert(
        format!("diagnostics.inbound_outcomes.{code}.count"),
        doc! {"$add":[
            {"$ifNull":[format!("$diagnostics.inbound_outcomes.{code}.count"),0_i64]},1_i64
        ]},
    );
    let result = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":&bot.id,"user_id":&bot.user_id},
            vec![doc! {"$set":set}],
        )
        .projection(doc! {"diagnostics.inbound_outcomes":1})
        .await;
    match result {
        Ok(Some(previous))
            if outcome == Outcome::EncryptedChatUnsupported
                && previous
                    .get_document("diagnostics")
                    .ok()
                    .and_then(|d| d.get_document("inbound_outcomes").ok())
                    .is_none_or(|counters| !counters.contains_key(code)) =>
        {
            // One attempt per bot/reason, across replicas. Use the owner's
            // existing notification channels; encrypted X has no reply authority.
            if let Some(advice) = recommendation(code) {
                super::channel_bot_health::notify_failure(db, bot, &advice);
            }
        }
        Err(_) => tracing::warn!(bot_id=%bot.id, code, "X inbound diagnostic write unavailable"),
        _ => {}
    }
}

pub(crate) async fn for_channel(
    db: &mongodb::Database,
    row: &crate::models::nyxbot_channel::NyxbotChannel,
    outcome: Outcome,
) {
    if row.platform != "x" {
        return;
    }
    match super::channel_bot_service::get_bot(db, &row.channel_bot_id).await {
        Ok(bot) => record(db, &bot, outcome).await,
        Err(_) => {
            tracing::warn!(bot_id=%row.channel_bot_id, code=outcome.code(), "X inbound diagnostic bot unavailable")
        }
    }
}

/// Late delivery can outlive the callback; resolve only persisted routing IDs.
pub(crate) async fn for_event(db: &mongodb::Database, event_id: &str, outcome: Outcome) {
    use crate::models::nyxbot_channel::{
        COLLECTION_NAME, EVENTS_COLLECTION_NAME, NyxbotChannel, NyxbotEvent,
    };
    let result = async {
        let Some(event) = db
            .collection::<NyxbotEvent>(EVENTS_COLLECTION_NAME)
            .find_one(doc! {"_id":event_id})
            .await?
        else {
            return Ok::<_, mongodb::error::Error>(());
        };
        if let Some(row) = db
            .collection::<NyxbotChannel>(COLLECTION_NAME)
            .find_one(doc! {"_id":&event.channel_id,"platform":"x"})
            .await?
        {
            let outcome = if outcome == Outcome::DeliveryDeferred
                && event
                    .delivery
                    .as_ref()
                    .is_some_and(|d| matches!(d.state.as_str(), "sending" | "unknown"))
            {
                Outcome::ReplyUnconfirmed
            } else {
                outcome
            };
            for_channel(db, &row, outcome).await;
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        tracing::warn!(
            code = outcome.code(),
            "Channel delivery diagnostic unavailable"
        );
    }
}

pub(crate) const CAPABILITY: &str = "NyxID can read and reply to unencrypted X DMs. Encrypted X Chat is not supported; use an unencrypted DM if X offers it, or continue in NyxID web or another supported channel. A DM-only subscription does not observe encrypted messages.";

pub(crate) fn recommendation(code: &str) -> Option<super::channel_bot_health::Recommendation> {
    let (action, text) = match code {
        "x_encrypted_chat_unsupported" => (
            "contact_admin",
            "An encrypted X Chat event arrived. NyxID cannot read or reply to its content. Use an unencrypted DM if X offers it, or continue in NyxID web or another supported channel. Enabling encrypted-chat notifications only records metadata.",
        ),
        "x_billing_refused" => (
            "enable_billing",
            "The inbound event was refused by billing. Check the org/bot owner's credits, provisioned Lago wallet and subscription, and a synced Requests price on api-twitter's Your own key (BYOK) lane; then select Verify and send a new message.",
        ),
        "x_bot_inactive" => (
            "verify",
            "The authenticated event belongs to a disabled or failed bot. Restore the bot and its connection, select Verify, then send a new message. Old events are not replayed.",
        ),
        "x_subscription_unregistered" | "x_event_not_selected" => (
            "resubscribe",
            "This event was not admitted under the current subscription. Check Events to receive and select Verify, then send a new message. Selecting encrypted chat does not enable reading or replying to it.",
        ),
        "x_connection_unavailable" => (
            "reconnect",
            "The X account connection could not be used. Check connection status and required permissions; reconnect if revoked, then Verify and send a new message.",
        ),
        "x_route_missing" | "x_route_unavailable" | "x_callback_key_unavailable" => (
            "verify",
            "No usable agent route was available for this inbound event. Check the linked agent, active route and its API key, then send a new message.",
        ),
        "x_org_access_lost" | "x_authority_changed" => (
            "verify",
            "Channel authority changed. An organization admin must check the bot's ownership and relink it to an authorized agent before sending a new message.",
        ),
        "x_org_access_unavailable" => (
            "verify",
            "Organization authority could not be checked. Retry with a new message once access/readiness is restored; no authority was bypassed.",
        ),
        "x_callback_refused" => (
            "verify",
            "The linked receiver acknowledged the callback but refused to admit it. Check the message's callback disposition and the receiver's sender, chat and content policies before sending a new message.",
        ),
        "x_callback_failed"
        | "x_processing_failed"
        | "x_reply_token_unavailable"
        | "x_storage_unavailable" => (
            "verify",
            "NyxID could not process or forward this inbound event. Check the linked agent and service health, then send a new message. The event is not automatically replayed.",
        ),
        "x_payload_invalid" | "x_event_unsupported" => (
            "contact_admin",
            "An authenticated X event had an unsupported or invalid shape. An administrator should check the event type and integration version; do not share message bodies or credentials.",
        ),
        "x_content_unavailable" => (
            "contact_admin",
            "The X event contained no readable message content. Send a text-only unencrypted DM if available, or use NyxID web. An empty event alone does not prove encryption.",
        ),
        "x_chat_policy_refused" | "x_owner_absent" | "x_sender_refused" => (
            "verify",
            "The sender or chat policy did not admit this message. Verify the sender account with a fresh owner link and check who may talk to the bot. Display names are not proof of identity.",
        ),
        "x_bot_budget_refused" => (
            "verify",
            "This message exceeded the bot-message budget. Check who may talk to the bot and its automation activity before sending a new message.",
        ),
        "x_turn_busy" => (
            "none",
            "This message was not accepted because the chat was busy. Send a new message after the current work finishes.",
        ),
        "x_admission_refused" => (
            "verify",
            "This inbound event was already claimed or its live-owner admission fence refused it. Check activity and owner status before sending a new message; it was not dispatched again.",
        ),
        "x_callback_pending" | "x_callback_accepted" | "x_turn_started" | "x_turn_queued" => (
            "none",
            "The event reached processing, but this observation does not confirm a reply. Check the linked chat's turn and delivery state.",
        ),
        "x_delivery_deferred" => (
            "none",
            "Reply delivery is waiting for a local dependency. NyxID may retry preparation; no send is confirmed. Check the chat's delivery state before sending the request again.",
        ),
        "x_reply_unconfirmed" => (
            "none",
            "The reply could not be confirmed. Check the X conversation before sending another message; NyxID will not resend an uncertain delivery.",
        ),
        "x_agent_silent" => (
            "none",
            "The agent completed without a reply. Check its chat and instructions; no platform delivery was claimed.",
        ),
        "x_notification_only" => (
            "none",
            "This event is a metadata notification; no automatic reply is supported.",
        ),
        "x_empty_batch" | "x_self_event" | "x_recipient_mismatch" | "x_inbound_duplicate" => (
            "none",
            "The authenticated event had no new eligible incoming message. See the inbound outcome counters for the filter or duplicate reason.",
        ),
        "x_reply_sent" => (
            "none",
            "X accepted the reply. This does not prove that the recipient read it. Encrypted X Chat remains unsupported.",
        ),
        _ => return None,
    };
    Some(super::channel_bot_health::Recommendation {
        action,
        text: text.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{channel_bot_health, channel_bot_service, channel_x_tests};

    #[tokio::test]
    async fn x_2043_outcomes_survive_liveness_and_round_trip_through_both_health_views() {
        use Outcome::*;
        let (state, _, _, owner, connection) = channel_x_tests::fixture().await;
        let bot = channel_x_tests::insert_bot(&state, &owner, &connection).await;
        let outcomes = [
            EventNotSelected,
            UnsupportedEvent,
            SelfEvent,
            RecipientMismatch,
            EmptyBatch,
            BotInactive,
            Unregistered,
            ConnectionUnavailable,
            InvalidPayload,
            BillingRefused,
            Duplicate,
            NoRoute,
            RouteUnavailable,
            AuthorityChanged,
            AdmissionRefused,
            StorageUnavailable,
            CallbackKeyUnavailable,
            ReplyTokenUnavailable,
            CallbackPending,
            CallbackAccepted,
            CallbackRefused,
            CallbackFailed,
            EncryptedChatUnsupported,
            NotificationOnly,
            EmptyContent,
            OrgAccessLost,
            OrgAccessUnavailable,
            SenderRefused,
            PolicyRefused,
            OwnerAbsent,
            BudgetRefused,
            Busy,
            Queued,
            TurnStarted,
            ProcessingFailed,
            DeliveryDeferred,
            AgentSilent,
            ReplySent,
            ReplyUnconfirmed,
        ];
        for outcome in outcomes {
            record(&state.db, &bot, outcome).await;
            let current = channel_bot_service::get_bot(&state.db, &bot.id)
                .await
                .unwrap();
            let code = outcome.code();
            let counter = &current.diagnostics.inbound_outcomes[code];
            assert_eq!(counter.count, 1, "{code}");
            assert!(counter.at <= chrono::Utc::now());
            let evidence = channel_bot_health::setup_evidence(&state.db, &current, false)
                .await
                .unwrap();
            let http = crate::handlers::channel_bots::health_for(&current, false, true);
            assert_eq!(evidence["inbound_outcomes"][code]["count"], 1);
            assert_eq!(http.diagnostics.inbound_outcomes[code].count, 1);
            assert_eq!(
                http.recommended_action_text,
                evidence["recommended_action_text"]
            );
            assert_ne!(http.recommended_action_text, "Healthy.");
            let advice = recommendation(code).unwrap();
            assert!(!advice.text.is_empty());
            // Actions remain compatible with existing clients.
            assert!(matches!(
                advice.action,
                "none"
                    | "verify"
                    | "resubscribe"
                    | "reconnect"
                    | "contact_admin"
                    | "enable_billing"
            ));
        }
        channel_bot_service::record_inbound_success(
            &state.db,
            &bot,
            crate::services::channel_inbound_service::InboundSource::Webhook,
        )
        .await;
        record(&state.db, &bot, ProcessingFailed).await;
        record(&state.db, &bot, Duplicate).await;
        record(&state.db, &bot, CallbackAccepted).await;
        record(&state.db, &bot, SelfEvent).await;
        let current = channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert_eq!(
            current.diagnostics.last_inbound_outcome.unwrap().code,
            ReplyUnconfirmed.code()
        );
        assert_eq!(current.diagnostics.inbound_outcomes.len(), outcomes.len());
        assert_eq!(
            current.diagnostics.inbound_outcomes[Duplicate.code()].count,
            2
        );
        // A callback's generic HTTP failure must retain the actual org refusal.
        for outcome in [OrgAccessLost, OrgAccessUnavailable, AuthorityChanged] {
            record(&state.db, &bot, outcome).await;
            record(&state.db, &bot, CallbackFailed).await;
            let current = channel_bot_service::get_bot(&state.db, &bot.id)
                .await
                .unwrap();
            assert_eq!(
                current.diagnostics.last_inbound_outcome.unwrap().code,
                outcome.code()
            );
        }
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn x_2043_concurrent_encrypted_observations_bound_the_owner_notice_and_ignore_other_platforms()
     {
        let (state, _, _, owner, connection) = channel_x_tests::fixture().await;
        let bot = channel_x_tests::insert_bot(&state, &owner, &connection).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        channel_bot_health::TEST_NOTICES
            .lock()
            .unwrap()
            .insert(bot.id.clone(), tx);
        let observations =
            (0..16).map(|_| record(&state.db, &bot, Outcome::EncryptedChatUnsupported));
        futures::future::join_all(observations).await;
        let notice = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(notice.0, owner);
        assert!(
            notice
                .1
                .recommended_action_text
                .contains("cannot read or reply")
        );
        assert!(rx.try_recv().is_err());
        channel_bot_health::TEST_NOTICES
            .lock()
            .unwrap()
            .remove(&bot.id);
        let current = channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert_eq!(
            current.diagnostics.inbound_outcomes["x_encrypted_chat_unsupported"].count,
            16
        );
        let before = bson::to_document(&current.diagnostics).unwrap();
        let mut non_x = bot;
        non_x.platform = "telegram".into();
        record(&state.db, &non_x, Outcome::ReplySent).await;
        authenticated(&state.db, &non_x).await;
        let current = channel_bot_service::get_bot(&state.db, &non_x.id)
            .await
            .unwrap();
        assert_eq!(bson::to_document(&current.diagnostics).unwrap(), before);
        state.db.drop().await.unwrap();
    }
}
