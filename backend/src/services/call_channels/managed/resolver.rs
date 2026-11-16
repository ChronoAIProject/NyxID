//! Server-authored line authority is the only managed credential consumer.
use super::super::{binding::Resolved, profiles, protocol};
use super::{admission, unavailable};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_adapter::*,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        managed_comms::*,
        usage_meter::CredentialClass,
    },
};
use mongodb::{Database, bson::doc};
use zeroize::Zeroizing;

pub fn fixed_service(service: &DownstreamService, provider: &str) -> AppResult<()> {
    let u = url::Url::parse(&service.base_url).map_err(|_| unavailable())?;
    let host = match provider {
        "telnyx" => "api.telnyx.com",
        "twilio" => "api.twilio.com",
        _ => return Err(unavailable()),
    };
    if u.scheme() != "https"
        || u.host_str() != Some(host)
        || u.port_or_known_default() != Some(443)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || !matches!(u.path(), "" | "/" | "/v2" | "/v2/")
        || !service.destination_targets.is_empty()
        || service.provider_config_id.is_some()
        || service.token_exchange_config.is_some()
        || service.forward_access_token
        || service.inject_delegation_token
        || !matches!(service.identity_propagation_mode.as_str(), "none")
        || !matches!(
            (provider, service.auth_method.as_str()),
            ("telnyx", "bearer") | ("twilio", "basic")
        )
        || service.managed_comms_role != Some(ManagedCommsRole::Credential)
        || service.platform_key.as_ref().is_some_and(|p| p.enabled)
    {
        return Err(unavailable());
    }
    Ok(())
}
async fn service(db: &Database, account: &ProviderAccount) -> AppResult<DownstreamService> {
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&account.catalog_service_id})
        .await?
        .ok_or_else(unavailable)?;
    fixed_service(&service, &account.provider)?;
    Ok(service)
}
pub async fn resolve(
    state: &AppState,
    owner: &str,
    credential: &CredentialBinding,
    profile: Profile,
) -> AppResult<Resolved> {
    let a = admission::read(&state.db, owner, credential).await?;
    let mut r = read_from_admission(&state.db, owner, profile, &a).await?;
    r.key = crate::services::catalog_credential_record_service::text(
        &state.encryption_keys,
        &r.service,
    )
    .await?;
    if r.key.is_empty() {
        return Err(unavailable());
    }
    Ok(r)
}
async fn read_from_admission(
    db: &Database,
    owner: &str,
    profile: Profile,
    a: &admission::Admission,
) -> AppResult<Resolved> {
    if profiles::provider(profile) != a.line.provider || profile == Profile::ElevenlabsCustomLlm {
        return Err(unavailable());
    }
    let capability = if profiles::sms(profile) {
        a.line.sms.as_ref()
    } else {
        a.line.voice.as_ref()
    }
    .ok_or_else(unavailable)?;
    if profiles::sms(profile)
        && (!a.claim.sms_enabled
            || a.eligibility.sms_registration_status != "approved"
            || a.line
                .regulatory
                .sms_sender_registration
                .as_ref()
                .is_none_or(|r| r.is_empty()))
    {
        return Err(unavailable());
    }
    let price = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&capability.billing_service_id,"is_active":true})
        .await?
        .ok_or_else(unavailable)?;
    if price.managed_comms_role
        != Some(if profiles::sms(profile) {
            ManagedCommsRole::Sms
        } else {
            ManagedCommsRole::Voice
        })
        || !price.credential_encrypted.is_empty()
    {
        return Err(unavailable());
    }
    let s = service(db, &a.account).await?;
    if !s.is_active || s.proxy_operation_policy.is_none() {
        return Err(unavailable());
    }
    Ok(Resolved {
        key: Zeroizing::new(String::new()),
        identity: super::super::session::identity(&["managed-canonical", &a.account.id, &s.id]),
        service: s,
        class: CredentialClass::NyxidManagedMaster,
        resource_owner: owner.into(),
        connection_id: None,
    })
}
pub async fn read_binding(db: &Database, b: &Binding) -> AppResult<Resolved> {
    let a = admission::read(db, &b.user_id, &b.credential).await?;
    let cap = if profiles::sms(b.profile) {
        a.line.sms.as_ref()
    } else {
        a.line.voice.as_ref()
    }
    .ok_or_else(unavailable)?;
    let expected_binding = if profiles::sms(b.profile) {
        a.claim.sms_binding_id.as_deref()
    } else {
        a.claim.voice_binding_id.as_deref()
    };
    if expected_binding != Some(&b.id)
        || b.agent_id != a.claim.agent_id
        || b.account_id != a.account.account_id
        || b.phone_number != a.line.phone_number
        || b.resource_id != a.line.number_resource_id
        || b.application_id != cap.application_id
        || b.webhook_public_key != a.account.webhook_public_key
        || b.trusted_account_revision != Some(a.account.revision)
    {
        return Err(unavailable());
    }
    if !profiles::sms(b.profile) {
        let selected = a
            .claim
            .standard_voice
            .clone()
            .unwrap_or_else(|| super::voice::default_option(&a.line.provider));
        if b.voice != selected.voice
            || b.language != selected.language
            || (a.claim.voice_tier == VoiceTier::Standard
                && !super::voice::options(&a.line).contains(&selected))
        {
            return Err(unavailable());
        }
    }
    if a.claim.voice_tier == VoiceTier::Premium {
        let c = super::synthesis::config(db, &a.account.id)
            .await?
            .ok_or_else(unavailable)?;
        let p = c.premium.as_ref().ok_or_else(unavailable)?;
        if a.line.provider != "telnyx"
            || !a.eligibility.premium_allowed
            || a.claim.accepted_terms.elevenlabs_eua.as_deref() != Some(&p.oem_terms_version)
            || !a
                .claim
                .standard_voice
                .as_ref()
                .is_some_and(|v| p.voices.contains(v))
        {
            return Err(unavailable());
        }
    }
    super::custody::verify(db, &a.line).await?;
    read_from_admission(db, &b.user_id, b.profile, &a).await
}

/// Authority retained by one persisted effect, independent of new admissions.
/// This deliberately exposes no general-purpose execution resolver.
pub struct ExistingAuthentication {
    pub key: Zeroizing<String>,
    pub webhook_public_key: Option<String>,
}
impl std::fmt::Debug for ExistingAuthentication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExistingAuthentication { [REDACTED] }")
    }
}
/// The exact trusted account an effect was admitted on.
async fn reference_account(db: &Database, r: &AdmittedReference) -> AppResult<ProviderAccount> {
    db.collection::<ProviderAccount>(ACCOUNTS).find_one(doc!{"_id":&r.trusted_account_id,"provider":&r.provider,"account_id":&r.account_id,"catalog_service_id":&r.credential_service_id}).await?.ok_or_else(unavailable)
}
pub(super) async fn existing_authentication(
    state: &AppState,
    r: &AdmittedReference,
) -> AppResult<ExistingAuthentication> {
    let account = reference_account(&state.db, r).await?;
    let service = service(&state.db, &account).await?;
    let key =
        crate::services::catalog_credential_record_service::text(&state.encryption_keys, &service)
            .await?;
    if key.is_empty() {
        return Err(unavailable());
    }
    Ok(ExistingAuthentication {
        key,
        webhook_public_key: account.webhook_public_key,
    })
}
fn matches_binding(b: &Binding, r: &AdmittedReference) -> AppResult<()> {
    let CredentialBinding::ManagedLine { line_id, .. } = &b.credential else {
        return Err(unavailable());
    };
    if line_id != &r.line_id
        || profiles::provider(b.profile) != r.provider
        || b.account_id != r.account_id
        || b.resource_id != r.number_resource_id
        || b.application_id != r.application_id
    {
        return Err(unavailable());
    }
    Ok(())
}
/// Load the admitted reference from durable state; callback input is only a
/// lookup hint until signature verification completes. It cannot select a key.
pub async fn callback_authentication(
    state: &AppState,
    b: &Binding,
    event: &protocol::Ingress,
    send: Option<&str>,
) -> AppResult<Option<ExistingAuthentication>> {
    let reference = match event {
        protocol::Ingress::CallEnded { call_id, .. }
        | protocol::Ingress::RelayEnded { call_id, .. }
        | protocol::Ingress::CallAnswered { call_id, .. } => {
            let session = state
                .db
                .collection::<CallSession>(SESSIONS)
                .find_one(doc! {"binding_id":&b.id,"user_id":&b.user_id,"provider_call_id":call_id})
                .await?
                .ok_or_else(unavailable)?;
            session_reference(state, b, &session).await?
        }
        protocol::Ingress::Receipt { message_id, .. } => {
            let mut filter = doc! {"binding_id":&b.id,"user_id":&b.user_id,"event_kind":"sms_send"};
            if let Some(send) = send {
                filter.insert("_id", send);
            } else {
                filter.insert("provider_message_id", message_id);
            }
            let row = state
                .db
                .collection::<EventReceipt>(EVENTS)
                .find_one(filter)
                .await?
                .ok_or_else(unavailable)?;
            if row
                .provider_message_id
                .as_deref()
                .is_some_and(|id| id != message_id)
            {
                return Err(unavailable());
            }
            let mut r = row.admitted_reference.ok_or_else(unavailable)?;
            if r.provider_message_id
                .as_deref()
                .is_some_and(|id| id != message_id)
                || r.provider_call_id.is_some()
            {
                return Err(unavailable());
            }
            r.provider_message_id = Some(message_id.clone());
            matches_binding(b, &r)?;
            r
        }
        _ => return Ok(None),
    };
    Ok(Some(existing_authentication(state, &reference).await?))
}
pub async fn session_reference(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
) -> AppResult<AdmittedReference> {
    let row=state.db.collection::<CallSession>(SESSIONS).find_one(doc!{"_id":&s.id,"binding_id":&b.id,"user_id":&b.user_id,"provider_call_id":&s.provider_call_id}).await?.ok_or_else(unavailable)?;
    let reference = row.admitted_reference.ok_or_else(unavailable)?;
    matches_binding(b, &reference)?;
    if reference.provider_call_id.as_deref() != Some(s.provider_call_id.as_str())
        || reference.provider_message_id.is_some()
    {
        return Err(unavailable());
    }
    Ok(reference)
}
pub async fn session_authentication(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
) -> AppResult<ExistingAuthentication> {
    existing_authentication(state, &session_reference(state, b, s).await?).await
}
/// Only typed exact-effect cleanup operations can reach this transport. No
/// owner policy, flag or mutable claim may block evidence or termination.
pub async fn request_existing(
    state: &AppState,
    b: &Binding,
    r: &AdmittedReference,
    op: CleanupOperation,
    fixture_origin: Option<&str>,
) -> AppResult<serde_json::Value> {
    matches_binding(b, r)?;
    request_reference(state, r, op, fixture_origin).await
}
/// Telnyx exact-message evidence read for a retained send. Unlike termination,
/// a refused read only leaves the part to its evidence window (never a charge),
/// so the canonical credential service's operation policy must allow it.
/// Twilio's message read stays on [`request_existing`]: the policy grammar has
/// no whole-segment form for `{MessageSid}.json`.
pub async fn read_telnyx_message(
    state: &AppState,
    b: &Binding,
    r: &AdmittedReference,
    fixture_origin: Option<&str>,
) -> AppResult<serde_json::Value> {
    matches_binding(b, r)?;
    if r.provider != "telnyx" {
        return Err(unavailable());
    }
    let service = service(&state.db, &reference_account(&state.db, r).await?).await?;
    let target = cleanup_request(r, CleanupOperation::ReadMessage)?;
    let base = url::Url::parse(&service.base_url).map_err(|_| unavailable())?;
    let relative = target
        .url
        .path()
        .strip_prefix(base.path().trim_end_matches('/'))
        .ok_or_else(unavailable)?;
    let canonical = crate::services::proxy_authorization::CanonicalPath::from_mcp_built(relative)?;
    crate::services::proxy_authorization::authorize_proxy_operation(&service, "GET", &canonical)?;
    request_reference(state, r, CleanupOperation::ReadMessage, fixture_origin).await
}
pub(super) async fn request_reference(
    state: &AppState,
    r: &AdmittedReference,
    op: CleanupOperation,
    fixture_origin: Option<&str>,
) -> AppResult<serde_json::Value> {
    let mut target = match cleanup_reference(state, r, op).await {
        Ok(target) => target,
        Err(error) => {
            super::super::management::audit(
                state,
                "system",
                "managed_cleanup_credential_failed",
                serde_json::json!({"line_id":r.line_id,"operation":format!("{op:?}")}),
            );
            return Err(error);
        }
    };
    if let Some(origin) = fixture_origin {
        let path = format!(
            "{}{}",
            target.url.path(),
            target
                .url
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        );
        target.url = profiles::provider_url(
            if r.provider == "twilio" {
                crate::models::channel_adapter::Profile::TwilioRelay
            } else {
                crate::models::channel_adapter::Profile::TelnyxRelay
            },
            origin,
            &path,
        )?;
    }
    let method =
        reqwest::Method::from_bytes(target.method.as_bytes()).map_err(|_| unavailable())?;
    let mut request = crate::services::proxy_service::no_redirect_http_client()
        .request(method, target.url)
        .timeout(std::time::Duration::from_secs(15));
    request = if r.provider == "twilio" {
        let (a, k) = protocol::twilio_key(&target.key, &r.account_id)?;
        request.basic_auth(a.to_string(), Some(k))
    } else {
        request.bearer_auth(target.key.as_str())
    };
    request = match target.body {
        Some(CleanupBody::Form(fields)) => request.form(&fields),
        Some(CleanupBody::Json(value)) => request.json(&value),
        None => request,
    };
    let response = request.send().await.map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let bytes = profiles::bounded_response(response).await?;
    serde_json::from_slice(&bytes).map_err(|_| unavailable())
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M4 cleanup dispatch uses Hangup and ReadDetailRecord; rotation already uses the reads"
    )
)]
pub enum CleanupOperation {
    Hangup,
    ReadCall,
    ReadDetailRecord,
    ReadMessage,
    ReadCostRecord(CostRecordType),
}
#[derive(Clone, Copy, Debug)]
pub enum CostRecordType {
    CallControl,
    Relay,
    Stt,
    Tts,
    Messaging,
}
impl CostRecordType {
    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "call-control" => Ok(Self::CallControl),
            "conversation-relay" => Ok(Self::Relay),
            "speech-to-text" => Ok(Self::Stt),
            "text-to-speech" => Ok(Self::Tts),
            "messaging" => Ok(Self::Messaging),
            _ => Err(unavailable()),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::CallControl => "call-control",
            Self::Relay => "conversation-relay",
            Self::Stt => "speech-to-text",
            Self::Tts => "text-to-speech",
            Self::Messaging => "messaging",
        }
    }
}
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M4 dispatch consumes the credential, method and body; rotation uses the URL"
    )
)]
pub struct CleanupTarget {
    pub key: Zeroizing<String>,
    pub method: &'static str,
    pub url: url::Url,
    pub body: Option<CleanupBody>,
}
pub enum CleanupBody {
    Form(Vec<(&'static str, &'static str)>),
    Json(serde_json::Value),
}
impl std::fmt::Debug for CleanupTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CleanupTarget { [REDACTED] }")
    }
}
/// The effect route is a retained server-authored record, never caller authority.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "M4 recovery consumes this M1 cleanup authority")
)]
pub async fn cleanup(
    state: &AppState,
    route_id: &str,
    op: CleanupOperation,
) -> AppResult<CleanupTarget> {
    let route = state
        .db
        .collection::<ManagedEffectRoute>(ManagedEffectRoute::COLLECTION_NAME)
        .find_one(doc! {"_id":route_id})
        .await?
        .ok_or_else(unavailable)?;
    let result = cleanup_reference(state, &route.admitted, op).await;
    if result.is_err() {
        super::super::management::audit(
            state,
            "system",
            "managed_cleanup_credential_failed",
            serde_json::json!({"effect_route_id":route.id,"operation":format!("{op:?}")}),
        );
    }
    result
}
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "M4 recovery consumes this M1 cleanup authority")
)]
async fn cleanup_reference(
    state: &AppState,
    r: &AdmittedReference,
    op: CleanupOperation,
) -> AppResult<CleanupTarget> {
    let authentication = existing_authentication(state, r).await?;
    let mut target = cleanup_request(r, op)?;
    target.key = authentication.key;
    Ok(target)
}
pub(super) fn cleanup_request(
    r: &AdmittedReference,
    op: CleanupOperation,
) -> AppResult<CleanupTarget> {
    let call = || {
        r.provider_call_id
            .as_deref()
            .filter(|id| {
                protocol::call_handle(
                    if r.provider == "telnyx" {
                        Profile::TelnyxRelay
                    } else {
                        Profile::TwilioRelay
                    },
                    id,
                )
            })
            .ok_or_else(unavailable)
    };
    let message = || {
        r.provider_message_id
            .as_deref()
            .filter(|id| protocol::handle(id))
            .ok_or_else(unavailable)
    };
    if r.provider == "twilio" {
        protocol::TwilioAccountSid::parse(&r.account_id)?;
    }
    if !protocol::handle(&r.account_id) {
        return Err(unavailable());
    }
    let (method, path, body) = match (r.provider.as_str(), op) {
        ("telnyx", CleanupOperation::Hangup) => (
            "POST",
            format!(
                "/v2/calls/{}/actions/hangup",
                protocol::call_segment(Profile::TelnyxRelay, call()?)?
            ),
            Some(CleanupBody::Json(
                serde_json::json!({"command_id":uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL,format!("managed-hangup:{}:{}",r.trusted_account_id,call()?).as_bytes()).to_string()}),
            )),
        ),
        ("telnyx", CleanupOperation::ReadCall) => (
            "GET",
            format!(
                "/v2/calls/{}",
                protocol::call_segment(Profile::TelnyxRelay, call()?)?
            ),
            None,
        ),
        ("telnyx", CleanupOperation::ReadDetailRecord) => {
            let leg = r
                .call_leg_id
                .as_deref()
                .filter(|id| protocol::handle(id))
                .ok_or_else(unavailable)?;
            let app = &r.application_id;
            if !protocol::handle(app) {
                return Err(unavailable());
            }
            (
                "GET",
                format!(
                    "/v2/detail_records?filter[record_type]=call-control&filter[call_leg_id]={leg}&filter[connection_id]={app}"
                ),
                None,
            )
        }
        ("telnyx", CleanupOperation::ReadCostRecord(record_type)) => {
            let path = if matches!(record_type, CostRecordType::Messaging) {
                if !protocol::handle(&r.application_id) {
                    return Err(unavailable());
                }
                format!(
                    "/v2/detail_records?filter[record_type]=messaging&filter[message_id]={}&filter[messaging_profile_id]={}",
                    message()?,
                    r.application_id
                )
            } else {
                let leg = r
                    .call_leg_id
                    .as_deref()
                    .filter(|id| protocol::handle(id))
                    .ok_or_else(unavailable)?;
                if !protocol::handle(&r.application_id) {
                    return Err(unavailable());
                }
                format!(
                    "/v2/detail_records?filter[record_type]={}&filter[call_leg_id]={leg}&filter[connection_id]={}",
                    record_type.as_str(),
                    r.application_id
                )
            };
            ("GET", path, None)
        }
        ("telnyx", CleanupOperation::ReadMessage) => {
            ("GET", format!("/v2/messages/{}", message()?), None)
        }
        ("twilio", CleanupOperation::Hangup) => (
            "POST",
            format!(
                "/2010-04-01/Accounts/{}/Calls/{}.json",
                r.account_id,
                call()?
            ),
            Some(CleanupBody::Form(vec![("Status", "completed")])),
        ),
        ("twilio", CleanupOperation::ReadCall) => (
            "GET",
            format!(
                "/2010-04-01/Accounts/{}/Calls/{}.json",
                r.account_id,
                call()?
            ),
            None,
        ),
        ("twilio", CleanupOperation::ReadMessage) => (
            "GET",
            format!(
                "/2010-04-01/Accounts/{}/Messages/{}.json",
                r.account_id,
                message()?
            ),
            None,
        ),
        _ => {
            return Err(AppError::BadRequest(
                "Unsupported managed cleanup operation".into(),
            ));
        }
    };
    let origin = if r.provider == "telnyx" {
        "https://api.telnyx.com"
    } else {
        "https://api.twilio.com"
    };
    let url = url::Url::parse(&format!("{origin}{path}")).map_err(|_| unavailable())?;
    Ok(CleanupTarget {
        key: Zeroizing::new(String::new()),
        method,
        url,
        body,
    })
}
/// A successor must prove read access to each exact unfinished provider effect.
/// Twilio uses the account Auth Token; Telnyx uses the same canonical account's
/// API credential and re-proves all of its retained resources before this read.
pub(super) async fn prove_cleanup_credential(
    r: &AdmittedReference,
    key: &str,
    fixture: Option<&str>,
) -> AppResult<()> {
    let op = if r.provider_call_id.is_some() {
        CleanupOperation::ReadCall
    } else if r.provider_message_id.is_some() {
        CleanupOperation::ReadMessage
    } else {
        return Err(unavailable());
    };
    let mut target = cleanup_request(r, op)?;
    if let Some(fixture) = fixture {
        let profile = if r.provider == "twilio" {
            Profile::TwilioRelay
        } else {
            Profile::TelnyxRelay
        };
        let origin = profiles::origin(profile, Some(fixture))?;
        target.url = profiles::provider_url(profile, &origin, target.url.path())?;
    }
    let mut request = crate::services::proxy_service::no_redirect_http_client()
        .get(target.url)
        .timeout(std::time::Duration::from_secs(15));
    request = if r.provider == "twilio" {
        let (a, k) = protocol::twilio_key(key, &r.account_id)?;
        request.basic_auth(a.to_string(), Some(k))
    } else {
        request.bearer_auth(key)
    };
    let response = request.send().await.map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let bytes = profiles::bounded_response(response).await?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if r.provider == "twilio" {
        if v["account_sid"] != r.account_id
            || v["sid"].as_str()
                != r.provider_call_id
                    .as_deref()
                    .or(r.provider_message_id.as_deref())
        {
            return Err(unavailable());
        }
    } else if v["data"]["call_control_id"]
        .as_str()
        .or(v["data"]["id"].as_str())
        != r.provider_call_id
            .as_deref()
            .or(r.provider_message_id.as_deref())
    {
        return Err(unavailable());
    }
    Ok(())
}
