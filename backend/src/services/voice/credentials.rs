//! Explicit voice binding; fixed provider origin and live ACL before decryption.
use crate::{
    AppState,
    errors::{AppError, AppResult, voice_start::Stage},
    models::{
        assistant_voice::{VoiceKeySource, VoicePreferences},
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService, VoiceProtocol},
        service_billing::{BillingMetric, ServiceBilling},
        usage_meter::CredentialClass,
        user_service::{COLLECTION_NAME as CONNECTIONS, UserService},
    },
    services::{
        billing::{BillingIngress, BillingRouteContext, NodeIntent},
        proxy_service as proxy,
    },
};
use mongodb::bson::doc;
use sha2::Sha256;
use zeroize::Zeroizing;

pub struct Resolved {
    pub concurrency_policy: Option<crate::models::service_concurrency::ServiceConcurrencyPolicy>,
    pub key: Zeroizing<String>,
    pub identity: String,
    pub voice: String,
    pub billing: BillingRouteContext,
    pub token_billing: BillingRouteContext,
    pub protocol: VoiceProtocol,
}

pub async fn resolve(
    state: &AppState,
    user: &str,
    conversation: &str,
    p: &VoicePreferences,
) -> AppResult<Resolved> {
    resolve_audio(state, user, conversation, p, false).await
}

pub(crate) async fn resolve_audio(
    state: &AppState,
    user: &str,
    conversation: &str,
    p: &VoicePreferences,
    dictation: bool,
) -> AppResult<Resolved> {
    super::super::assistant_voice::validate_preferences(p)?;
    let thread = super::super::assistant_voice::thread(&state.db, user, conversation)
        .await
        .map_err(|e| Stage::Thread.error(e))?;
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&p.service_id,"is_active":true})
        .await?
        .ok_or(AppError::VoiceProviderUnavailable)?;
    let metadata = service
        .inference
        .as_ref()
        .and_then(|i| i.voice.as_ref())
        .ok_or(AppError::VoiceProviderUnavailable)?;
    let voice = super::selected_voice(metadata, &p.model, p.voice.as_deref())?;
    let protocol = metadata.protocol;
    if protocol == VoiceProtocol::XaiRealtime {
        if p.input_mode != crate::models::assistant_voice::VoiceInputMode::PushToTalk {
            return Err(AppError::ValidationError("Grok beta requires Hold to talk with headphones; automatic speaker mode is not verified".into()));
        }
        if !super::super::feature_flag_service::personal_flag_enabled(
            &state.db,
            user,
            super::super::feature_flag_service::VOICE_GROK_FLAG_KEY,
        )
        .await
        .map_err(|e| Stage::Flag.error(e))?
        {
            return Err(Stage::Flag.error(AppError::VoiceProviderUnavailable));
        }
    }
    if !official_provider_origin(&service.base_url, &protocol) {
        return Err(Stage::Origin.error(AppError::VoiceProviderUnavailable));
    }
    let voice_billing = duration_billing(&p.key_source, service.billing.as_ref())
        .map_err(|e| Stage::BillingReservation.error(e))?;
    let selected_connection = (p.key_source == VoiceKeySource::Own)
        .then_some(p.connection_id.as_deref())
        .flatten();
    let authority = inference_authority(&state.db, &thread).await?;
    authorize_operation(
        &state.db,
        &authority,
        &service,
        selected_connection,
        dictation,
    )
    .await?;
    let resource_owner = thread.agent_owner_id.as_deref().unwrap_or(user);
    let (mut target, class, owner, key_id, revision) = match p.key_source {
        VoiceKeySource::Platform => {
            // Paid enablement is a separate, default-off operator gate.
            if !super::super::feature_flag_service::personal_flag_enabled(
                &state.db,
                user,
                platform_flag(&protocol),
            )
            .await
            .map_err(|e| Stage::Flag.error(e))?
            {
                return Err(Stage::Flag.error(AppError::VoiceProviderUnavailable));
            }
            let revision =
                keyed_fingerprint(state, b"credential-revision", &service.credential_encrypted);
            let target = Box::pin(proxy::resolve_catalog_platform_target(
                &state.db,
                &state.encryption_keys,
                user,
                service.clone(),
                state.platform_user_rate_limit,
            ))
            .await?;
            (
                target,
                CredentialClass::NyxidManagedMaster,
                user.to_owned(),
                None,
                revision,
            )
        }
        VoiceKeySource::Own => {
            let id = p
                .connection_id
                .as_deref()
                .ok_or(AppError::VoiceProviderUnavailable)?;
            Box::pin(own_snapshot(
                &state.db,
                &state.encryption_keys,
                &thread,
                &service,
                id,
            ))
            .await?;
            let resolved = Box::pin(proxy::resolve_proxy_target_by_user_service_id(
                &state.db,
                &state.encryption_keys,
                user,
                id,
                None,
                Some(&service.id),
                proxy::ProxyExecutionContext::new(
                    Some(&state.connection_expiry_notifier),
                    state.platform_user_rate_limit,
                ),
            ))
            .await?
            .ok_or(AppError::VoiceProviderUnavailable)?;
            if resolved.node_id.is_some()
                || resolved.master_credential
                || !official_provider_origin(&resolved.target.base_url, &protocol)
            {
                return Err(AppError::VoiceProviderUnavailable);
            }
            let revision = format!(
                "{}:{}",
                resolved.api_key_id.as_deref().unwrap_or_default(),
                resolved.credential_epoch
            );
            (
                resolved.target,
                CredentialClass::UserOwned,
                resource_owner.to_owned(),
                resolved.api_key_id,
                revision,
            )
        }
    };
    let key = Zeroizing::new(std::mem::take(&mut target.credential));
    if target.auth_method != "bearer" || key.is_empty() {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let payer = state
        .billing
        .owner_resolver()
        .resolve_for_execution(user, &owner, class)
        .await
        .map_err(|e| Stage::BillingReservation.error(e))?;
    let billing = BillingRouteContext::new(
        BillingIngress::LlmProvider,
        uuid::Uuid::new_v4().to_string(),
        payer.owner_id.clone(),
        user.into(),
        key_id.clone(),
        p.connection_id.clone(),
        Some(service.id.clone()),
        Some(service.slug.clone()),
        NodeIntent::Direct,
        "bearer".into(),
        class,
        BillingMetric::VoiceSeconds,
        voice_billing,
        false,
    );
    let token_billing = BillingRouteContext::new(
        BillingIngress::LlmProvider,
        uuid::Uuid::new_v4().to_string(),
        payer.owner_id,
        user.into(),
        key_id,
        p.connection_id.clone(),
        Some(service.id.clone()),
        Some(service.slug.clone()),
        NodeIntent::Direct,
        "bearer".into(),
        class,
        BillingMetric::Tokens,
        service.billing.as_ref().filter(|b| match p.key_source {
            VoiceKeySource::Own => b.byok_pricing.is_some(),
            VoiceKeySource::Platform => b.platform_key_pricing.is_some(),
        }),
        false,
    );
    let identity = keyed_fingerprint(
        state,
        if protocol == VoiceProtocol::XaiRealtime {
            b"session-identity-xai-realtime"
        } else {
            b"session-identity"
        },
        format!(
            "{}:{}:{}:{}:{}",
            thread.credential_api_key_id,
            service.id,
            owner,
            p.connection_id.as_deref().unwrap_or("platform"),
            revision
        )
        .as_bytes(),
    );
    Ok(Resolved {
        concurrency_policy: service.concurrency_policy.clone(),
        voice,
        key,
        identity,
        billing,
        token_billing,
        protocol,
    })
}

/// Shared read-only BYOK eligibility for discovery and start. Never decrypts,
/// refreshes, provisions or touches a credential, and preserves snapshot errors.
pub(crate) async fn own_snapshot(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    thread: &crate::models::assistant_conversation::AssistantConversation,
    service: &DownstreamService,
    id: &str,
) -> AppResult<proxy::UserServiceResolution> {
    let owner = thread.agent_owner_id.as_deref().unwrap_or(&thread.user_id);
    let connection = db
        .collection::<UserService>(CONNECTIONS)
        .find_one(doc! {"_id":id,"user_id":owner,"is_active":true,
        "catalog_service_id":&service.id,"deleted_at":mongodb::bson::Bson::Null})
        .await?
        .ok_or(AppError::VoiceProviderUnavailable)?;
    if connection.node_id.is_some()
        || connection.credential_binding.as_deref() == Some("platform")
        || connection.api_key_id.is_none()
        || connection.auth_method != "bearer"
    {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let snapshot = Box::pin(proxy::read_proxy_authority_snapshot_by_user_service_id(
        db,
        keys,
        &thread.user_id,
        id,
        None,
    ))
    .await?
    .ok_or(AppError::VoiceProviderUnavailable)?;
    let protocol = &service
        .inference
        .as_ref()
        .and_then(|i| i.voice.as_ref())
        .ok_or(AppError::VoiceProviderUnavailable)?
        .protocol;
    if snapshot.node_id.is_some()
        || snapshot.master_credential
        || !snapshot.has_server_credential
        || snapshot.target.auth_method != "bearer"
        || !official_provider_origin(&snapshot.target.base_url, protocol)
    {
        return Err(AppError::VoiceProviderUnavailable);
    }
    Ok(snapshot)
}

/// Only expected authority/credential refusals hide an option. Infrastructure
/// and data-integrity errors must remain errors, never eligible discoveries.
pub(crate) fn discovery_result<T>(result: AppResult<T>) -> AppResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(
            AppError::Forbidden(_)
            | AppError::Unauthorized(_)
            | AppError::NotFound(_)
            | AppError::ApiKeyScopeForbidden(_)
            | AppError::OrgRoleInsufficient(_)
            | AppError::CredentialUnavailable(_)
            | AppError::VoiceProviderUnavailable,
        ) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Keyed, domain-separated fingerprint for change detection. Never a password
/// hash: inputs may include credential ciphertext, so an unkeyed digest is not used.
fn keyed_fingerprint(state: &AppState, domain: &[u8], material: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(state.audit_chain_hmac_key.as_slice())
        .expect("HMAC accepts any key length");
    mac.update(b"nyxid-voice-");
    mac.update(domain);
    mac.update(b"\0");
    mac.update(material);
    hex::encode(mac.finalize().into_bytes())
}

pub fn platform_flag(protocol: &VoiceProtocol) -> &'static str {
    if *protocol == VoiceProtocol::XaiRealtime {
        super::super::feature_flag_service::VOICE_GROK_PLATFORM_FLAG_KEY
    } else {
        super::super::feature_flag_service::VOICE_OPENAI_PLATFORM_FLAG_KEY
    }
}
pub fn official_provider_origin(base: &str, protocol: &VoiceProtocol) -> bool {
    url::Url::parse(base).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str()
                == Some(if *protocol == VoiceProtocol::XaiRealtime {
                    "api.x.ai"
                } else {
                    "api.openai.com"
                })
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
            && matches!(u.path(), "" | "/" | "/v1" | "/v1/")
    })
}

/// A request-local, exact private-thread authentication result. Only the loader
/// can construct it; reuse in discovery never persists beyond that request.
pub(crate) struct InferenceAuthority(crate::mw::auth::AuthUser);

pub(crate) async fn inference_authority(
    db: &mongodb::Database,
    thread: &crate::models::assistant_conversation::AssistantConversation,
) -> AppResult<InferenceAuthority> {
    super::super::assistant_voice::private_thread(thread)?;
    let key =
        super::super::key_service::get_api_key(db, &thread.user_id, &thread.credential_api_key_id)
            .await?;
    let auth = crate::mw::auth::api_key_auth_user(db, &key, None, None, None).await?;
    auth.ensure_llm_proxy_access()?;
    // Model transport uses the normal assistant inference exception, bounded
    // here to this exact private thread and its live durable credential. A draft
    // needs no active turn. Ordinary/unbound/rebound keys cannot use this path.
    let chat = auth.assistant_chat.as_ref().ok_or_else(|| {
        AppError::Forbidden("Voice requires the thread's live conversation credential".into())
    })?;
    if chat.guest
        || chat.user_id != thread.user_id
        || chat.conversation_id != thread.id
        || chat.api_key_id != thread.credential_api_key_id
        || match thread.agent_id.as_deref() {
            Some(id) => chat.agent_id != id,
            // Legacy threads belong to the person's NyxBot; auth resolves that
            // same thread's agent before constructing this authority.
            None => chat.role != crate::models::assistant_conversation::AgentRole::Orchestrator,
        }
    {
        return Err(AppError::Forbidden(
            "Voice conversation credential does not match this thread".into(),
        ));
    }
    Ok(InferenceAuthority(auth))
}

pub(crate) async fn authorize_with_authority(
    db: &mongodb::Database,
    authority: &InferenceAuthority,
    service: &DownstreamService,
    connection: Option<&str>,
) -> AppResult<()> {
    authorize_operation(db, authority, service, connection, false).await
}

pub(crate) async fn authorize_dictation(
    db: &mongodb::Database,
    authority: &InferenceAuthority,
    service: &DownstreamService,
    connection: Option<&str>,
) -> AppResult<()> {
    authorize_operation(db, authority, service, connection, true).await
}

async fn authorize_operation(
    db: &mongodb::Database,
    authority: &InferenceAuthority,
    service: &DownstreamService,
    connection: Option<&str>,
    dictation: bool,
) -> AppResult<()> {
    let auth = &authority.0;
    // Platform audio is inference, independent of tool grants. BYOK must hold
    // the selected UserService grant, never a catalog-ID substitute.
    if connection.is_some_and(|id| {
        auth.api_key_service_scope()
            .is_some_and(|ids| !ids.iter().any(|allowed| allowed == id))
    }) {
        return Err(AppError::Forbidden(
            "This agent cannot use the selected voice service".into(),
        ));
    }
    let grok = service
        .inference
        .as_ref()
        .and_then(|i| i.voice.as_ref())
        .is_some_and(|v| v.protocol == VoiceProtocol::XaiRealtime);
    let method = if grok || dictation { "GET" } else { "POST" };
    let path =
        super::super::proxy_authorization::CanonicalPath::from_mcp_literal(if grok || dictation {
            "/realtime"
        } else {
            "/live/sessions"
        })?;
    super::super::proxy_authorization::authorize_proxy_operation(service, method, &path)?;
    // Resolve the selected connection's identity like proxy/LLM do, so a scope
    // saved on that connection applies; catalog-only intersects every scope
    // bound to the catalog.
    let (scope_id, catalog_id) = super::super::agent_operation_scope_service::execution_identity(
        auth,
        connection,
        &service.id,
    );
    let scoped = super::super::agent_operation_scope_service::is_scoped(
        &auth.assistant_operation_scopes,
        scope_id,
        catalog_id,
    );
    super::super::agent_operation_scope_service::authorize(
        &auth.assistant_operation_scopes,
        scope_id,
        catalog_id,
        None,
        method,
        &path,
        false,
        true, // Both adapters open a WebSocket (OpenAI attaches a sideband).
    )?;
    // Voice sessions send no caller query or body; any input limit refuses them.
    super::super::agent_operation_scope_service::check_inputs(
        &auth.assistant_operation_scopes,
        scope_id,
        catalog_id,
        None,
        method,
        &path,
        None,
        &[],
        None,
    )?;
    Box::pin(
        super::super::agent_operation_scope_service::check_non_mcp_context(
            db,
            auth,
            scoped,
            scope_id,
            catalog_id,
            method,
            Some(&path),
        ),
    )
    .await?;
    Ok(())
}

/// Shared admission/quote policy. BYOK without an authored duration price meters
/// seconds without legacy fallback; any authored primary/duration must be synced.
pub fn duration_billing<'a>(
    source: &VoiceKeySource,
    billing: Option<&'a ServiceBilling>,
) -> AppResult<Option<&'a ServiceBilling>> {
    use crate::models::service_billing::PricingSyncStatus;
    if billing.is_some_and(|b| b.resale_billable) {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let lane = billing.and_then(|b| match source {
        VoiceKeySource::Own => b.byok_pricing.as_ref(),
        VoiceKeySource::Platform => b.platform_key_pricing.as_ref(),
    });
    if *source == VoiceKeySource::Own
        && lane.is_none_or(|p| {
            p.sync_status == PricingSyncStatus::Synced
                && p.metric != BillingMetric::VoiceSeconds
                && !p
                    .components
                    .iter()
                    .any(|c| c.metric == BillingMetric::VoiceSeconds)
        })
    {
        return Ok(None);
    }
    if lane.is_some_and(|p| {
        p.sync_status == PricingSyncStatus::Synced
            && (p.metric == BillingMetric::VoiceSeconds
                || p.components.iter().any(|c| {
                    c.metric == BillingMetric::VoiceSeconds
                        && c.sync_status == PricingSyncStatus::Synced
                }))
    }) {
        Ok(billing)
    } else {
        Err(AppError::VoiceProviderUnavailable)
    }
}
