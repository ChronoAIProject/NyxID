//! Explicit voice binding; fixed provider origin and live ACL before decryption.
use crate::{
    AppState,
    errors::{AppError, AppResult},
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
    super::super::assistant_voice::validate_preferences(p)?;
    let thread = super::super::assistant_voice::thread(&state.db, user, conversation).await?;
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
        .await?
        {
            return Err(AppError::VoiceProviderUnavailable);
        }
    }
    if !official_provider_origin(&service.base_url, &protocol) {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let voice_billing = duration_billing(&p.key_source, service.billing.as_ref())?;
    authorize_inference(&state.db, &thread, &service).await?;
    let resource_owner = thread.agent_owner_id.as_deref().unwrap_or(user);
    let (mut target, class, owner, key_id, revision) = match p.key_source {
        VoiceKeySource::Platform => {
            // Paid enablement is a separate, default-off operator gate.
            if !super::super::feature_flag_service::personal_flag_enabled(
                &state.db,
                user,
                platform_flag(&protocol),
            )
            .await?
            {
                return Err(AppError::VoiceProviderUnavailable);
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
            let connection = state
                .db
                .collection::<UserService>(CONNECTIONS)
                .find_one(doc! {"_id":id,"user_id":resource_owner,
                "catalog_service_id":&service.id,"deleted_at":mongodb::bson::Bson::Null})
                .await?
                .ok_or(AppError::VoiceProviderUnavailable)?;
            if connection.node_id.is_some()
                || connection.credential_binding.as_deref() == Some("platform")
                || connection.api_key_id.is_none()
            {
                return Err(AppError::VoiceProviderUnavailable);
            }
            let snapshot = Box::pin(proxy::read_proxy_authority_snapshot_by_user_service_id(
                &state.db,
                &state.encryption_keys,
                user,
                id,
                None,
            ))
            .await?
            .ok_or(AppError::VoiceProviderUnavailable)?;
            if snapshot.node_id.is_some()
                || snapshot.master_credential
                || !official_provider_origin(&snapshot.target.base_url, &protocol)
            {
                return Err(AppError::VoiceProviderUnavailable);
            }
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
        .await?;
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
        voice,
        key,
        identity,
        billing,
        token_billing,
        protocol,
    })
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

pub(crate) async fn authorize_inference(
    db: &mongodb::Database,
    thread: &crate::models::assistant_conversation::AssistantConversation,
    service: &DownstreamService,
) -> AppResult<()> {
    let key =
        super::super::key_service::get_api_key(db, &thread.user_id, &thread.credential_api_key_id)
            .await?;
    let auth = crate::mw::auth::api_key_auth_user(db, &key, None, None, None).await?;
    auth.ensure_llm_proxy_access()?;
    if auth
        .api_key_service_scope()
        .is_some_and(|ids| !ids.contains(&service.id))
    {
        return Err(AppError::Forbidden(
            "This agent cannot use the selected voice service".into(),
        ));
    }
    let grok = service
        .inference
        .as_ref()
        .and_then(|i| i.voice.as_ref())
        .is_some_and(|v| v.protocol == VoiceProtocol::XaiRealtime);
    let method = if grok { "GET" } else { "POST" };
    let path = super::super::proxy_authorization::CanonicalPath::from_mcp_literal(if grok {
        "/realtime"
    } else {
        "/live/sessions"
    })?;
    super::super::proxy_authorization::authorize_proxy_operation(service, method, &path)?;
    super::super::agent_operation_scope_service::authorize(
        &auth.assistant_operation_scopes,
        &service.id,
        Some(&service.id),
        None,
        method,
        &path,
        false,
        grok,
    )?;
    if !auth.assistant_operation_scopes.is_empty() {
        Box::pin(
            super::super::agent_operation_scope_service::check_non_mcp_context(
                db,
                &auth,
                &service.id,
                Some(&service.id),
                method,
                &path,
            ),
        )
        .await?;
    }
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
