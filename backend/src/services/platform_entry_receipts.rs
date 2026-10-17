//! Adapter-authenticated receipts retain metadata and a keyed content commitment.
//! Both direct delivery and a selected gateway redeem the same native event.
use super::{
    platform_entry_identity::{content_commitment, opaque, refused, verify_content},
    platform_entry_native::AuthenticatedNativeBatch,
};
use crate::{
    errors::AppResult,
    models::platform_entry_identity::{
        EntryTransport, NATIVE_RECEIPTS, NativeEntryConfiguration, NativeEntryReceipt,
    },
};
use bson::{Bson, Document, doc};
use chrono::{Duration, Utc};
use mongodb::{ClientSession, Database};

/// This type cannot be deserialized from normalized gateway activity.
/// Construction below checks the retained native receipt and exact text.
pub(crate) struct NativeAdmission {
    receipt: NativeEntryReceipt,
    text: zeroize::Zeroizing<String>,
}
impl NativeAdmission {
    pub(crate) fn receipt(&self) -> &NativeEntryReceipt {
        &self.receipt
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn verified_update(
        &self,
    ) -> super::platform_manager_contract::VerifiedManagerUpdate {
        use super::platform_manager_contract::*;
        VerifiedManagerUpdate {
            profile_id: self.receipt.profile_id.clone(),
            manager_bot_id: self.receipt.configuration.identity.platform_bot_id.clone(),
            entry: EntryIdentity::NativeV2 {
                platform_entry: self.receipt.configuration.identity.clone(),
            },
            observation_id: self.receipt.id.clone(),
            sender: SenderIdentity::NativeV2 {
                platform_sender: self.receipt.sender.clone(),
            },
            chat_kind: ManagerChatKind::Private,
            event: EventIdentity::NativeV2 {
                platform_event_id: self.receipt.event_id.clone(),
                platform_message_id: self.receipt.message_id.clone(),
            },
            command_eligibility: ManagerCommandEligibility::OriginalPrivateMessage,
        }
    }
}

pub(super) async fn fence_configuration(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
    epoch: i64,
    configuration: &NativeEntryConfiguration,
) -> AppResult<()> {
    if epoch <= 0 || !super::platform_entry_migration::native_writes_ready(db).await? {
        return Err(refused());
    }
    let identity = bson::to_bson(&configuration.identity).map_err(|_| refused())?;
    let configured = bson::to_bson(configuration).map_err(|_| refused())?;
    let profile=db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
        .update_one(doc! {"_id":profile_id,"identity_version":2,"channel_bot_id":&configuration.identity.channel_bot_id,
            "manager_bot_id":&configuration.identity.platform_bot_id,"native_entry":configured,"deleted_at":Bson::Null},
            doc! {"$inc":{"native_effect_revision":1_i64}}).session(&mut *session).await?;
    if profile.matched_count != 1 {
        return Err(refused());
    }
    let controls = db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":profile_id,"contract_version":2,"platform_entry":identity,
            "manager_authority_epoch":epoch,"retired":false},
            doc! {"$inc":{"effect_revision":1_i64}},
        )
        .session(&mut *session)
        .await?;
    if controls.matched_count != 1 {
        return Err(refused());
    }
    // This independent transport marker survives ordinary NyxBot channel cleanup.
    let bot=db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(doc! {"_id":&configuration.identity.channel_bot_id,"platform_profile_id":profile_id,
            "platform_entry_version":2,"platform_bot_id":&configuration.identity.platform_bot_id,"is_active":true,"status":"active","platform_entry_pending":{"$exists":false}},
            doc! {"$inc":{"platform_entry_effect_revision":1_i64}}).session(&mut *session).await?;
    if bot.matched_count != 1 {
        return Err(refused());
    }
    super::platform_entry_gateway_service::fence_selected_transport(
        db,
        session,
        profile_id,
        configuration,
    )
    .await?;
    Ok(())
}

pub(crate) struct NativeReceiptInput<'a> {
    pub(crate) profile_id: &'a str,
    pub(crate) epoch: i64,
    pub(crate) configuration: &'a NativeEntryConfiguration,
    pub(crate) batch: &'a AuthenticatedNativeBatch,
    pub(crate) index: usize,
    pub(crate) relay_message_id: &'a str,
}

pub(crate) async fn record_in_session(
    db: &Database,
    session: &mut ClientSession,
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    key: &[u8],
    input: NativeReceiptInput<'_>,
) -> AppResult<NativeAdmission> {
    let NativeReceiptInput {
        profile_id,
        epoch,
        configuration,
        batch,
        index,
        relay_message_id,
    } = input;
    // The authenticated parser fixes both entry and transport. A caller cannot
    // relabel its facts using another independently live configuration.
    if batch.configuration() != configuration {
        return Err(refused());
    }
    let native = batch.fact(index).ok_or_else(refused)?;
    opaque(relay_message_id)?;
    fence_configuration(db, session, profile_id, epoch, configuration).await?;
    if !batch.fence_credentials(db, session).await? {
        return Err(refused());
    }
    let now = Utc::now();
    let mut receipt = NativeEntryReceipt {
        id: relay_message_id.into(),
        profile_id: profile_id.into(),
        manager_authority_epoch: epoch,
        configuration: batch.configuration().clone(),
        sender: native.sender.clone(),
        event_id: native.event_id.clone(),
        message_id: native.message_id.clone(),
        original_private_human: true,
        content_commitment: String::new(),
        received_at: now,
        expires_at: now + Duration::minutes(10),
    };
    receipt.content_commitment = content_commitment(key, &receipt, native.text())?;
    let rows = db.collection::<NativeEntryReceipt>(NATIVE_RECEIPTS);
    let subject = doc! {"profile_id":profile_id,"manager_authority_epoch":epoch,
    "configuration.identity":bson::to_bson(&configuration.identity).map_err(|_|refused())?,
    "sender.sender_id":&native.sender.sender_id,"event_id":&native.event_id};
    if let Some(previous) = rows.find_one(subject).session(&mut *session).await? {
        // An event retry retains its original destination/body/transport and
        // cannot extend receipt expiry or reclassify audience after login/logout.
        if previous.configuration != *configuration
            || previous.sender != native.sender
            || previous.message_id != native.message_id
            || previous.expires_at <= now
        {
            return Err(refused());
        }
        verify_content(key, &previous, native.text())?;
        return Ok(NativeAdmission {
            receipt: previous,
            text: zeroize::Zeroizing::new(native.text().to_owned()),
        });
    }
    let reply_handle = batch
        .seal_reply_handle(encryption_keys, &receipt, index)
        .await?;
    rows.insert_one(&receipt).session(&mut *session).await?;
    if let Some(handle) = reply_handle {
        db.collection::<crate::models::platform_native_reply_handle::NativeReplyHandle>(
            crate::models::platform_native_reply_handle::COLLECTION_NAME,
        )
        .insert_one(handle)
        .session(&mut *session)
        .await?;
    }
    Ok(NativeAdmission {
        receipt,
        text: zeroize::Zeroizing::new(native.text().to_owned()),
    })
}

pub(crate) struct GatewayReceiptInput<'a> {
    pub(crate) profile_id: &'a str,
    pub(crate) configuration: &'a NativeEntryConfiguration,
    pub(crate) transport_channel_id: &'a str,
    pub(crate) activity: &'a serde_json::Value,
    pub(crate) text: &'a str,
}

pub(crate) async fn verify_gateway_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    cache: &std::sync::Arc<super::provider_token_exchange_service::TokenExchangeCache>,
    input: GatewayReceiptInput<'_>,
) -> AppResult<NativeAdmission> {
    let GatewayReceiptInput {
        profile_id,
        configuration,
        transport_channel_id,
        activity,
        text,
    } = input;
    let EntryTransport::NyxagentGateway { channel_id, .. } = &configuration.transport else {
        return Err(refused());
    };
    if channel_id != transport_channel_id
        || activity["source"]["type"] != "nyxid_relay"
        || activity["conversation"]["kind"] != "private"
        || activity["actor"]["kind"] != "unknown"
        || activity["schema_version"] != 1
        || activity["source"]["platform"]
            != super::platform_entry_native::platform_name(configuration.identity.platform)
        || activity["kind"]["type"] != "message"
        || activity["kind"]["text"] != text
        || activity["kind"]["mentions_bot"] != false
        || activity["kind"]["attachments"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
        || !activity["kind"]["reply_to"].is_null()
        || !activity["kind"]["album_id"].is_null()
        || !activity["conversation"]["thread_id"].is_null()
        || !activity["routing"].is_null()
    {
        return Err(refused());
    }
    super::platform_entry_gateway_service::require_supported_platform(
        configuration.identity.platform,
    )?;
    let event_id = activity["event_id"].as_str().ok_or_else(refused)?;
    opaque(event_id)?;
    let receipt = db
        .collection::<NativeEntryReceipt>(NATIVE_RECEIPTS)
        .find_one(doc! {"_id":event_id,"profile_id":profile_id})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    let now = Utc::now();
    if receipt.configuration != *configuration
        || !receipt.original_private_human
        || receipt.received_at > now
        || receipt.expires_at <= now
        || receipt.expires_at > receipt.received_at + Duration::minutes(10)
        || activity["actor"]["id"] != receipt.sender.sender_id
        || activity["conversation"]["id"] != receipt.sender.private_conversation_id
    {
        return Err(refused());
    }
    // The commitment, rather than a normalized actor's claimed kind, binds the
    // exact authenticated command/code or ordinary question before any effect.
    verify_content(key, &receipt, text)?;
    fence_configuration(
        db,
        session,
        profile_id,
        receipt.manager_authority_epoch,
        configuration,
    )
    .await?;
    // Gateway redemption also resolves and fences the current real signing and
    // owner-bound connection sources; retained evidence cannot outlive rotation.
    let bot =
        super::channel_bot_service::get_bot(db, &configuration.identity.channel_bot_id).await?;
    let adapter = super::channel_adapters::resolve_adapter(&bot.platform, cache)?;
    let snapshot = super::platform_entry_credentials::EntryCredentialSnapshot::load(
        db,
        adapter.as_ref(),
        &bot,
    )
    .await?;
    if snapshot.fingerprint()? != configuration.credential_fingerprint
        || !snapshot.fence_in_session(db, session).await?
    {
        return Err(refused());
    }
    Ok(NativeAdmission {
        receipt,
        text: zeroize::Zeroizing::new(text.to_owned()),
    })
}
