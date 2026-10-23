//! Shared-entry transport has dedicated gateway keys and persistence. It never
//! selects ordinary bot-owner routing, synthetic-success responses or fallback.
use super::{
    platform_entry_identity::refused, platform_entry_receipts::NativeAdmission,
    platform_manager_contract::PlatformManagerProducers,
};
use crate::{
    AppState,
    errors::AppResult,
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        platform_bot_config::PlatformBotConfig,
        platform_entry_gateway::{COLLECTION_NAME as GATEWAYS, PlatformEntryGateway},
        platform_entry_identity::{EntryTransport, NativeEntryConfiguration},
    },
};
use axum::http::HeaderMap;
use bson::{Document, doc};
use mongodb::ClientSession;
use serde_json::{Value, json};
use std::sync::Arc;
use zeroize::Zeroizing;

#[cfg(test)]
#[path = "platform_entry_gateway_tests.rs"]
pub(crate) mod tests;

pub(crate) struct GatewayAuthority {
    row: PlatformEntryGateway,
    key: ApiKey,
    bearer: Zeroizing<String>,
}
impl std::fmt::Debug for GatewayAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayAuthority([REDACTED])")
    }
}

fn configuration_matches(
    row: &PlatformEntryGateway,
    profile: &str,
    configuration: &NativeEntryConfiguration,
) -> AppResult<()> {
    if row.state == "prepared" && row.setup_state != "observed" {
        let EntryTransport::NyxagentGateway {
            gateway_channel_id,
            gateway_version,
            ..
        } = &configuration.transport
        else {
            return Err(refused());
        };
        if gateway_channel_id != &format!("pending:{}", row.id) || *gateway_version != 1 {
            return Err(refused());
        }
        let mut direct = configuration.clone();
        direct.transport = EntryTransport::Direct;
        super::platform_entry_identity::validate_configuration(&direct)?;
    } else {
        super::platform_entry_identity::validate_configuration(configuration)?;
    }
    let EntryTransport::NyxagentGateway { channel_id, .. } = &configuration.transport else {
        return Err(refused());
    };
    if row.id != *channel_id
        || row.profile_id != profile
        || row.configuration != *configuration
        || row.revision <= 0
    {
        return Err(refused());
    }
    Ok(())
}

async fn fence_gateway(
    state: &AppState,
    session: &mut ClientSession,
    row: &PlatformEntryGateway,
) -> AppResult<()> {
    configuration_matches(row, &row.profile_id, &row.configuration)?;
    let changed = state.db.collection::<Document>(GATEWAYS).update_one(doc! {
        "_id":&row.id,"profile_id":&row.profile_id,"configuration":bson::to_bson(&row.configuration).map_err(|_|refused())?,
        "credential_owner_id":&row.credential_owner_id,"route_api_key_id":&row.route_api_key_id,"route_id":&row.route_id,
        "provider_api_key_id":&row.provider_api_key_id,"callback_url":&row.callback_url,"binding_id":row.binding_id.clone(),
        "revision":row.revision,"state":&row.state}, doc! {"$inc":{"effect_revision":1_i64}}).session(&mut *session).await?;
    if changed.matched_count != 1 {
        return Err(refused());
    }
    Ok(())
}

async fn fence_key(
    db: &mongodb::Database,
    session: &mut ClientSession,
    key: &ApiKey,
) -> AppResult<()> {
    // Validation touches metadata through the authoritative mutation service.
    // Compare security fields, then fence the exact version read in this transaction.
    // A benign concurrent use may change timestamps/version; rotation, grants,
    // expiry, owner or any other key configuration still refuses this authority.
    let live = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {
            "_id":&key.id,"user_id":&key.user_id,"key_hash":&key.key_hash,"is_active":true
        })
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    fn security_projection(mut key: ApiKey) -> ApiKey {
        key.last_used_at = None;
        key.updated_at = None;
        key.state_version = 0;
        key
    }
    if live.purpose != crate::models::api_key::ApiKeyPurpose::SharedEntryTransport
        || security_projection(live.clone()) != security_projection(key.clone())
    {
        return Err(refused());
    }
    let result = db.collection::<Document>(KEYS).update_one(doc! {
        "_id":&live.id,"user_id":&live.user_id,"key_hash":&live.key_hash,"is_active":true,
        "state_version": if live.state_version == 0 {bson::Bson::Document(doc! {"$in":[0_i64,bson::Bson::Null]})}else{bson::Bson::Int64(live.state_version)},
        "$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
        doc! {"$inc":{"platform_entry_gateway_effect_revision":1_i64}}).session(&mut *session).await?;
    if result.matched_count != 1 {
        return Err(refused());
    }
    Ok(())
}

/// Called before ordinary provider_channel performs ownership cleanup. A
/// matching retired/malformed shared transport returns an error, never None.
pub(crate) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> AppResult<Option<GatewayAuthority>> {
    let Some(raw) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return Ok(None);
    };
    let raw = Zeroizing::new(raw.trim().to_owned());
    // Select the dedicated namespace without touching unchanged ordinary keys.
    // Retired and inactive associations remain visible and refuse below.
    let hash = crate::crypto::token::hash_token(&raw);
    let primary = state
        .db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"key_hash":&hash})
        .await?;
    let (selected, is_child) = if let Some(primary) = primary {
        (Some(primary), false)
    } else {
        let child = state
            .db
            .collection::<Document>(crate::models::api_key_credential::COLLECTION_NAME)
            .find_one(doc! {"secret_hash":&hash})
            .await?;
        let parent = match child
            .as_ref()
            .and_then(|child| child.get_str("api_key_id").ok())
        {
            Some(id) => {
                state
                    .db
                    .collection::<ApiKey>(KEYS)
                    .find_one(doc! {"_id":id})
                    .await?
            }
            None => None,
        };
        (parent, true)
    };
    let Some(selected) = selected else {
        return Ok(None);
    };
    let row = state
        .db
        .collection::<PlatformEntryGateway>(GATEWAYS)
        .find_one(doc! {"provider_api_key_id":&selected.id})
        .await?;
    let Some(row) = row else {
        if selected.purpose == crate::models::api_key::ApiKeyPurpose::SharedEntryTransport {
            return Err(refused());
        }
        return Ok(None);
    };
    if is_child {
        return Err(refused());
    }
    let (owner, key, child): (String, ApiKey, Option<String>) =
        super::key_service::validate_api_key(&state.db, &raw).await?;
    if key.id != selected.id {
        return Err(refused());
    }
    if child.is_some()
        || key.purpose != crate::models::api_key::ApiKeyPurpose::SharedEntryTransport
        || owner != row.credential_owner_id
        || !matches!(row.state.as_str(), "prepared" | "active")
    {
        return Err(refused());
    }
    let hash = crate::crypto::token::hash_token(&raw);
    let key = state
        .db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {
            "_id":&key.id,"user_id":&owner,"key_hash":hash,"is_active":true,
            "$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]
        })
        .await?
        .ok_or_else(refused)?;
    configuration_matches(&row, &row.profile_id, &row.configuration)?;
    Ok(Some(GatewayAuthority {
        row,
        key,
        bearer: raw,
    }))
}

pub(crate) async fn forward_native(
    state: &AppState,
    profile: &PlatformBotConfig,
    admission: &NativeAdmission,
) -> AppResult<()> {
    let receipt = admission.receipt();
    let EntryTransport::NyxagentGateway { channel_id, .. } = &receipt.configuration.transport
    else {
        return Err(refused());
    };
    let row = state
        .db
        .collection::<PlatformEntryGateway>(GATEWAYS)
        .find_one(doc! {"_id":channel_id})
        .await?
        .ok_or_else(refused)?;
    configuration_matches(&row, &profile.id, &receipt.configuration)?;
    if row.state != "active" || receipt.expires_at <= chrono::Utc::now() {
        return Err(refused());
    }
    let key = state
        .db
        .collection::<ApiKey>(KEYS)
        .find_one(
            doc! {"_id":&row.route_api_key_id,"user_id":&row.credential_owner_id,"is_active":true},
        )
        .await?
        .ok_or_else(refused)?;
    if key.callback_url.as_deref() != Some(&row.callback_url) {
        return Err(refused());
    }
    let base = super::nyxbot_gateway_service::gateway_base(state).await?;
    if base != row.gateway_base
        || row.request_policy["sources"][0]["issuer"] != state.config.jwt_issuer
    {
        return Err(refused());
    }
    checked_callback(&base, &row.callback_url)?;
    let mut session = state.db.client().start_session().await?;
    let claimed = session.start_transaction().and_run((state,&row,&key,receipt), |session,(state,row,key,receipt)|Box::pin(async move {
        super::api_key_mutation_service::transaction_result(async {
            fence_gateway(state,session,row).await?;
            fence_key(&state.db,session,key).await?;
            super::platform_entry_receipts::fence_configuration(&state.db,session,&receipt.profile_id,receipt.manager_authority_epoch,&receipt.configuration).await?;
            // Pending is claimed once. An unavailable/uncertain selected gateway
            // cannot execute directly, emit owner tokens, or replay its callback.
            let claim = state.db.collection::<ChannelMessage>(MESSAGES).update_one(doc! {"_id":&receipt.id,"channel_bot_id":&receipt.configuration.identity.channel_bot_id,
                "conversation_id":&receipt.profile_id,"callback_status":"pending"},doc! {"$set":{"callback_status":"dispatching","agent_api_key_id":&key.id}}).session(&mut *session).await?;
            Ok(claim.matched_count == 1)
        }.await)
    })).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    if !claimed {
        return Ok(());
    }
    let mut reply_config = state.config.clone();
    reply_config.jwt_relay_reply_ttl_secs = reply_config
        .jwt_relay_reply_ttl_secs
        .min((receipt.expires_at - chrono::Utc::now()).num_seconds());
    if reply_config.jwt_relay_reply_ttl_secs <= 0 {
        return Err(refused());
    }
    let reply_token = crate::crypto::jwt::generate_relay_reply_token(
        &state.jwt_keys,
        &reply_config,
        &row.route_api_key_id,
        &row.route_id,
        &receipt.id,
        super::platform_entry_native::platform_name(receipt.configuration.identity.platform),
    )?;
    use super::channel_relay_service::*;
    let payload = CallbackPayload {
        thread_context: None,
        activity: None,
        message_id: receipt.id.clone(),
        correlation_id: String::new(),
        platform: super::platform_entry_native::platform_name(
            receipt.configuration.identity.platform,
        )
        .into(),
        reply_token: Some(reply_token),
        agent: CallbackAgent {
            api_key_id: key.id.clone(),
            name: "NyxID shared chat entry".into(),
        },
        conversation: CallbackConversation {
            id: row.route_id.clone(),
            platform_id: receipt.sender.private_conversation_id.clone(),
            conversation_type: "private".into(),
        },
        sender: CallbackSender {
            platform_id: receipt.sender.sender_id.clone(),
            display_name: None,
        },
        content: CallbackContent {
            content_type: "text".into(),
            text: Some(admission.text().into()),
            attachments: vec![],
        },
        reply_to_message_id: None,
        reply_to_platform_message_id: None,
        thread_id: None,
        timestamp: chrono::Utc::now().to_rfc3339(),
        raw_platform_data: None,
    };
    // The audience-bound relay token names only this dedicated route/receipt;
    // no owner access token or ordinary routing record is created.
    let delivery = forward_to_agent(
        &state.http_client,
        &state.config,
        &state.jwt_keys,
        &row.callback_url,
        payload,
        &key.id,
        &key.key_hash,
        None,
    )
    .await;
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state.db.collection::<ChannelMessage>(MESSAGES).update_one(
            doc! {"_id":&receipt.id,"callback_status":"dispatching"},
            doc! {"$set":{"callback_status":if delivery.result.is_ok(){"delivered"}else{"failed"}}},
        ),
    )
    .await;
    delivery.result
}

pub(crate) async fn put_binding(
    state: &AppState,
    authority: GatewayAuthority,
    binding: &str,
    body: &Value,
) -> AppResult<Value> {
    use subtle::ConstantTimeEq;
    if binding.is_empty()
        || binding.len() > 128
        || !binding
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || !bool::from(
            body["agent_key"]
                .as_str()
                .unwrap_or_default()
                .as_bytes()
                .ct_eq(authority.bearer.as_bytes()),
        )
        || body["profile"]["metadata"]["owner"]["subject"].as_str()
            != Some(&authority.row.credential_owner_id)
        || body["profile"]["metadata"]["provider"] != "nyxbot"
        || body["profile"] != authority.row.request_policy["profile"]
        || body["profile_digest"]
            != sha256(canonical_json(authority.row.request_policy["profile"].clone()).as_bytes())
        || authority
            .row
            .binding_id
            .as_deref()
            .is_some_and(|old| old != binding)
    {
        return Err(refused());
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    fence_gateway(state, &mut session, &authority.row).await?;
    fence_key(&state.db, &mut session, &authority.key).await?;
    state
        .db
        .collection::<Document>(GATEWAYS)
        .update_one(
            doc! {"_id":&authority.row.id},
            doc! {"$set":{"binding_id":binding}},
        )
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    Ok(
        json!({"binding_id":binding,"level":"l1","honoured":["metadata"],"refused":[
            {"field":"model","reason":"Shared entry selects the current public or personally linked runtime."},
            {"field":"permission","reason":"Sender consent and live NyxID authority govern each event."},
            {"field":"context","reason":"Only native receipt-bound text is admitted."}
        ]}),
    )
}

pub(crate) async fn conversation(
    state: &AppState,
    authority: GatewayAuthority,
    binding: &str,
    partition: &str,
) -> AppResult<()> {
    if authority.row.binding_id.as_deref() != Some(binding)
        || !partition
            .strip_prefix("conv_")
            .is_some_and(|tail| tail.len() == 32 && tail.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(refused());
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    fence_gateway(state, &mut session, &authority.row).await?;
    fence_key(&state.db, &mut session, &authority.key).await?;
    // Gateway partition bookkeeping is not sender authority and stores no text.
    state.db.collection::<Document>("platform_entry_gateway_partitions").update_one(doc! {"channel_id":&authority.row.id,"partition":partition},
        doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"created_at":bson::DateTime::now()}}).upsert(true).session(&mut session).await?;
    session.commit_transaction().await?;
    Ok(())
}

pub(crate) enum GatewayResponsesOutcome {
    ConversationMissing,
    Dispatched,
}

pub(crate) fn responses<'a>(
    state: &'a AppState,
    authority: GatewayAuthority,
    request: &'a Value,
    producers: Option<Arc<PlatformManagerProducers>>,
) -> futures::future::BoxFuture<'a, AppResult<GatewayResponsesOutcome>> {
    // Ordinary gateway handlers construct native work only after authentication
    // has selected this transport, and retain just its boxed future.
    Box::pin(responses_impl(state, authority, request, producers))
}

async fn responses_impl(
    state: &AppState,
    authority: GatewayAuthority,
    request: &Value,
    producers: Option<Arc<PlatformManagerProducers>>,
) -> AppResult<GatewayResponsesOutcome> {
    if authority.row.state != "active" {
        return Err(refused());
    }
    let partition = request["conversation"].as_str().ok_or_else(refused)?;
    if !partition
        .strip_prefix("conv_")
        .is_some_and(|tail| tail.len() == 32 && tail.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(refused());
    }
    let context = &request["event_context"];
    let EntryTransport::NyxagentGateway {
        gateway_channel_id, ..
    } = &authority.row.configuration.transport
    else {
        return Err(refused());
    };
    if context["schema_version"] != 1
        || context["channel_id"].as_str() != Some(gateway_channel_id)
        || request["stream"] != true
        || context["conversation_id"].as_str() != Some(partition)
        || context["activity"].is_null()
        || context["activity"]["source"]["route_id"] != authority.row.route_id
    {
        return Err(refused());
    }
    let input = request["input"].as_array().ok_or_else(refused)?;
    if input.len() != 1 || input[0]["role"] != "user" {
        return Err(refused());
    }
    let content = input[0]["content"].as_array().ok_or_else(refused)?;
    if content.len() != 1 || content[0]["type"] != "input_text" {
        return Err(refused());
    }
    let text = content[0]["text"].as_str().ok_or_else(refused)?;
    let mut session = state.db.client().start_session().await?;
    let admission = session
        .start_transaction()
        .and_run(
            (state, &authority, partition, &context["activity"], text),
            |session, (state, authority, partition, activity, text)| {
                Box::pin(async move {
                    super::api_key_mutation_service::transaction_result(
                        async {
                            fence_gateway(state, session, &authority.row).await?;
                            fence_key(&state.db, session, &authority.key).await?;
                            let admission =
                                super::platform_entry_receipts::verify_gateway_in_session(
                                    &state.db,
                                    session,
                                    state.auth_device_hmac_key.as_slice(),
                                    &state.token_exchange_cache,
                                    super::platform_entry_receipts::GatewayReceiptInput {
                                        profile_id: &authority.row.profile_id,
                                        configuration: &authority.row.configuration,
                                        transport_channel_id: &authority.row.id,
                                        activity,
                                        text,
                                    },
                                )
                                .await?;
                            if state
                                .db
                                .collection::<Document>("platform_entry_gateway_partitions")
                                .find_one(
                                    doc! {"channel_id":&authority.row.id,"partition":*partition},
                                )
                                .session(&mut *session)
                                .await?
                                .is_none()
                            {
                                // Only a live transport and exact retained native
                                // event may request the consumer's partition setup.
                                return Ok(None);
                            }
                            Ok(Some(admission))
                        }
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)?;
    let Some(admission) = admission else {
        return Ok(GatewayResponsesOutcome::ConversationMissing);
    };
    #[cfg(test)]
    if let Ok(gate) = BEFORE_NATIVE_DISPATCH.try_with(Arc::clone) {
        gate.entered.notify_one();
        gate.release.notified().await;
    }
    super::platform_manager_transport::dispatch_native_admission(state, producers, admission)
        .await?;
    Ok(GatewayResponsesOutcome::Dispatched)
}

#[cfg(test)]
struct NativeDispatchGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[cfg(test)]
tokio::task_local! {
    static BEFORE_NATIVE_DISPATCH: Arc<NativeDispatchGate>;
}

pub(crate) const PARTITIONS: &str = "platform_entry_gateway_partitions";

pub(crate) async fn ensure_indexes(db: &mongodb::Database) -> AppResult<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    for (name, keys, partial) in [
        (
            "shared_gateway_provider_key_v2",
            doc! {"provider_api_key_id":1},
            None,
        ),
        (
            "shared_gateway_route_key_v2",
            doc! {"route_api_key_id":1},
            None,
        ),
        (
            "shared_gateway_live_profile_v2",
            doc! {"profile_id":1},
            Some(doc! {"state":{"$in":["prepared","active"]}}),
        ),
    ] {
        db.collection::<Document>(GATEWAYS)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .unique(true)
                            .partial_filter_expression(partial)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>(PARTITIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"channel_id":1,"partition":1})
                .options(
                    IndexOptions::builder()
                        .name("shared_gateway_partition_v2".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

/// Joins every native control, turn and delivery fence, including those that
/// originate from direct callers rather than the gateway HTTP handler.
pub(crate) async fn evaluate_selected_transport(
    db: &mongodb::Database,
    session: &mut ClientSession,
    profile: &str,
    configuration: &NativeEntryConfiguration,
    write_fence: bool,
) -> AppResult<bool> {
    let EntryTransport::NyxagentGateway { channel_id, .. } = &configuration.transport else {
        return Ok(true);
    };
    let Some(row)=db.collection::<PlatformEntryGateway>(GATEWAYS).find_one(doc! {
        "_id":channel_id,"profile_id":profile,"configuration":bson::to_bson(configuration).map_err(|_|refused())?,"state":"active"})
        .session(&mut *session).await? else {return Ok(false);};
    configuration_matches(&row, profile, configuration)?;
    for id in [&row.provider_api_key_id, &row.route_api_key_id] {
        let mut filter = doc! {"_id":id,"user_id":&row.credential_owner_id,"is_active":true,"purpose":"shared_entry_transport",
        "scopes":"","allow_all_services":false,"allow_auto_connected_services":false,"allow_all_nodes":false,
        "$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]};
        if id == &row.route_api_key_id {
            filter.insert("callback_url", &row.callback_url);
        }
        let Some(key) = db
            .collection::<ApiKey>(KEYS)
            .find_one(filter.clone())
            .session(&mut *session)
            .await?
        else {
            return Ok(false);
        };
        if write_fence {
            filter.insert("key_hash", key.key_hash);
            filter.insert(
                "state_version",
                if key.state_version == 0 {
                    bson::Bson::Document(doc! {"$in":[0_i64,bson::Bson::Null]})
                } else {
                    bson::Bson::Int64(key.state_version)
                },
            );
            if db
                .collection::<Document>(KEYS)
                .update_one(
                    filter,
                    doc! {"$inc":{"platform_entry_gateway_effect_revision":1_i64}},
                )
                .session(&mut *session)
                .await?
                .matched_count
                != 1
            {
                return Ok(false);
            }
        }
    }
    if write_fence && db.collection::<Document>(GATEWAYS).update_one(doc! {
        "_id":channel_id,"profile_id":profile,"state":"active","revision":row.revision,
        "configuration":bson::to_bson(configuration).map_err(|_|refused())?,"provider_api_key_id":&row.provider_api_key_id,
        "route_api_key_id":&row.route_api_key_id,"callback_url":&row.callback_url,"binding_id":&row.binding_id},
        doc! {"$inc":{"effect_revision":1_i64}}).session(&mut *session).await?.matched_count!=1{return Ok(false);}
    Ok(true)
}
pub(crate) async fn fence_selected_transport(
    db: &mongodb::Database,
    session: &mut ClientSession,
    profile: &str,
    configuration: &NativeEntryConfiguration,
) -> AppResult<()> {
    if !evaluate_selected_transport(db, session, profile, configuration, true).await? {
        return Err(refused());
    }
    Ok(())
}

pub(crate) async fn delete_binding(
    state: &AppState,
    authority: GatewayAuthority,
    binding: &str,
) -> AppResult<()> {
    if authority.row.binding_id.as_deref() != Some(binding) {
        return Err(refused());
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    fence_gateway(state, &mut session, &authority.row).await?;
    fence_key(&state.db, &mut session, &authority.key).await?;
    // Keep the dedicated key/entry association as a retirement tombstone.
    state.db.collection::<Document>(GATEWAYS).update_one(doc! {"_id":&authority.row.id},
        doc! {"$set":{"state":"retired","updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}}).session(&mut session).await?;
    state
        .db
        .collection::<Document>(PARTITIONS)
        .delete_many(doc! {"channel_id":&authority.row.id})
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    Ok(())
}

pub(crate) async fn delete_conversation(
    state: &AppState,
    authority: GatewayAuthority,
    binding: &str,
    partition: &str,
) -> AppResult<()> {
    if authority.row.binding_id.as_deref() != Some(binding) {
        return Err(refused());
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    fence_gateway(state, &mut session, &authority.row).await?;
    fence_key(&state.db, &mut session, &authority.key).await?;
    state
        .db
        .collection::<Document>(PARTITIONS)
        .delete_one(doc! {"channel_id":&authority.row.id,"partition":partition})
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    Ok(())
}

pub(crate) async fn event_context(
    state: &AppState,
    authority: GatewayAuthority,
    binding: &str,
    partition: &str,
    _event: &str,
) -> AppResult<()> {
    if authority.row.binding_id.as_deref() != Some(binding) {
        return Err(refused());
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    fence_gateway(state, &mut session, &authority.row).await?;
    fence_key(&state.db, &mut session, &authority.key).await?;
    if state
        .db
        .collection::<Document>(PARTITIONS)
        .find_one(doc! {"channel_id":&authority.row.id,"partition":partition})
        .session(&mut session)
        .await?
        .is_none()
    {
        return Err(refused());
    }
    session.commit_transaction().await?;
    Ok(())
}

/// Retained successful setup result. Activation checks the exact observed row,
/// dedicated primary keys and configuration in the conversion transaction.
pub(crate) struct PreparedGateway {
    row: PlatformEntryGateway,
    provider: ApiKey,
    route: ApiKey,
}
impl PreparedGateway {
    pub(crate) fn configuration(&self) -> &NativeEntryConfiguration {
        &self.row.configuration
    }
    pub(crate) async fn activate_in_session(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        configuration: &NativeEntryConfiguration,
    ) -> AppResult<()> {
        configuration_matches(&self.row, &self.row.profile_id, configuration)?;
        let row = &self.row;
        let result=db.collection::<Document>(GATEWAYS).update_one(setup_filter(row)?,
            doc! {"$set":{"state":"active","updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}}).session(&mut *session).await?;
        if result.matched_count != 1 {
            return Err(refused());
        }
        fence_key(db, session, &self.provider).await?;
        fence_key(db, session, &self.route).await?;
        Ok(())
    }
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_TRANSPORT_ISSUED_AT: chrono::DateTime<chrono::Utc>;
}

fn transport_key(owner: &str) -> (ApiKey, Zeroizing<String>) {
    use rand::RngCore;
    let mut entropy = Zeroizing::new([0_u8; 32]);
    rand::rngs::OsRng.fill_bytes(entropy.as_mut());
    let raw = Zeroizing::new(format!(
        "{}{}",
        crate::crypto::token::AGENT_KEY_PREFIX,
        hex::encode(entropy.as_ref())
    ));
    let issued_at = chrono::Utc::now();
    #[cfg(test)]
    let issued_at = TEST_TRANSPORT_ISSUED_AT
        .try_with(|date| *date)
        .unwrap_or(issued_at);
    let now = bson::DateTime::from_chrono(issued_at).to_chrono();
    let key = ApiKey {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: owner.into(),
        name: "NyxID shared entry transport".into(),
        key_prefix: crate::crypto::token::agent_key_display_prefix(&raw)
            .expect("generated key prefix")
            .into(),
        key_hash: crate::crypto::token::hash_token(&raw),
        scopes: String::new(),
        last_used_at: None,
        expires_at: None,
        is_active: true,
        created_at: now,
        rotation_predecessor_id: None,
        machine_external_grants_present: false,
        state_version: 1,
        updated_at: Some(now),
        description: None,
        allowed_service_ids: vec![],
        allowed_platform_service_ids: vec![],
        assistant_agent_owner_id: None,
        assistant_group_id: None,
        assistant_operation_scopes: Default::default(),
        allowed_node_ids: vec![],
        allow_all_services: false,
        allow_auto_connected_services: false,
        allow_all_nodes: false,
        rate_limit_per_second: Some(5),
        rate_limit_burst: Some(10),
        platform: Some("nyxid-shared-entry".into()),
        callback_url: None,
        purpose: crate::models::api_key::ApiKeyPurpose::SharedEntryTransport,
        scheduled_write_enabled: false,
    };
    (key, raw)
}

fn checked_callback(base: &str, callback: &str) -> AppResult<()> {
    let base = url::Url::parse(base).map_err(|_| refused())?;
    let callback = url::Url::parse(callback).map_err(|_| refused())?;
    let path = base.path().trim_end_matches('/');
    if base.origin() != callback.origin()
        || (!path.is_empty()
            && callback.path() != path
            && !callback.path().starts_with(&format!("{path}/")))
        || !callback.username().is_empty()
        || callback.password().is_some()
        || callback.query().is_some()
        || callback.fragment().is_some()
    {
        return Err(refused());
    }
    Ok(())
}

const SETUP_REQUEST_SECONDS: i64 = 20;
const SETUP_UNCERTAINTY_SECONDS: i64 = 120;

pub(crate) fn supported_transports(
    platform: crate::models::platform_entry_identity::EntryPlatform,
) -> &'static [&'static str] {
    use crate::models::platform_entry_identity::EntryPlatform::*;
    match platform {
        Discord | Slack | Whatsapp => &["direct", "gateway"],
        Lark | Feishu | X => &["direct"],
    }
}

pub(crate) fn require_supported_platform(
    platform: crate::models::platform_entry_identity::EntryPlatform,
) -> AppResult<()> {
    if !supported_transports(platform).contains(&"gateway") {
        return Err(crate::errors::AppError::ValidationError(
            "Gateway transport is unavailable for this platform; select direct transport explicitly".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn preflight(
    state: &AppState,
    platform: crate::models::platform_entry_identity::EntryPlatform,
) -> AppResult<()> {
    require_supported_platform(platform)?;
    super::nyxbot_gateway_service::gateway_base(state).await?;
    let issuer = url::Url::parse(&state.config.jwt_issuer).map_err(|_| refused())?;
    if issuer.scheme() != "https"
        || issuer.host_str().is_none()
        || issuer.path() != "/"
        || issuer.query().is_some()
        || issuer.fragment().is_some()
        || !issuer.username().is_empty()
        || issuer.password().is_some()
    {
        return Err(crate::errors::AppError::ValidationError(
            "Shared gateway requires an HTTPS issuer origin".into(),
        ));
    }
    Ok(())
}

pub(crate) fn request_policy(
    issuer: &str,
    owner: &str,
    record: &str,
    route_key: &str,
    route: &str,
    platform: crate::models::platform_entry_identity::EntryPlatform,
) -> AppResult<Value> {
    require_supported_platform(platform)?;
    Ok(
        json!({"schema_version":1,"record_id":record,"name":"NyxID shared entry",
        "profile":{"schema_version":1,"metadata":{"name":"NyxID shared entry","description":"Public help and sender-consented personal NyxBot",
            "owner":{"subject":owner,"kind":"human"},"provider":"nyxbot"},
            "model":{"access":"managed","service":super::assistant_nyxagent::SERVICE_SLUG,"api_type":"responses",
                "model":super::assistant_nyxagent::DEFAULT_MODEL,"thinking_effort":"low"},
            "permission":{"action_mode":"autonomous","service_scope":[]},"context":{"system_prompt":"","skills":[],"mcp_servers":[]}},
        "provider":{"slug":"nyxbot"},"sources":[{"type":"nyxid_relay","issuer":issuer,"key_id":route_key,
            "route_ids":[route],"platform":super::platform_entry_native::platform_name(platform),"trust_normalized":true,
            "admission":{"type":"scoped","senders":{"type":"open"},"chats":{"type":"open"},"groups":"deny"}}],
        "partition":{"version":1,"key":"conversation_and_sender"},"reply":{"auto_final":false,"ack_after_ms":2500,"sinks":[]}}),
    )
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn canonical_json(value: Value) -> String {
    fn sort(value: Value) -> Value {
        match value {
            Value::Object(value) => Value::Object(
                value
                    .into_iter()
                    .map(|(k, v)| (k, sort(v)))
                    .collect::<std::collections::BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(value) => Value::Array(value.into_iter().map(sort).collect()),
            other => other,
        }
    }
    serde_json::to_string(&sort(value)).expect("JSON values serialize")
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct GatewayRecoveryReview {
    pub(crate) record_id: String,
    pub(crate) attempt_id: String,
    pub(crate) revision: i64,
    pub(crate) request_hash: String,
    pub(crate) credential_fingerprint: String,
    pub(crate) remote_channel_id: Option<String>,
    pub(crate) remote_version: Option<i64>,
    pub(crate) remote_ready: bool,
}
impl std::fmt::Debug for GatewayRecoveryReview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayRecoveryReview([REDACTED])")
    }
}
pub(crate) enum GatewayPreparation {
    Ready(Box<PreparedGateway>),
    Recovery(GatewayRecoveryReview),
}

fn setup_filter(row: &PlatformEntryGateway) -> AppResult<Document> {
    Ok(
        doc! {"_id":&row.id,"profile_id":&row.profile_id,"state":"prepared","revision":row.revision,
        "setup_state":&row.setup_state,"setup_attempt_id":&row.setup_attempt_id,
        "setup_dispatch_deadline":row.setup_dispatch_deadline.map_or(bson::Bson::Null,|date|bson::Bson::DateTime(bson::DateTime::from_chrono(date))),
        "request_hash":&row.request_hash,"creation_fingerprint":&row.creation_fingerprint,"request_policy":bson::to_bson(&row.request_policy).map_err(|_|refused())?,
        "configuration":bson::to_bson(&row.configuration).map_err(|_|refused())?,"gateway_base":&row.gateway_base,
        "provider_api_key_id":&row.provider_api_key_id,"route_api_key_id":&row.route_api_key_id,
        "configured_by":&row.configured_by,"credential_owner_id":&row.credential_owner_id,
        "binding_id":&row.binding_id,"callback_url":&row.callback_url},
    )
}

fn retained_matches(
    row: &PlatformEntryGateway,
    authority: &super::platform_entry_admin_service::NativeConversionAuthority,
    base: &str,
    issuer: &str,
) -> AppResult<()> {
    let mut expected = authority.configuration().clone();
    expected.transport = row.configuration.transport.clone();
    let policy = request_policy(
        issuer,
        authority.actor(),
        &row.id,
        &row.route_api_key_id,
        &row.route_id,
        expected.identity.platform,
    )?;
    if expected != row.configuration
        || row.profile_id != authority.profile_id
        || row.configured_by != authority.actor()
        || row.credential_owner_id != authority.actor()
        || row.gateway_base != base
        || row.request_policy != policy
        || row.state != "prepared"
    {
        return Err(refused());
    }
    Ok(())
}

/// Validate the consumer's public retained definition before trusting a record
/// lookup or create response. Secrets are compared by the provider's public hash.
fn observe_record(row: &PlatformEntryGateway, body: &Value, provider: &ApiKey) -> AppResult<bool> {
    let definition = &body["definition"];
    let mut sources = row.request_policy["sources"].clone();
    for source in sources.as_array_mut().ok_or_else(refused)? {
        source["jwks_url"] = format!(
            "{}/.well-known/jwks.json",
            source["issuer"]
                .as_str()
                .ok_or_else(refused)?
                .trim_end_matches('/')
        )
        .into();
    }
    if body["record_id"] != row.id
        || definition["record_id"] != row.id
        || definition["schema_version"] != 1
        || definition["name"] != row.request_policy["name"]
        || definition["owner"] != row.request_policy["profile"]["metadata"]["owner"]
        || definition["profile"] != row.request_policy["profile"]
        || definition["profile_digest"]
            != sha256(canonical_json(row.request_policy["profile"].clone()).as_bytes())
        || definition["sources"] != sources
        || definition["partition"] != row.request_policy["partition"]
        || definition["reply"] != row.request_policy["reply"]
        || definition["provider"]["slug"] != "nyxbot"
        || body["provider"]["slug"] != "nyxbot"
        || definition["provider"]["binding_id"].as_str() != row.binding_id.as_deref()
        || body["provider"]["binding_id"].as_str() != row.binding_id.as_deref()
        || definition["secrets"]["agent_key_hash"] != format!("sha256:{}", provider.key_hash)
        || definition["connection_ref"].as_str().is_some()
        || !definition["routing"].is_null()
        || definition["creation"]["channel_id"] != body["channel_id"]
        || definition["creation"]["fingerprint"] != row.creation_fingerprint
        || body["version"].as_i64().is_none_or(|v| v <= 0)
        || definition["version"] != body["version"]
    {
        return Err(refused());
    }
    let remote = super::platform_entry_identity::gateway_channel_id(
        body["channel_id"].as_str().ok_or_else(refused)?,
    )?;
    let callback = body["endpoints"]["nyxid_callback_url"]
        .as_str()
        .ok_or_else(refused)?;
    checked_callback(&row.gateway_base, callback)?;
    if callback != format!("{}/callbacks/{remote}", row.gateway_base) {
        return Err(refused());
    }
    definition["creation"]["ready"]
        .as_bool()
        .ok_or_else(refused)
}

async fn keys_for_setup(
    state: &AppState,
    row: &PlatformEntryGateway,
) -> AppResult<(ApiKey, ApiKey)> {
    let mut result = Vec::new();
    for id in [&row.provider_api_key_id, &row.route_api_key_id] {
        let key = state
            .db
            .collection::<ApiKey>(KEYS)
            .find_one(doc! {"_id":id,"user_id":&row.credential_owner_id,"is_active":true})
            .await?
            .ok_or_else(refused)?;
        if key.purpose != crate::models::api_key::ApiKeyPurpose::SharedEntryTransport
            || !key.scopes.is_empty()
            || !key.allowed_service_ids.is_empty()
            || !key.allowed_platform_service_ids.is_empty()
            || !key.allowed_node_ids.is_empty()
            || key.allow_all_nodes
            || key.allow_all_services
            || key.allow_auto_connected_services
            || key
                .expires_at
                .is_some_and(|date| date <= chrono::Utc::now())
        {
            return Err(refused());
        }
        result.push(key);
    }
    Ok((result.remove(0), result.remove(0)))
}

async fn observe_ready(
    state: &AppState,
    authority: &super::platform_entry_admin_service::NativeConversionAuthority,
    row: PlatformEntryGateway,
    body: &Value,
    provider: &ApiKey,
    route: &ApiKey,
) -> AppResult<PreparedGateway> {
    if !observe_record(&row, body, provider)? {
        return Err(refused());
    }
    let mut completed = row.clone();
    completed.configuration.transport = EntryTransport::NyxagentGateway {
        channel_id: row.id.clone(),
        gateway_channel_id: body["channel_id"].as_str().ok_or_else(refused)?.into(),
        gateway_version: body["version"].as_i64().ok_or_else(refused)?,
    };
    completed.callback_url = body["endpoints"]["nyxid_callback_url"]
        .as_str()
        .ok_or_else(refused)?
        .into();
    completed.setup_state = "observed".into();
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((state,authority,&row,&completed,provider,route),|session,(state,authority,row,next,provider,route)|Box::pin(async move {
        super::api_key_mutation_service::transaction_result(async {
            authority.fence_in_session(&state.db,session).await?;
            fence_key(&state.db,session,provider).await?;
            fence_key(&state.db,session,route).await?;
            if state.db.collection::<Document>(GATEWAYS).update_one(setup_filter(row)?,doc! {"$set":{
                "configuration":bson::to_bson(&next.configuration).map_err(|_|refused())?,"callback_url":&next.callback_url,
                "setup_state":"observed","updated_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1{return Err(refused());}
            if super::api_key_mutation_service::update_one(&state.db,doc! {"_id":&route.id,"purpose":"shared_entry_transport","key_hash":&route.key_hash},
                doc! {"$set":{"callback_url":&next.callback_url}},Some(session)).await?.matched_count!=1{return Err(refused());}
            Ok(())
        }.await)
    })).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    let mut route = route.clone();
    route.callback_url = Some(completed.callback_url.clone());
    Ok(PreparedGateway {
        row: completed,
        provider: provider.clone(),
        route,
    })
}

async fn dispatch_setup(
    state: &AppState,
    authority: &super::platform_entry_admin_service::NativeConversionAuthority,
    row: &mut PlatformEntryGateway,
    provider: &ApiKey,
    route: &ApiKey,
) -> AppResult<Value> {
    let next_attempt = uuid::Uuid::new_v4().to_string();
    let deadline = chrono::Utc::now() + chrono::Duration::seconds(SETUP_REQUEST_SECONDS);
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((state,authority,&*row,provider,route,&next_attempt,deadline),|session,(state,authority,row,provider,route,attempt,deadline)|Box::pin(async move {
        super::api_key_mutation_service::transaction_result(async {
            authority.fence_in_session(&state.db,session).await?;
            fence_key(&state.db,session,provider).await?;
            fence_key(&state.db,session,route).await?;
            if row.setup_state=="dispatched" && row.setup_dispatch_deadline.is_none_or(|old|old+chrono::Duration::seconds(SETUP_UNCERTAINTY_SECONDS)>chrono::Utc::now()) {
                return Err(crate::errors::AppError::Conflict("Gateway setup is still in flight; inspect after its retained deadline".into()));
            }
            if state.db.collection::<Document>(GATEWAYS).update_one(setup_filter(row)?,doc! {"$set":{"setup_state":"dispatched",
                "setup_attempt_id":attempt,"setup_dispatch_deadline":bson::DateTime::from_chrono(*deadline)}}).session(&mut *session).await?.matched_count!=1{return Err(refused());}
            Ok(())
        }.await)
    })).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    row.setup_attempt_id = next_attempt;
    row.setup_dispatch_deadline = Some(deadline);
    row.setup_state = "dispatched".into();
    let remaining = (deadline - chrono::Utc::now())
        .to_std()
        .map_err(|_| refused())?;
    tokio::time::timeout(remaining, async {
        let bytes = Zeroizing::new(
            state
                .encryption_keys
                .decrypt(&row.request_ciphertext)
                .await
                .map_err(|_| refused())?,
        );
        if sha256(&bytes) != row.request_hash {
            return Err(refused());
        }
        // No await separates the exact retained-attempt check and HTTP request
        // construction; the absolute dispatch deadline also bounds send and body.
        if chrono::Utc::now() >= deadline
            || state
                .db
                .collection::<Document>(GATEWAYS)
                .find_one(setup_filter(row)?)
                .await?
                .is_none()
        {
            return Err(refused());
        }
        if chrono::Utc::now() >= deadline {
            return Err(refused());
        }
        let creator = super::nyxbot_gateway_service::creator_bearer(state, authority.actor())?;
        super::platform_entry_admin_service::provider_json(
            super::nyxbot_gateway_service::GATEWAY_HTTP_CLIENT
                .post(format!("{}/channels", row.gateway_base))
                .bearer_auth(creator.as_str())
                .header("idempotency-key", &row.id)
                .header("content-type", "application/json")
                .body(bytes.to_vec()),
        )
        .await
    })
    .await
    .map_err(|_| refused())?
}

pub(crate) async fn prepare(
    state: &AppState,
    authority: &super::platform_entry_admin_service::NativeConversionAuthority,
    recovery: Option<&GatewayRecoveryReview>,
) -> AppResult<GatewayPreparation> {
    preflight(state, authority.configuration().identity.platform).await?;
    ensure_indexes(&state.db).await?;
    let base = super::nyxbot_gateway_service::gateway_base(state).await?;
    let existing = state
        .db
        .collection::<PlatformEntryGateway>(GATEWAYS)
        .find_one(doc! {"profile_id":&authority.profile_id,"state":{"$in":["prepared","active"]}})
        .await?;
    let (mut row, provider, route) = if let Some(row) = existing {
        retained_matches(&row, authority, &base, &state.config.jwt_issuer)?;
        let (provider, route) = keys_for_setup(state, &row).await?;
        let creator = super::nyxbot_gateway_service::creator_bearer(state, authority.actor())?;
        let observed = super::nyxbot_gateway_service::gateway_call(
            state,
            reqwest::Method::GET,
            &format!("/channel-records/{}", row.id),
            &creator,
            None,
            None,
        )
        .await?;
        if !matches!(observed.status, 200 | 404) {
            return Err(refused());
        }
        let ready = if observed.status == 200 {
            observe_record(&row, &observed.body, &provider)?
        } else {
            false
        };
        if ready {
            return Ok(GatewayPreparation::Ready(Box::new(
                observe_ready(state, authority, row, &observed.body, &provider, &route).await?,
            )));
        }
        // A disappeared previously-observed record is never silently recreated.
        if row.setup_state == "observed" {
            return Err(refused());
        }
        let review = GatewayRecoveryReview {
            record_id: row.id.clone(),
            attempt_id: row.setup_attempt_id.clone(),
            revision: row.revision,
            request_hash: row.request_hash.clone(),
            credential_fingerprint: row.configuration.credential_fingerprint.clone(),
            remote_channel_id: observed.body["channel_id"].as_str().map(str::to_owned),
            remote_version: observed.body["version"].as_i64(),
            remote_ready: ready,
        };
        if recovery != Some(&review) {
            return Ok(GatewayPreparation::Recovery(review));
        }
        if row.setup_dispatch_deadline.is_some_and(|deadline| {
            deadline + chrono::Duration::seconds(SETUP_UNCERTAINTY_SECONDS) > chrono::Utc::now()
        }) {
            return Err(crate::errors::AppError::Conflict(
                "Gateway setup may still be in flight; inspect again after its retained deadline"
                    .into(),
            ));
        }
        (row, provider, route)
    } else {
        if recovery.is_some() {
            return Err(refused());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (provider, provider_raw) = transport_key(authority.actor());
        let (route, route_raw) = transport_key(authority.actor());
        let route_id = uuid::Uuid::new_v4().to_string();
        let policy = request_policy(
            &state.config.jwt_issuer,
            authority.actor(),
            &id,
            &route.id,
            &route_id,
            authority.configuration().identity.platform,
        )?;
        #[derive(serde::Serialize)]
        struct Credentials<'a> {
            agent_key: &'a str,
            channel_key: &'a str,
        }
        #[derive(serde::Serialize)]
        struct Request<'a> {
            #[serde(flatten)]
            policy: &'a Value,
            credentials: Credentials<'a>,
        }
        let request = Zeroizing::new(
            serde_json::to_vec(&Request {
                policy: &policy,
                credentials: Credentials {
                    agent_key: &provider_raw,
                    channel_key: &route_raw,
                },
            })
            .map_err(|_| refused())?,
        );
        let request_hash = sha256(&request);
        // SourceRequest serializes its optional bot_username as null. Match the
        // consumer's parsed fingerprint, rather than hashing authored JSON alone.
        let mut fingerprint_sources = policy["sources"].clone();
        for source in fingerprint_sources.as_array_mut().ok_or_else(refused)? {
            source["bot_username"] = Value::Null;
        }
        let creation_fingerprint=sha256(canonical_json(json!({
            "record_id":id,"schema_version":1,"name":policy["name"],"profile":policy["profile"],"provider":policy["provider"],
            "sources":fingerprint_sources,"partition":policy["partition"],"reply":policy["reply"],
            "agent_key":format!("sha256:{}",provider.key_hash),"channel_key":format!("sha256:{}",route.key_hash),"hmac_secrets":[]
        })).as_bytes());
        let ciphertext = state
            .encryption_keys
            .encrypt(&request)
            .await
            .map_err(|_| refused())?;
        let mut configuration = authority.configuration().clone();
        configuration.transport = EntryTransport::NyxagentGateway {
            channel_id: id.clone(),
            gateway_channel_id: format!("pending:{id}"),
            gateway_version: 1,
        };
        let now = chrono::Utc::now();
        let row = PlatformEntryGateway {
            id: id.clone(),
            profile_id: authority.profile_id.clone(),
            configuration,
            configured_by: authority.actor().into(),
            credential_owner_id: authority.actor().into(),
            route_api_key_id: route.id.clone(),
            route_id,
            provider_api_key_id: provider.id.clone(),
            callback_url: format!("{base}/callbacks/pending"),
            binding_id: Some(format!(
                "bnd_{}",
                uuid::Uuid::parse_str(&id).map_err(|_| refused())?.simple()
            )),
            gateway_base: base,
            setup_state: "prepared".into(),
            setup_attempt_id: uuid::Uuid::new_v4().to_string(),
            setup_dispatch_deadline: None,
            request_policy: policy,
            request_hash,
            creation_fingerprint,
            request_ciphertext: ciphertext,
            state: "prepared".into(),
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        let mut session = state.db.client().start_session().await?;
        session
            .start_transaction()
            .and_run(
                (state, authority, &row, &provider, &route),
                |session, (state, authority, row, provider, route)| {
                    Box::pin(async move {
                        super::api_key_mutation_service::transaction_result(
                            async {
                                authority.fence_in_session(&state.db, session).await?;
                                state
                                    .db
                                    .collection::<ApiKey>(KEYS)
                                    .insert_one(*provider)
                                    .session(&mut *session)
                                    .await?;
                                state
                                    .db
                                    .collection::<ApiKey>(KEYS)
                                    .insert_one(*route)
                                    .session(&mut *session)
                                    .await?;
                                state
                                    .db
                                    .collection::<PlatformEntryGateway>(GATEWAYS)
                                    .insert_one(*row)
                                    .session(&mut *session)
                                    .await?;
                                Ok(())
                            }
                            .await,
                        )
                    })
                },
            )
            .await
            .map_err(super::api_key_mutation_service::map_transaction_error)?;
        (row, provider, route)
    };
    let observed = dispatch_setup(state, authority, &mut row, &provider, &route).await?;
    Ok(GatewayPreparation::Ready(Box::new(
        observe_ready(state, authority, row, &observed, &provider, &route).await?,
    )))
}

pub(super) async fn inspect_pending(
    state: &AppState,
    authority: &super::platform_entry_admin_service::NativeConversionAuthority,
) -> AppResult<GatewayRecoveryReview> {
    let base = super::nyxbot_gateway_service::gateway_base(state).await?;
    let row = state
        .db
        .collection::<PlatformEntryGateway>(GATEWAYS)
        .find_one(doc! {"profile_id":&authority.profile_id,"state":"prepared"})
        .await?
        .ok_or_else(refused)?;
    retained_matches(&row, authority, &base, &state.config.jwt_issuer)?;
    let (provider, route) = keys_for_setup(state, &row).await?;
    let creator = super::nyxbot_gateway_service::creator_bearer(state, authority.actor())?;
    let response = super::nyxbot_gateway_service::gateway_call(
        state,
        reqwest::Method::GET,
        &format!("/channel-records/{}", row.id),
        &creator,
        None,
        None,
    )
    .await?;
    if !matches!(response.status, 200 | 404) {
        return Err(refused());
    }
    let ready = if response.status == 200 {
        observe_record(&row, &response.body, &provider)?
    } else {
        false
    };
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (state, authority, &row, &provider, &route),
            |session, (state, authority, row, provider, route)| {
                Box::pin(async move {
                    super::api_key_mutation_service::transaction_result(
                        async {
                            authority.fence_in_session(&state.db, session).await?;
                            fence_key(&state.db, session, provider).await?;
                            fence_key(&state.db, session, route).await?;
                            if state
                                .db
                                .collection::<Document>(GATEWAYS)
                                .update_one(
                                    setup_filter(row)?,
                                    doc! {"$inc":{"inspection_revision":1_i64}},
                                )
                                .session(&mut *session)
                                .await?
                                .matched_count
                                != 1
                            {
                                return Err(refused());
                            }
                            Ok(())
                        }
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)?;
    Ok(GatewayRecoveryReview {
        record_id: row.id,
        attempt_id: row.setup_attempt_id,
        revision: row.revision,
        request_hash: row.request_hash,
        credential_fingerprint: row.configuration.credential_fingerprint,
        remote_channel_id: response.body["channel_id"].as_str().map(str::to_owned),
        remote_version: response.body["version"].as_i64(),
        remote_ready: ready,
    })
}

/// Retired native entries retain their primary key association. This permits
/// only idempotent DELETE acknowledgement, without reopening entry authority.
pub(crate) async fn acknowledge_retired_binding(
    db: &mongodb::Database,
    key: &ApiKey,
    binding: &str,
) -> AppResult<Option<super::nyxbot_gateway_release_service::RetiredAcknowledgement>> {
    use super::nyxbot_gateway_release_service::RetiredAcknowledgement as Ack;
    if key.purpose != crate::models::api_key::ApiKeyPurpose::SharedEntryTransport {
        return Ok(None);
    }
    let rows = db.collection::<PlatformEntryGateway>(GATEWAYS);
    if rows.find_one(doc! {"provider_api_key_id":&key.id,"credential_owner_id":&key.user_id,"binding_id":binding,"state":"retired"}).await?.is_none() {return Ok(None);}
    let live=rows.find_one(doc! {"state":{"$in":["prepared","active"]},"$or":[{"provider_api_key_id":&key.id},{"binding_id":binding}]}).await?.is_some()
        || db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME).find_one(doc! {"status":{"$in":["pending","active"]},
            "$or":[{"agent_api_key_id":&key.id},{"pending_agent_api_key_id":&key.id},{"binding_id":binding}]}).await?.is_some();
    Ok(Some(if live {
        Ack::ConflictingLiveReference
    } else {
        Ack::Acknowledged
    }))
}

#[cfg(test)]
mod future_size_tests {
    use super::*;

    fn responses_future_size<F: std::future::Future>(
        _: impl FnOnce(
            &'static AppState,
            GatewayAuthority,
            &'static Value,
            Option<Arc<PlatformManagerProducers>>,
        ) -> F,
    ) -> usize {
        std::mem::size_of::<F>()
    }

    #[test]
    fn native_gateway_keeps_responses_caller_future_small() {
        let size = responses_future_size(responses);
        assert!(
            size < 1024,
            "native gateway caller future grew to {size} bytes"
        );
    }
}
