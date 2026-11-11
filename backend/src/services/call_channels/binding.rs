//! Live connection/platform ACLs, provider resource proof and tenant allocation.
use super::{profiles, protocol};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AgentKind, AssistantAgent},
        channel_adapter::*,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        user_service::{COLLECTION_NAME as CONNECTIONS, UserService},
    },
};
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub struct Resolved {
    pub key: Zeroizing<String>,
    pub identity: String,
    pub service: DownstreamService,
    pub class: crate::models::usage_meter::CredentialClass,
    pub resource_owner: String,
    pub connection_id: Option<String>,
}
impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResolvedAdapterCredential { [REDACTED] }")
    }
}
fn unavailable() -> AppError {
    AppError::NotFound("Adapter connection unavailable".into())
}

pub async fn resolve(
    state: &AppState,
    actor: &str,
    credential: &CredentialBinding,
    profile: Profile,
) -> AppResult<Resolved> {
    use crate::models::usage_meter::CredentialClass;
    use crate::services::proxy_service as proxy;
    let (mut target, service, class, resource_owner, connection_id, identity) = match credential {
        CredentialBinding::Platform { catalog_service_id } => {
            let service = state
                .db
                .collection::<DownstreamService>(SERVICES)
                .find_one(doc! {"_id":catalog_service_id,"is_active":true})
                .await?
                .ok_or_else(unavailable)?;
            if service.platform_key.as_ref().is_none_or(|p| !p.enabled) {
                return Err(unavailable());
            }
            let target = Box::pin(proxy::resolve_catalog_platform_target(
                &state.db,
                &state.encryption_keys,
                actor,
                service.clone(),
                state.platform_user_rate_limit,
            ))
            .await?;
            let identity = crate::services::audit_service::keyed_fingerprint(
                crate::services::audit_service::audit_chain_hmac_key(),
                b"channel-adapter-credential-v1",
                &service.credential_encrypted,
            )
            .ok_or_else(unavailable)?;
            (
                target,
                service,
                CredentialClass::NyxidManagedMaster,
                actor.to_owned(),
                None,
                identity,
            )
        }
        CredentialBinding::Connection { user_service_id } => {
            let connection = state
                .db
                .collection::<UserService>(CONNECTIONS)
                .find_one(
                    doc! {"_id":user_service_id,"is_active":true,"deleted_at":bson::Bson::Null},
                )
                .await?
                .ok_or_else(unavailable)?;
            if connection.node_id.is_some()
                || connection.credential_binding.as_deref() == Some("platform")
            {
                return Err(unavailable());
            }
            let catalog_id = connection
                .catalog_service_id
                .as_deref()
                .ok_or_else(unavailable)?;
            let service = state
                .db
                .collection::<DownstreamService>(SERVICES)
                .find_one(doc! {"_id":catalog_id,"is_active":true})
                .await?
                .ok_or_else(unavailable)?;
            let resolved = Box::pin(proxy::resolve_proxy_target_by_user_service_id(
                &state.db,
                &state.encryption_keys,
                actor,
                user_service_id,
                None,
                Some(catalog_id),
                proxy::ProxyExecutionContext::new(
                    Some(&state.connection_expiry_notifier),
                    state.platform_user_rate_limit,
                ),
            ))
            .await?
            .ok_or_else(unavailable)?;
            if resolved.master_credential || resolved.node_id.is_some() {
                return Err(unavailable());
            }
            let identity = format!(
                "{}:{}",
                resolved.api_key_id.as_deref().unwrap_or_default(),
                resolved.credential_epoch
            );
            (
                resolved.target,
                service,
                CredentialClass::UserOwned,
                connection.user_id,
                Some(user_service_id.clone()),
                identity,
            )
        }
    };
    let parsed = url::Url::parse(&target.base_url).map_err(|_| unavailable())?;
    let expected = match profiles::provider(profile) {
        "telnyx" => "api.telnyx.com",
        "twilio" => "api.twilio.com",
        _ => "api.elevenlabs.io",
    };
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(expected)
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/" | "/v2" | "/v2/" | "/v1" | "/v1/")
        || service
            .billing
            .as_ref()
            .is_some_and(|b| b.managed_execution.is_some())
    {
        return Err(unavailable());
    }
    let key = std::mem::take(&mut target.credential);
    let method_ok = match profiles::provider(profile) {
        "twilio" => target.auth_method == "basic",
        "telnyx" => target.auth_method == "bearer",
        _ => matches!(target.auth_method.as_str(), "header" | "api_key"),
    };
    if key.is_empty() || !method_ok {
        return Err(unavailable());
    }
    Ok(Resolved {
        key,
        identity,
        service,
        class,
        resource_owner,
        connection_id,
    })
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupInput {
    pub adapter_id: String,
    pub contract_version: u32,
    pub label: String,
    pub agent_id: String,
    pub credential: CredentialBinding,
    pub account_id: String,
    pub resource_id: String,
    pub phone_number: String,
    pub application_id: String,
    pub webhook_public_key: Option<String>,
    pub voice: String,
    pub language: String,
}

pub async fn validate_agent(state: &AppState, owner: &str, id: &str) -> AppResult<AssistantAgent> {
    let agent = crate::services::assistant_team_service::agent(&state.db, owner, id).await?;
    if agent.user_id != owner || agent.kind != AgentKind::Specialist || agent.destroyed_at.is_some()
    {
        return Err(AppError::ValidationError("Telephone and speech callers require your active specialist. Grant only the FAQ service and set its Guest access to Read.".into()));
    }
    Ok(agent)
}

pub async fn load(state: &AppState, id: &str) -> AppResult<Binding> {
    state
        .db
        .collection::<Binding>(BINDINGS)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)
}

/// Cleanup receipts may outlive disable, but cannot recreate retained data
/// after the binding has been removed or the owner cascade has purged it.
pub(super) async fn retain_receipt(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    b: &Binding,
) -> AppResult<()> {
    let retained = db
        .collection::<Binding>(BINDINGS)
        .update_one(
            doc! {"_id":&b.id,"user_id":&b.user_id},
            doc! {"$inc":{"receipt_fence":1_i64}},
        )
        .session(tx)
        .await?;
    if retained.matched_count != 1 {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(())
}
pub async fn live(state: &AppState, binding: &Binding) -> AppResult<Resolved> {
    require_enabled(&state.db, &binding.user_id).await?;
    read_authority(&state.db, binding).await?;
    let current = load(state, &binding.id).await?;
    if !current.enabled
        || current.revision != binding.revision
        || current.agent_id != binding.agent_id
    {
        return Err(unavailable());
    }
    crate::services::assistant_nyxagent::require_enabled(&state.db, &binding.user_id).await?;
    validate_agent(state, &binding.user_id, &binding.agent_id).await?;
    let resolved = resolve(
        state,
        &binding.user_id,
        &binding.credential,
        binding.profile,
    )
    .await?;
    if resolved.identity != binding.verified_credential_identity {
        return Err(AppError::Conflict(
            "Credential changed; verify adapter setup again".into(),
        ));
    }
    if let CredentialBinding::Platform { catalog_service_id } = &binding.credential {
        allocation(state, binding, catalog_service_id).await?;
        let account = trusted_account(
            state,
            catalog_service_id,
            &binding.account_id,
            binding.profile,
        )
        .await?;
        if binding.trusted_account_revision != Some(account.revision)
            || binding.webhook_public_key != account.webhook_public_key
        {
            return Err(AppError::Conflict(
                "Platform account verification changed; verify adapter setup again".into(),
            ));
        }
    }
    Ok(resolved)
}

pub async fn allocation(
    state: &AppState,
    b: &Binding,
    catalog: &str,
) -> AppResult<ResourceAllocation> {
    let allocation = state.db.collection::<ResourceAllocation>(ALLOCATIONS).find_one(doc!{"catalog_service_id":catalog,"user_id":&b.user_id,"provider":profiles::provider(b.profile),"account_id":&b.account_id,"resource_id":&b.resource_id,"phone_number":&b.phone_number,"application_id":&b.application_id}).await?.ok_or_else(||AppError::Forbidden("This provider resource is not allocated to you".into()))?;
    Ok(allocation)
}

pub async fn create(state: &AppState, owner: &str, input: SetupInput) -> AppResult<Binding> {
    require_enabled(&state.db, owner).await?;
    let adapter = crate::services::channel_adapters::resolve_nyxbot_profile(
        &input.adapter_id,
        &state.token_exchange_cache,
    )?;
    if input.contract_version != 1
        || input.label.is_empty()
        || input.label.len() > 80
        || !protocol::handle(&input.account_id)
        || !protocol::handle(&input.resource_id)
        || !protocol::handle(&input.application_id)
        || input.voice.len() > 128
        || input.language.is_empty()
        || input.language.len() > 32
        || (!matches!(adapter.profile(), Profile::ElevenlabsCustomLlm)
            && !protocol::phone(&input.phone_number))
    {
        return Err(AppError::ValidationError("Invalid adapter setup".into()));
    }
    validate_agent(state, owner, &input.agent_id).await?;
    let resolved = resolve(state, owner, &input.credential, adapter.profile()).await?;
    let now = chrono::Utc::now();
    let mut b = Binding {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: owner.into(),
        profile: adapter.profile(),
        contract_version: 1,
        revision: 1,
        label: input.label,
        agent_id: input.agent_id,
        credential: input.credential,
        account_id: input.account_id,
        resource_id: input.resource_id,
        phone_number: input.phone_number,
        application_id: input.application_id,
        webhook_public_key: input.webhook_public_key,
        voice: input.voice,
        language: input.language,
        enabled: true,
        verified_credential_identity: resolved.identity.clone(),
        trusted_account_revision: None,
        verified_at: now,
        created_at: now,
        updated_at: now,
    };
    if let CredentialBinding::Platform { catalog_service_id } = &b.credential {
        allocation(state, &b, catalog_service_id).await?;
        let account = trusted_account(state, catalog_service_id, &b.account_id, b.profile).await?;
        if b.webhook_public_key.is_some() && b.webhook_public_key != account.webhook_public_key {
            return Err(AppError::Forbidden(
                "Webhook verification key does not match platform account".into(),
            ));
        }
        b.trusted_account_revision = Some(account.revision);
        b.webhook_public_key = account.webhook_public_key;
    }
    if profiles::provider(b.profile) == "telnyx" {
        protocol::validate_public_key(b.webhook_public_key.as_deref().ok_or_else(unavailable)?)?;
    }
    validate_voice(&b)?;
    for path in resource_paths(&b) {
        authorize(state, &b, &resolved, "GET", &path, None).await?;
    }
    verify_resources(
        crate::services::proxy_service::no_redirect_http_client(),
        &b,
        &resolved.key,
        None,
    )
    .await?;
    persist(state, &b).await?;
    super::management::audit(
        state,
        owner,
        "channel_adapter_binding_created",
        serde_json::json!({"binding_id":b.id,"revision":b.revision,"agent_id":b.agent_id,"adapter_id":profiles::id(b.profile)}),
    );
    Ok(b)
}
pub(crate) fn duplicate(e: &mongodb::error::Error) -> bool {
    e.to_string().contains("E11000")
}

pub(crate) async fn verify_resources(
    http: &reqwest::Client,
    b: &Binding,
    key: &str,
    fixture: Option<&str>,
) -> AppResult<()> {
    let origin = if fixture.is_some() {
        profiles::origin(b.profile, fixture)?
    } else {
        profiles::binding_origin(b)?
    };
    let paths = resource_paths(b);
    for (index, path) in paths.iter().enumerate() {
        let mut request = http
            .get(profiles::provider_url(b.profile, &origin, path)?)
            .timeout(std::time::Duration::from_secs(15));
        request = match profiles::provider(b.profile) {
            "twilio" => {
                let (a, k) = protocol::twilio_key(key, &b.account_id)?;
                request.basic_auth(a.to_string(), Some(k))
            }
            "telnyx" => request.bearer_auth(key),
            _ => request.header("xi-api-key", key),
        };
        let response = request.send().await.map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let bytes = profiles::bounded_response(response).await?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        let valid = match profiles::provider(b.profile) {
            "twilio" => {
                value["account_sid"] == b.account_id
                    && (profiles::sms(b.profile)
                        || value["voice_fallback_url"].is_null()
                        || value["voice_fallback_url"] == "")
                    && if index == 0 {
                        value["sid"] == b.resource_id
                            && value["phone_number"] == b.phone_number
                            && if profiles::sms(b.profile) {
                                value["sms_application_sid"] == b.application_id
                            } else {
                                value["voice_application_sid"] == b.application_id
                            }
                    } else {
                        value["sid"] == b.application_id
                    }
            }
            "telnyx" => {
                if index == 0 {
                    value["data"]["id"] == b.resource_id
                        && value["data"]["phone_number"] == b.phone_number
                        && if profiles::sms(b.profile) {
                            value["data"]["messaging_profile_id"] == b.application_id
                        } else {
                            value["data"]["connection_id"] == b.application_id
                        }
                } else {
                    value["data"]["id"] == b.application_id
                }
            }
            _ => value["agent_id"] == b.resource_id,
        };
        if !valid {
            return Err(AppError::Forbidden(
                "Provider account, number or application did not match".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    Disabled,
    ConnectionRequired,
    VerificationRequired,
    ScopeMissing,
    ResourcePolicyUnresolved,
    BillingUnavailable,
    ResourceNotAllocated,
    UnsupportedOperation,
    ProviderLimitUnavailable,
    ExternalSpeechBillingRequired,
}
#[derive(Clone, Debug, Serialize)]
pub struct OperationAvailability {
    pub operation: crate::services::channel_capability::Operation,
    pub implemented: bool,
    pub available: bool,
    pub reason: Option<UnavailableReason>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Availability {
    pub operations: Vec<OperationAvailability>,
    pub adapter_id: String,
    pub contract_version: u32,
    pub binding_revision: i64,
    pub available: bool,
    pub reason: Option<UnavailableReason>,
}
pub async fn availability(state: &AppState, b: &Binding) -> Availability {
    let reason = if b.profile == Profile::TelnyxRelay
        && matches!(b.credential, CredentialBinding::Platform { .. })
    {
        Some(UnavailableReason::ProviderLimitUnavailable)
    } else if b.profile == Profile::ElevenlabsCustomLlm
        && matches!(b.credential, CredentialBinding::Platform { .. })
    {
        Some(UnavailableReason::ExternalSpeechBillingRequired)
    } else if require_enabled(&state.db, &b.user_id).await.is_err()
        || crate::services::assistant_nyxagent::require_enabled(&state.db, &b.user_id)
            .await
            .is_err()
        || validate_agent(state, &b.user_id, &b.agent_id)
            .await
            .is_err()
        || !b.enabled
    {
        Some(UnavailableReason::Disabled)
    } else {
        match read_authority(&state.db, b).await {
            Ok(resolved) => super::billing::validate(state, b, &resolved)
                .await
                .err()
                .map(|_| UnavailableReason::BillingUnavailable),
            Err(AppError::Forbidden(_)) => Some(UnavailableReason::ResourceNotAllocated),
            Err(AppError::Conflict(_)) => Some(UnavailableReason::VerificationRequired),
            Err(_) => Some(UnavailableReason::ConnectionRequired),
        }
    };
    let descriptor = profiles::descriptor(b.profile);
    use crate::services::channel_capability::Operation as O;
    let mut operations = Vec::new();
    let authority = read_authority(&state.db, b).await.ok();
    for operation in [
        O::ReceiveMessage,
        O::SendMessage,
        O::EditMessage,
        O::ReceiveCall,
        O::EndSession,
        O::ReceiveDtmf,
        O::StreamReply,
        O::Interrupt,
        O::Dial,
        O::Transfer,
        O::PlayAudio,
    ] {
        let implemented = descriptor.operations.contains(&operation);
        let mut op_reason = if implemented {
            reason.clone()
        } else {
            Some(UnavailableReason::UnsupportedOperation)
        };
        if implemented
            && op_reason.is_none()
            && let Some(r) = &authority
        {
            if profiles::relay(b.profile)
                && matches!(operation, O::ReceiveCall | O::EndSession)
                && r.service.proxy_operation_policy.is_some()
            {
                op_reason = Some(UnavailableReason::ResourcePolicyUnresolved);
            }
            let paths = operation_paths(b, operation);
            for (method, path) in paths {
                let base = url::Url::parse(&r.service.base_url)
                    .ok()
                    .map(|u| u.path().trim_end_matches('/').to_owned())
                    .unwrap_or_default();
                let relative = path.strip_prefix(&base).unwrap_or(&path);
                if crate::services::proxy_authorization::CanonicalPath::from_mcp_literal(relative)
                    .and_then(|p| {
                        crate::services::proxy_authorization::authorize_proxy_operation(
                            &r.service, method, &p,
                        )
                    })
                    .is_err()
                {
                    op_reason = Some(UnavailableReason::ScopeMissing);
                    break;
                }
            }
        }
        operations.push(OperationAvailability {
            operation,
            implemented,
            available: implemented && op_reason.is_none(),
            reason: op_reason,
        });
    }
    let available = operations
        .iter()
        .filter(|o| o.implemented)
        .all(|o| o.available);
    let reason = reason.or_else(|| {
        operations
            .iter()
            .find(|o| o.implemented && !o.available)
            .and_then(|o| o.reason.clone())
    });
    Availability {
        operations,
        adapter_id: profiles::id(b.profile).into(),
        contract_version: 1,
        binding_revision: b.revision,
        available,
        reason,
    }
}

pub async fn trusted_account(
    state: &AppState,
    catalog: &str,
    account: &str,
    p: Profile,
) -> AppResult<ProviderAccount> {
    state.db.collection::<ProviderAccount>(ACCOUNTS).find_one(doc!{"catalog_service_id":catalog,"provider":profiles::provider(p),"account_id":account}).await?.ok_or_else(unavailable)
}
pub fn validate_voice(b: &Binding) -> AppResult<()> {
    // Twilio's compiled profile deliberately pins Google en-US Standard-A.
    if b.profile == Profile::TwilioRelay && (b.voice != "en-US-Standard-A" || b.language != "en-US")
    {
        return Err(AppError::ValidationError(
            "Twilio profile requires Google voice en-US-Standard-A and language en-US".into(),
        ));
    }
    Ok(())
}
pub fn resource_paths(b: &Binding) -> Vec<String> {
    match profiles::provider(b.profile) {
        "twilio" => vec![
            format!(
                "/2010-04-01/Accounts/{}/IncomingPhoneNumbers/{}.json",
                b.account_id, b.resource_id
            ),
            format!(
                "/2010-04-01/Accounts/{}/Applications/{}.json",
                b.account_id, b.application_id
            ),
        ],
        "telnyx" => vec![
            format!("/v2/phone_numbers/{}", b.resource_id),
            format!(
                "/v2/{}/{}",
                if profiles::sms(b.profile) {
                    "messaging_profiles"
                } else {
                    "call_control_applications"
                },
                b.application_id
            ),
        ],
        _ => vec![format!("/v1/convai/agents/{}", b.resource_id)],
    }
}
/// Transport belongs to the owner's binding, separate from specialist tool grants.
pub async fn authorize(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> AppResult<()> {
    let url = url::Url::parse(&r.service.base_url).map_err(|_| unavailable())?;
    let base = url.path().trim_end_matches('/');
    let relative = if base.is_empty() {
        path
    } else {
        path.strip_prefix(base).ok_or_else(unavailable)?
    };
    // Adapter paths are server-built and encode validated provider identifiers.
    // Decode exactly once for policy matching, retaining the shared rejection
    // of encoded separators, traversal and additional decoding layers.
    let canonical = crate::services::proxy_authorization::CanonicalPath::from_mcp_built(relative)?;
    crate::services::proxy_authorization::authorize_proxy_operation(
        &r.service, method, &canonical,
    )?;
    let operation =
        crate::services::operation_descriptor::build_http_descriptor(method, relative, body);
    if crate::services::approval_service::evaluate_deny_only(
        &state.db,
        &b.user_id,
        &r.resource_owner,
        &r.service.id,
        &operation,
        false,
    )
    .await?
    {
        return Err(AppError::Forbidden(
            "Carrier operation denied by owner policy".into(),
        ));
    }
    // Provider transport is fixed and cannot implement catalog token exchanges,
    // delegated destinations or per-request identity forwarding.
    if r.service.token_exchange_config.is_some()
        || r.service.forward_access_token
        || r.service.inject_delegation_token
    {
        return Err(unavailable());
    }
    Ok(())
}

/// Informational readiness and tool callbacks share execution's read-only
/// connection resolver. This performs no refresh, decryption, or last-used write.
pub async fn read_authority(db: &mongodb::Database, b: &Binding) -> AppResult<Resolved> {
    require_enabled(db, &b.user_id).await?;
    super::owner_admission::check(db, &b.user_id).await?;
    let current = db
        .collection::<Binding>(BINDINGS)
        .find_one(doc! {"_id":&b.id,"enabled":true,"revision":b.revision})
        .await?
        .ok_or_else(unavailable)?;
    if current.verified_credential_identity != b.verified_credential_identity {
        return Err(unavailable());
    }
    for (kind, resource) in if b.profile == Profile::ElevenlabsCustomLlm {
        vec![("agent", b.resource_id.as_str())]
    } else {
        vec![
            ("number_id", b.resource_id.as_str()),
            ("phone", b.phone_number.as_str()),
            ("application", b.application_id.as_str()),
        ]
    } {
        if db.collection::<ResourceClaim>(CLAIMS).find_one(doc!{"identity_key":super::session::identity(&[profiles::provider(b.profile),kind,resource]),"user_id":&b.user_id}).await?.is_none(){return Err(AppError::Forbidden("Provider resource ownership unavailable".into()));}
    }
    let (service, class, owner, connection_id, identity, base, method) = match &b.credential {
        CredentialBinding::Connection { user_service_id } => {
            let target = crate::services::proxy_service::read_connection_authority(
                db,
                &b.user_id,
                user_service_id,
            )
            .await?
            .ok_or_else(unavailable)?;
            if target.master_credential || target.node_id.is_some() {
                return Err(unavailable());
            }
            let connection = db
                .collection::<UserService>(CONNECTIONS)
                .find_one(doc! {"_id":user_service_id})
                .await?
                .ok_or_else(unavailable)?;
            let service=db.collection::<DownstreamService>(SERVICES).find_one(doc!{"_id":connection.catalog_service_id.as_deref().ok_or_else(unavailable)?,"is_active":true}).await?.ok_or_else(unavailable)?;
            let identity = format!(
                "{}:{}",
                target.api_key_id.as_deref().unwrap_or_default(),
                target.credential_epoch
            );
            (
                service,
                crate::models::usage_meter::CredentialClass::UserOwned,
                connection.user_id,
                Some(user_service_id.clone()),
                identity,
                target.target.base_url,
                target.target.auth_method,
            )
        }
        CredentialBinding::Platform { catalog_service_id } => {
            let service = db
                .collection::<DownstreamService>(SERVICES)
                .find_one(doc! {"_id":catalog_service_id,"is_active":true})
                .await?
                .ok_or_else(unavailable)?;
            if service.platform_key.as_ref().is_none_or(|p| !p.enabled) {
                return Err(unavailable());
            }
            crate::services::proxy_service::authorize_master_credential(
                db,
                &service,
                &crate::services::proxy_service::EffectiveActor::from_user_id(&b.user_id),
            )
            .await?;
            let account=db.collection::<ProviderAccount>(ACCOUNTS).find_one(doc!{"catalog_service_id":catalog_service_id,"provider":profiles::provider(b.profile),"account_id":&b.account_id,"revision":b.trusted_account_revision}).await?.ok_or_else(unavailable)?;
            if account.webhook_public_key != b.webhook_public_key {
                return Err(unavailable());
            }
            db.collection::<ResourceAllocation>(ALLOCATIONS).find_one(doc!{"catalog_service_id":catalog_service_id,"user_id":&b.user_id,"provider":profiles::provider(b.profile),"account_id":&b.account_id,"resource_id":&b.resource_id,"phone_number":&b.phone_number,"application_id":&b.application_id}).await?.ok_or_else(unavailable)?;
            let identity = crate::services::audit_service::keyed_fingerprint(
                crate::services::audit_service::audit_chain_hmac_key(),
                b"channel-adapter-credential-v1",
                &service.credential_encrypted,
            )
            .ok_or_else(unavailable)?;
            let base = service.base_url.clone();
            let method = service.auth_method.clone();
            (
                service,
                crate::models::usage_meter::CredentialClass::NyxidManagedMaster,
                b.user_id.clone(),
                None,
                identity,
                base,
                method,
            )
        }
    };
    if identity != b.verified_credential_identity {
        return Err(AppError::Conflict(
            "Credential changed; verify setup again".into(),
        ));
    }
    let u = url::Url::parse(&base).map_err(|_| unavailable())?;
    let expected = match profiles::provider(b.profile) {
        "telnyx" => "api.telnyx.com",
        "twilio" => "api.twilio.com",
        _ => "api.elevenlabs.io",
    };
    if u.scheme() != "https"
        || u.host_str() != Some(expected)
        || u.port_or_known_default() != Some(443)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || !matches!(u.path(), "" | "/" | "/v2" | "/v2/" | "/v1" | "/v1/")
        || service.token_exchange_config.is_some()
        || service.forward_access_token
        || service.inject_delegation_token
        || service
            .billing
            .as_ref()
            .is_some_and(|p| p.managed_execution.is_some())
        || !matches!(
            (profiles::provider(b.profile), method.as_str()),
            ("twilio", "basic")
                | ("telnyx", "bearer")
                | ("elevenlabs", "header")
                | ("elevenlabs", "api_key")
        )
    {
        return Err(unavailable());
    }
    Ok(Resolved {
        key: Zeroizing::new(String::new()),
        identity,
        service,
        class,
        resource_owner: owner,
        connection_id,
    })
}

pub fn operation_paths(
    b: &Binding,
    op: crate::services::channel_capability::Operation,
) -> Vec<(&'static str, String)> {
    use crate::services::channel_capability::Operation as O;
    match (b.profile, op) {
        (Profile::TelnyxSms, O::SendMessage) => vec![("POST", "/v2/messages".into())],
        (Profile::TwilioSms, O::SendMessage) => vec![(
            "POST",
            format!("/2010-04-01/Accounts/{}/Messages.json", b.account_id),
        )],
        // Exact call IDs do not exist at setup. A policy requiring resource
        // matching cannot be proven by substituting an invented call ID.
        (Profile::TelnyxRelay, O::ReceiveCall) => vec![(
            "GET",
            format!("/v2/connections/{}/active_calls", b.application_id),
        )],
        _ => vec![],
    }
}

pub async fn verification_key(state: &AppState, b: &Binding) -> AppResult<String> {
    if let CredentialBinding::Platform { catalog_service_id } = &b.credential {
        let account = trusted_account(state, catalog_service_id, &b.account_id, b.profile).await?;
        if b.trusted_account_revision != Some(account.revision)
            || b.webhook_public_key != account.webhook_public_key
        {
            return Err(unavailable());
        }
        account.webhook_public_key.ok_or_else(unavailable)
    } else {
        b.webhook_public_key.clone().ok_or_else(unavailable)
    }
}

pub async fn claim_resources(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    b: &Binding,
) -> AppResult<()> {
    let resources = if b.profile == Profile::ElevenlabsCustomLlm {
        vec![("agent", b.resource_id.as_str())]
    } else {
        vec![
            ("number_id", b.resource_id.as_str()),
            ("phone", b.phone_number.as_str()),
            ("application", b.application_id.as_str()),
        ]
    };
    for (kind, resource) in resources {
        let identity_key =
            super::session::identity(&[profiles::provider(b.profile), kind, resource]);
        let row = db
            .collection::<ResourceClaim>(CLAIMS)
            .find_one(doc! {"identity_key":&identity_key})
            .session(&mut *tx)
            .await?;
        if let Some(row) = row {
            if row.user_id != b.user_id {
                return Err(AppError::Forbidden(
                    "Provider number or application belongs to another owner".into(),
                ));
            }
            let written = db
                .collection::<ResourceClaim>(CLAIMS)
                .update_one(
                    doc! {"_id":&row.id,"user_id":&b.user_id},
                    doc! {"$inc":{"revision":1}},
                )
                .session(&mut *tx)
                .await?;
            if written.matched_count != 1 {
                return Err(AppError::Conflict("Provider resource claim changed".into()));
            }
        } else {
            db.collection::<ResourceClaim>(CLAIMS)
                .insert_one(ResourceClaim {
                    id: uuid::Uuid::new_v4().to_string(),
                    identity_key,
                    revision: 1,
                    user_id: b.user_id.clone(),
                    provider: profiles::provider(b.profile).into(),
                    resource_kind: kind.into(),
                    resource_id: resource.into(),
                    created_at: chrono::Utc::now(),
                })
                .session(&mut *tx)
                .await?;
        }
    }
    Ok(())
}
pub(super) async fn persist(state: &AppState, b: &Binding) -> AppResult<()> {
    let mut tx = state.db.client().start_session().await?;
    let db = state.db.clone();
    let b = b.clone();
    tx.start_transaction().and_run2(async move |tx| {
        let result:AppResult<()>=async {
            super::owner_admission::fence(&db,tx,&b.user_id).await?;
            claim_resources(&db,tx,&b).await?;
            if let CredentialBinding::Platform{catalog_service_id}=&b.credential {
                let allocation=db.collection::<ResourceAllocation>(ALLOCATIONS).update_one(doc!{"catalog_service_id":catalog_service_id,"user_id":&b.user_id,"provider":profiles::provider(b.profile),"account_id":&b.account_id,"resource_id":&b.resource_id,"phone_number":&b.phone_number,"application_id":&b.application_id},doc!{"$inc":{"revision":1}}).session(&mut *tx).await?;
                if allocation.modified_count!=1{return Err(AppError::Forbidden("Resource allocation changed".into()));}
                let account=db.collection::<ProviderAccount>(ACCOUNTS).update_one(doc!{"catalog_service_id":catalog_service_id,"account_id":&b.account_id,"provider":profiles::provider(b.profile),"revision":b.trusted_account_revision},doc!{"$inc":{"allocation_fence":1}}).session(&mut *tx).await?;
                if account.matched_count!=1{return Err(AppError::Conflict("Provider account changed".into()));}
            }
            db.collection::<Binding>(BINDINGS).insert_one(&b).session(&mut *tx).await?;
            Ok(())
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub async fn require_enabled(db: &mongodb::Database, owner: &str) -> AppResult<()> {
    if crate::services::feature_flag_service::personal_flag_enabled(
        db,
        owner,
        crate::services::feature_flag_service::NYXBOT_ADAPTERS_FLAG_KEY,
    )
    .await?
    {
        Ok(())
    } else {
        Err(AppError::NotFound(
            "Phone and speech adapters are disabled".into(),
        ))
    }
}
