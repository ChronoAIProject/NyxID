//! Explicit voice binding; fixed provider origin and live ACL before decryption.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_voice::{VoiceKeySource, VoicePreferences},
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
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
    if !official_origin(&service.base_url) {
        return Err(AppError::VoiceProviderUnavailable);
    }
    let voice_billing = duration_billing(&p.key_source, service.billing.as_ref())?;
    authorize_inference(state, &thread, &service).await?;
    let resource_owner = thread.agent_owner_id.as_deref().unwrap_or(user);
    let (mut target, class, owner, key_id, revision) = match p.key_source {
        VoiceKeySource::Platform => {
            // Paid enablement is a separate, default-off operator gate.
            if !super::super::feature_flag_service::personal_flag_enabled(
                &state.db,
                user,
                super::super::feature_flag_service::VOICE_OPENAI_PLATFORM_FLAG_KEY,
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
                || !official_origin(&snapshot.target.base_url)
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
                || !official_origin(&resolved.target.base_url)
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
        payer.owner_id,
        user.into(),
        key_id,
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
    let identity = keyed_fingerprint(
        state,
        b"session-identity",
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

fn official_origin(base: &str) -> bool {
    url::Url::parse(base).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("api.openai.com")
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
            && matches!(u.path(), "" | "/" | "/v1" | "/v1/")
    })
}

async fn authorize_inference(
    state: &AppState,
    thread: &crate::models::assistant_conversation::AssistantConversation,
    service: &DownstreamService,
) -> AppResult<()> {
    let key = super::super::key_service::get_api_key(
        &state.db,
        &thread.user_id,
        &thread.credential_api_key_id,
    )
    .await?;
    let auth = crate::mw::auth::api_key_auth_user(&state.db, &key, None, None, None).await?;
    auth.ensure_llm_proxy_access()?;
    if auth
        .api_key_service_scope()
        .is_some_and(|ids| !ids.contains(&service.id))
    {
        return Err(AppError::Forbidden(
            "This agent cannot use the selected voice service".into(),
        ));
    }
    let path =
        super::super::proxy_authorization::CanonicalPath::from_mcp_literal("/live/sessions")?;
    super::super::proxy_authorization::authorize_proxy_operation(service, "POST", &path)?;
    super::super::agent_operation_scope_service::authorize(
        &auth.assistant_operation_scopes,
        &service.id,
        Some(&service.id),
        None,
        "POST",
        &path,
        false,
        false,
    )?;
    if !auth.assistant_operation_scopes.is_empty() {
        Box::pin(
            super::super::agent_operation_scope_service::check_non_mcp_context(
                &state.db,
                &auth,
                &service.id,
                Some(&service.id),
                "POST",
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
