//! Receipt-scoped native delivery. Provider reply capabilities are decrypted
//! only after the ordinary delivery transaction has claimed its single send.
use super::{
    channel_platform::{BotCredentials, OutboundReply, PlatformAdapter},
    platform_entry_credentials::EntryCredentialSnapshot,
    platform_entry_identity::{opaque, refused, validate_configuration},
};
use crate::{
    AppState,
    errors::AppResult,
    models::{
        channel_bot::ChannelBot,
        platform_entry_identity::{EntryPlatform, NativeControlSurface, NativeEntryReceipt},
        platform_entry_projection::{
            StoredEntryIdentity, StoredEventIdentity, StoredSenderIdentity,
        },
        platform_manager_delivery::{COLLECTION_NAME as DELIVERIES, PlatformManagerDelivery},
        platform_native_reply_handle::{COLLECTION_NAME as HANDLES, NativeReplyHandle},
    },
};
use chrono::{DateTime, Duration, Utc};
use mongodb::{ClientSession, bson::doc};
use serde::Deserialize;
use zeroize::{Zeroize, Zeroizing};

pub(crate) fn binding_digest(receipt: &NativeEntryReceipt) -> AppResult<String> {
    validate_configuration(&receipt.configuration)?;
    opaque(&receipt.sender.sender_id)?;
    opaque(&receipt.sender.private_conversation_id)?;
    opaque(&receipt.event_id)?;
    opaque(&receipt.message_id)?;
    if !uuid::Uuid::parse_str(&receipt.id).is_ok_and(|id| id.get_version_num() == 4)
        || receipt.manager_authority_epoch <= 0
        || !receipt.original_private_human
        || receipt.expires_at <= receipt.received_at
        || receipt.expires_at > receipt.received_at + Duration::minutes(10)
    {
        return Err(refused());
    }
    Ok(super::mcp_service::canonical_sha256(serde_json::json!([
        "nyxid-native-reply-handle:v2",
        receipt.id,
        receipt.profile_id,
        receipt.manager_authority_epoch,
        receipt.configuration,
        receipt.sender,
        receipt.event_id,
        receipt.message_id,
        receipt.content_commitment,
        receipt.received_at.timestamp_millis(),
        receipt.expires_at.timestamp_millis()
    ])))
}

fn check_handle(handle: &NativeReplyHandle, receipt: &NativeEntryReceipt) -> AppResult<()> {
    if !uuid::Uuid::parse_str(&handle.id).is_ok_and(|id| id.get_version_num() == 4)
        || handle.receipt_id != receipt.id
        || handle.binding_digest != binding_digest(receipt)?
        || handle.created_at < receipt.received_at
        || handle.created_at > Utc::now()
        || handle.expires_at <= Utc::now()
        || handle.expires_at > receipt.expires_at
        || handle.ciphertext.is_empty()
        || handle.ciphertext.len() > 16 * 1024
    {
        return Err(refused());
    }
    Ok(())
}

/// Ciphertext, live credential snapshot and destination captured by the send
/// transaction. This is neither deserializable nor a generic thread-ID proof.
pub(crate) struct NativeSendAuthority {
    delivery_id: String,
    claim_id: String,
    receipt: NativeEntryReceipt,
    handle: Option<NativeReplyHandle>,
    bot: ChannelBot,
    credentials: EntryCredentialSnapshot,
    deadline: DateTime<Utc>,
}
impl NativeSendAuthority {
    pub(crate) fn deadline(&self) -> DateTime<Utc> {
        self.deadline
    }
}

/// Run before the final send transaction. A long model turn may outlive its
/// access token, so use the ordinary refresh producer while the receipt's exact
/// credential identity still matches. The final capture reloads and fences the
/// resulting encrypted material; nothing refreshes or re-reads it after claim.
pub(crate) async fn refresh_before_claim(
    state: &AppState,
    receipt: &NativeEntryReceipt,
    deadline: DateTime<Utc>,
    adapter: &dyn PlatformAdapter,
) -> AppResult<()> {
    let deadline = deadline.min(receipt.expires_at);
    let remaining = (deadline - Utc::now()).to_std().map_err(|_| refused())?;
    if remaining.is_zero() {
        return Err(refused());
    }
    tokio::time::timeout(remaining.min(std::time::Duration::from_secs(20)), async {
        binding_digest(receipt)?;
        if adapter.platform_id()
            != super::platform_entry_native::platform_name(receipt.configuration.identity.platform)
        {
            return Err(refused());
        }
        let bot = super::channel_bot_service::get_bot(
            &state.db,
            &receipt.configuration.identity.channel_bot_id,
        )
        .await?;
        let before = EntryCredentialSnapshot::load(&state.db, adapter, &bot).await?;
        if before.fingerprint()? != receipt.configuration.credential_fingerprint {
            return Err(refused());
        }
        if bot.credential_source == "connection" {
            super::channel_credentials::resolve_bot_token(
                &state.db,
                &state.encryption_keys,
                adapter,
                &bot,
            )
            .await?;
        }
        let after = EntryCredentialSnapshot::load(&state.db, adapter, &bot).await?;
        if after.fingerprint()? != receipt.configuration.credential_fingerprint
            || Utc::now() >= deadline
        {
            return Err(refused());
        }
        Ok(())
    })
    .await
    .map_err(|_| refused())?
}
impl std::fmt::Debug for NativeSendAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeSendAuthority([REDACTED])")
    }
}

/// Called inside the final delivery claim transaction, after its personal or
/// public authority/body checks. A pending delivery cannot decrypt a handle.
pub(crate) async fn capture_in_session(
    state: &AppState,
    session: &mut ClientSession,
    delivery: &PlatformManagerDelivery,
    claim_id: &str,
    ingress: &crate::models::platform_manager_ingress::PlatformManagerIngress,
    adapter: &dyn PlatformAdapter,
    send_deadline: DateTime<Utc>,
) -> AppResult<NativeSendAuthority> {
    let StoredEntryIdentity::NativeV2 { platform_entry } = &delivery.entry else {
        return Err(refused());
    };
    let StoredSenderIdentity::NativeV2 { platform_sender } = &delivery.sender else {
        return Err(refused());
    };
    let StoredEventIdentity::NativeV2 {
        platform_event_id,
        platform_message_id,
    } = &ingress.event
    else {
        return Err(refused());
    };
    if ingress.profile_id != delivery.profile_id
        || ingress.entry != delivery.entry
        || ingress.sender != delivery.sender
        || ingress.manager_authority_epoch != delivery.manager_authority_epoch
        || delivery.ingress_id.as_deref() != Some(&ingress.id)
        || delivery.ingress_claim_id.as_deref() != Some(&ingress.claim_id)
        || adapter.platform_id()
            != super::platform_entry_native::platform_name(platform_entry.platform)
    {
        return Err(refused());
    }
    let claimed = state
        .db
        .collection::<mongodb::bson::Document>(DELIVERIES)
        .find_one(doc! {
            "_id":&delivery.id, "state":"dispatching", "claim_id":claim_id,
            "expires_at":{"$gt":mongodb::bson::DateTime::now()}
        })
        .session(&mut *session)
        .await?
        .is_some();
    if !claimed {
        return Err(refused());
    }
    let receipt = state.db.collection::<NativeEntryReceipt>(crate::models::platform_entry_identity::NATIVE_RECEIPTS)
        .find_one(doc! {"_id":&ingress.observation_id,"profile_id":&delivery.profile_id,"manager_authority_epoch":delivery.manager_authority_epoch,
            "configuration.identity":mongodb::bson::to_bson(platform_entry).map_err(|_|refused())?,
            "sender":mongodb::bson::to_bson(platform_sender).map_err(|_|refused())?,
            "event_id":platform_event_id,"message_id":platform_message_id})
        .session(&mut *session).await?.ok_or_else(refused)?;
    binding_digest(&receipt)?;
    if receipt.expires_at <= Utc::now() || receipt.received_at > Utc::now() {
        return Err(refused());
    }
    super::platform_entry_receipts::fence_configuration(
        &state.db,
        session,
        &receipt.profile_id,
        receipt.manager_authority_epoch,
        &receipt.configuration,
    )
    .await?;
    let bot =
        super::channel_bot_service::get_bot(&state.db, &platform_entry.channel_bot_id).await?;
    let credentials = EntryCredentialSnapshot::load(&state.db, adapter, &bot).await?;
    if credentials.fingerprint()? != receipt.configuration.credential_fingerprint
        || !credentials.fence_in_session(&state.db, session).await?
    {
        return Err(refused());
    }
    let handle = if platform_entry.platform == EntryPlatform::Discord {
        let handle = state
            .db
            .collection::<NativeReplyHandle>(HANDLES)
            .find_one(doc! {"receipt_id":&receipt.id})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        check_handle(&handle, &receipt)?;
        Some(handle)
    } else {
        None
    };
    let deadline = send_deadline
        .min(delivery.expires_at)
        .min(receipt.expires_at)
        .min(
            handle
                .as_ref()
                .map_or(receipt.expires_at, |handle| handle.expires_at),
        );
    if deadline <= Utc::now() {
        return Err(refused());
    }
    Ok(NativeSendAuthority {
        delivery_id: delivery.id.clone(),
        claim_id: claim_id.into(),
        receipt,
        handle,
        bot,
        credentials,
        deadline,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscordReplySecret {
    application_id: String,
    token: String,
}
impl Drop for DiscordReplySecret {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
impl std::fmt::Debug for DiscordReplySecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscordReplySecret([REDACTED])")
    }
}

pub(crate) enum NativeSendOutcome {
    Sent { platform_message_id: String },
    Refused,
    Uncertain,
}

pub(crate) fn text_limit(platform: EntryPlatform) -> usize {
    match platform {
        EntryPlatform::Discord => 2000,
        _ => 4096,
    }
}

#[cfg(test)]
pub(crate) struct NativeTestTransport {
    pub adapter: std::sync::Arc<dyn PlatformAdapter>,
    pub discord_base: String,
}
#[cfg(test)]
tokio::task_local! {
    static TEST_NATIVE_TRANSPORT: std::sync::Arc<NativeTestTransport>;
}
#[cfg(test)]
pub(crate) async fn with_transport_for_test<T>(
    transport: std::sync::Arc<NativeTestTransport>,
    work: impl std::future::Future<Output = T>,
) -> T {
    TEST_NATIVE_TRANSPORT.scope(transport, work).await
}

pub(crate) fn send_adapter(
    state: &AppState,
    platform: EntryPlatform,
) -> AppResult<std::sync::Arc<dyn PlatformAdapter>> {
    #[cfg(test)]
    if let Ok(transport) = TEST_NATIVE_TRANSPORT.try_with(std::sync::Arc::clone) {
        if transport.adapter.platform_id() != super::platform_entry_native::platform_name(platform)
        {
            return Err(refused());
        }
        return Ok(transport.adapter.clone());
    }
    Ok(std::sync::Arc::from(
        super::channel_adapters::resolve_adapter(
            super::platform_entry_native::platform_name(platform),
            &state.token_exchange_cache,
        )?,
    ))
}

pub(crate) fn send_claimed<'a>(
    state: &'a AppState,
    authority: NativeSendAuthority,
    body: Zeroizing<String>,
    adapter: &'a dyn PlatformAdapter,
) -> futures::future::BoxFuture<'a, AppResult<NativeSendOutcome>> {
    Box::pin(send_claimed_impl(state, authority, body, adapter))
}

async fn send_claimed_impl(
    state: &AppState,
    authority: NativeSendAuthority,
    body: Zeroizing<String>,
    adapter: &dyn PlatformAdapter,
) -> AppResult<NativeSendOutcome> {
    #[cfg(test)]
    if let Ok(transport) = TEST_NATIVE_TRANSPORT.try_with(std::sync::Arc::clone) {
        return send_claimed_at(state, authority, body, adapter, &transport.discord_base).await;
    }
    send_claimed_at(
        state,
        authority,
        body,
        adapter,
        super::platform_entry_discord_controls::DISCORD_BASE,
    )
    .await
}

#[cfg(test)]
pub(super) async fn send_claimed_for_test(
    state: &AppState,
    authority: NativeSendAuthority,
    body: Zeroizing<String>,
    adapter: &dyn PlatformAdapter,
    discord_base: &str,
) -> AppResult<NativeSendOutcome> {
    send_claimed_at(state, authority, body, adapter, discord_base).await
}

async fn send_claimed_at(
    state: &AppState,
    authority: NativeSendAuthority,
    body: Zeroizing<String>,
    adapter: &dyn PlatformAdapter,
    discord_base: &str,
) -> AppResult<NativeSendOutcome> {
    let attempted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let remaining = match (authority.deadline - Utc::now()).to_std() {
        Ok(value) if !value.is_zero() => value.min(std::time::Duration::from_secs(20)),
        _ => return Ok(NativeSendOutcome::Refused),
    };
    let end = tokio::time::Instant::now() + remaining;
    match super::channel_platform::track_provider_attempt(
        attempted.clone(),
        tokio::time::timeout_at(
            end,
            materialize_and_send(
                state,
                authority,
                body,
                adapter,
                discord_base,
                attempted.as_ref(),
            ),
        ),
    )
    .await
    {
        Ok(Ok(outcome)) => Ok(outcome),
        _ => Ok(if attempted.load(std::sync::atomic::Ordering::Relaxed) {
            NativeSendOutcome::Uncertain
        } else {
            NativeSendOutcome::Refused
        }),
    }
}

async fn materialize_and_send(
    state: &AppState,
    authority: NativeSendAuthority,
    body: Zeroizing<String>,
    adapter: &dyn PlatformAdapter,
    discord_base: &str,
    attempted: &std::sync::atomic::AtomicBool,
) -> AppResult<NativeSendOutcome> {
    if Utc::now() >= authority.deadline
        || body.trim().is_empty()
        || body.encode_utf16().count()
            > text_limit(authority.receipt.configuration.identity.platform)
        || state
            .db
            .collection::<mongodb::bson::Document>(DELIVERIES)
            .find_one(doc! {
                "_id":&authority.delivery_id,"state":"dispatching","claim_id":&authority.claim_id,
                "expires_at":{"$gt":mongodb::bson::DateTime::now()}
            })
            .await?
            .is_none()
    {
        return Ok(NativeSendOutcome::Refused);
    }
    if authority.receipt.configuration.identity.platform == EntryPlatform::Discord {
        let handle = authority.handle.as_ref().ok_or_else(refused)?;
        check_handle(handle, &authority.receipt)?;
        let plaintext = Zeroizing::new(
            state
                .encryption_keys
                .decrypt_with_aad(&handle.ciphertext, handle.binding_digest.as_bytes())
                .await
                .map_err(|_| refused())?,
        );
        if plaintext.len() > 4096 {
            return Ok(NativeSendOutcome::Refused);
        }
        let secret: DiscordReplySecret =
            serde_json::from_slice(&plaintext).map_err(|_| refused())?;
        let NativeControlSurface::DiscordCommand { application_id, .. } =
            &authority.receipt.configuration.control_surface
        else {
            return Ok(NativeSendOutcome::Refused);
        };
        if secret.application_id != *application_id
            || secret.token.is_empty()
            || secret.token.len() > 2048
            || !secret
                .token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            || !application_id.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Ok(NativeSendOutcome::Refused);
        }
        // An interaction has exactly this provider follow-up surface. Never call
        // the ordinary adapter's /channels fallback, even on missing/old data.
        let url = Zeroizing::new(format!(
            "{discord_base}/webhooks/{application_id}/{}?wait=true",
            secret.token
        ));
        let result = async {
            if Utc::now() >= authority.deadline { return None; }
            super::channel_platform::mark_provider_attempt();
            let mut response = state.http_client.post(url.as_str())
                .json(&serde_json::json!({"content":body.as_str(),"allowed_mentions":{"parse":[]},"flags":4}))
                .send().await.ok()?;
            let status = response.status();
            let mut bytes = Zeroizing::new(Vec::new());
            while let Some(chunk) = response.chunk().await.ok()? {
                if bytes.len().checked_add(chunk.len()).is_none_or(|size|size>65536) { return None; }
                bytes.extend_from_slice(&chunk);
            }
            Some((status,serde_json::from_slice::<serde_json::Value>(&bytes).ok()))
        }.await;
        return Ok(match result {
            Some((status, Some(value))) if status.is_success() => {
                match value["id"].as_str().filter(|id| opaque(id).is_ok()) {
                    Some(id) => NativeSendOutcome::Sent {
                        platform_message_id: id.into(),
                    },
                    None => NativeSendOutcome::Uncertain,
                }
            }
            Some((status, _)) if status.is_client_error() => NativeSendOutcome::Refused,
            _ if !attempted.load(std::sync::atomic::Ordering::Relaxed) => {
                NativeSendOutcome::Refused
            }
            _ => NativeSendOutcome::Uncertain,
        });
    }
    let token = authority
        .credentials
        .send_token(&state.encryption_keys)
        .await?;
    let secrets = authority
        .credentials
        .secrets(&state.encryption_keys, adapter)
        .await?;
    let billing = super::channel_billing_service::ChannelBilling::for_bot(
        &state.db,
        &state.billing,
        &authority.bot,
        None,
    );
    let credentials = BotCredentials {
        token: &token,
        platform_bot_id: Some(&authority.bot.platform_bot_id),
        platform_secrets: Some(&secrets),
        billing: billing.as_ref(),
    };
    let outbound = OutboundReply {
        attachments: vec![],
        text: Some(body.as_str().into()),
        reply_to_platform_message_id: None,
        metadata: None,
    };
    // X retains its ordinary reservation -> provider -> settlement/uncertainty
    // path through BotCredentials.billing. One bounded text reply means one send.
    if Utc::now() >= authority.deadline {
        return Ok(NativeSendOutcome::Refused);
    }
    let result = adapter
        .send_reply(
            &state.http_client,
            &credentials,
            &authority.receipt.sender.private_conversation_id,
            &outbound,
        )
        .await;
    Ok(match result {
        Ok(Some(id)) if opaque(&id).is_ok() => NativeSendOutcome::Sent {
            platform_message_id: id,
        },
        Err(error)
            if billing
                .as_ref()
                .is_some_and(|billing| !billing.has_forwarded())
                && super::channel_billing_service::blocks_channel(&error) =>
        {
            NativeSendOutcome::Refused
        }
        _ if !attempted.load(std::sync::atomic::Ordering::Relaxed) => NativeSendOutcome::Refused,
        _ => NativeSendOutcome::Uncertain,
    })
}

#[cfg(test)]
#[path = "platform_native_reply_tests.rs"]
pub(crate) mod tests;

/// Preserve authenticated local test adapters across each asynchronous worker.
/// Production uses the ordinary Tokio spawn contract.
pub(crate) fn spawn_in_native_scope<F, T>(work: F) -> tokio::task::JoinHandle<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    #[cfg(test)]
    if let Ok(transport) = TEST_NATIVE_TRANSPORT.try_with(std::sync::Arc::clone) {
        return tokio::spawn(async move { with_transport_for_test(transport, work).await });
    }
    tokio::spawn(work)
}
