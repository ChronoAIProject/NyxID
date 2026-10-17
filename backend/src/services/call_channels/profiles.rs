//! Implemented profiles participate in the single channel adapter registry.
use super::protocol;
use crate::services::channel_capability::*;
use crate::{
    errors::{AppError, AppResult},
    models::channel_adapter::Profile,
};
use serde_json::{Value, json};

pub fn id(p: Profile) -> &'static str {
    match p {
        Profile::TelnyxRelay => "telnyx.conversation_relay",
        Profile::TelnyxSms => "telnyx.messaging",
        Profile::TwilioRelay => "twilio.conversation_relay",
        Profile::TwilioSms => "twilio.messaging",
        Profile::ElevenlabsCustomLlm => "elevenlabs.custom_llm",
    }
}
pub fn provider(p: Profile) -> &'static str {
    match p {
        Profile::TelnyxRelay | Profile::TelnyxSms => "telnyx",
        Profile::TwilioRelay | Profile::TwilioSms => "twilio",
        Profile::ElevenlabsCustomLlm => "elevenlabs",
    }
}
pub fn sms(p: Profile) -> bool {
    matches!(p, Profile::TelnyxSms | Profile::TwilioSms)
}
pub fn relay(p: Profile) -> bool {
    matches!(p, Profile::TelnyxRelay | Profile::TwilioRelay)
}
pub fn descriptor(p: Profile) -> Descriptor {
    let messaging = sms(p);
    let realtime = relay(p);
    let custom = p == Profile::ElevenlabsCustomLlm;
    Descriptor {
        adapter_id: id(p).into(),
        contract_version: CONTRACT_VERSION,
        operations: if messaging {
            vec![Operation::ReceiveMessage, Operation::SendMessage]
        } else if realtime {
            vec![
                Operation::ReceiveCall,
                Operation::EndSession,
                Operation::StreamReply,
                Operation::Interrupt,
            ]
        } else {
            vec![Operation::StreamReply, Operation::EndSession]
        },
        messaging: Messaging {
            reply_to: false,
            initiated_send: false,
            inbound_media: &[],
            outbound_media: &[],
            max_attachment_bytes: None,
            threads: Default::default(),
            inbound_text: messaging,
            outbound_text: messaging,
            edit: false,
            max_text_bytes: messaging.then_some(1600),
            delivery: if messaging {
                Evidence::ProviderReceipt
            } else {
                Evidence::Unsupported
            },
        },
        calls: Calls {
            inbound: realtime,
            outbound: false,
            transfer: false,
            dtmf_input: false,
            end_session: !messaging,
        },
        conversation: Conversation {
            final_transcripts: !messaging,
            cumulative_interim_transcripts: false,
            streamed_output: !messaging,
            interruption: realtime,
            playback_evidence: if realtime {
                Evidence::InterruptedPrefix
            } else {
                Evidence::Unsupported
            },
            explicit_cancel: false,
        },
        instructions: Instructions {
            external_nyxbot: true,
            nyxbot_refresh: InstructionTiming::BetweenTurns,
            provider_prompt_replacement: InstructionTiming::Unsupported,
        },
        lifecycle: Lifecycle {
            reconnect: false,
            automatic_effect_replay: false,
            durable_ingress_dedup: true,
            max_session_seconds: (!messaging).then_some(1800),
            max_frame_bytes: realtime.then_some(16384),
            output_queue_capacity: Some(64),
        },
        usage: Usage {
            units: if messaging {
                &["requests"]
            } else if custom {
                &[]
            } else {
                &["voice_seconds"]
            },
            evidence: if messaging {
                "provider_acceptance_receipt_or_authenticated_message_parts"
            } else if custom {
                "nyxagent_model_billed_independently"
            } else {
                "provider_terminal_duration_or_persisted_terminal_observation"
            },
            configured_billing: !custom,
        },
        setup: Setup {
            credential_modes: vec![
                CredentialModeSupport {
                    kind: "connection",
                    implemented: true,
                    prerequisite: None,
                },
                CredentialModeSupport {
                    kind: "platform",
                    implemented: !matches!(p, Profile::TelnyxRelay | Profile::ElevenlabsCustomLlm),
                    prerequisite: if p == Profile::TelnyxRelay {
                        Some("provider_limit_unavailable")
                    } else if custom {
                        Some("external_speech_billing_required")
                    } else {
                        Some("allocated_resource_and_configured_billing")
                    },
                },
            ],
            credential_alternatives: &["connection", "platform"],
            required_resources: if custom {
                &["agent", "server_issued_session"]
            } else {
                &["account", "number", "application", "scoped_specialist"]
            },
            carrier: if custom { "external" } else { provider(p) },
            speech: if custom {
                "elevenlabs"
            } else {
                "carrier_conversation_relay"
            },
            agent: "nyxbot_specialist",
            documentation: Some(match p {
                Profile::TelnyxRelay => {
                    "https://developers.telnyx.com/docs/voice/programmable-voice/conversation-relay"
                }
                Profile::TelnyxSms => {
                    "https://developers.telnyx.com/docs/messaging/messages/receiving-webhooks"
                }
                Profile::TwilioRelay => {
                    "https://www.twilio.com/docs/voice/conversationrelay/websocket-messages"
                }
                Profile::TwilioSms => {
                    "https://www.twilio.com/docs/messaging/guides/webhook-request"
                }
                Profile::ElevenlabsCustomLlm => {
                    "https://elevenlabs.io/docs/eleven-agents/customization/llm/custom-llm"
                }
            }),
        },
    }
}

/// Typed executable interfaces; chat-bot registration is a separate projection.
pub enum Adapter {
    Messaging(SmsAdapter),
    Conversation(RelayAdapter),
    CustomLlm(CustomLlmAdapter),
}
pub struct SmsAdapter(pub Profile);
pub struct RelayAdapter(pub Profile);
pub struct CustomLlmAdapter;
impl Adapter {
    pub fn new(p: Profile) -> Self {
        if sms(p) {
            Self::Messaging(SmsAdapter(p))
        } else if relay(p) {
            Self::Conversation(RelayAdapter(p))
        } else {
            Self::CustomLlm(CustomLlmAdapter)
        }
    }
    pub fn profile(&self) -> Profile {
        match self {
            Self::Messaging(a) => a.0,
            Self::Conversation(a) => a.0,
            Self::CustomLlm(_) => Profile::ElevenlabsCustomLlm,
        }
    }
    pub fn descriptor(&self) -> Descriptor {
        descriptor(self.profile())
    }
}
impl SmsAdapter {
    pub fn receive(
        &self,
        b: &crate::models::channel_adapter::Binding,
        body: &[u8],
    ) -> AppResult<protocol::Ingress> {
        if self.0 == Profile::TelnyxSms {
            protocol::telnyx(b, body)
        } else {
            protocol::twilio(b, &protocol::form(body)?)
        }
    }
    pub async fn send(
        &self,
        http: &reqwest::Client,
        b: &crate::models::channel_adapter::Binding,
        key: &str,
        to: &str,
        text: &str,
        callback: &str,
    ) -> Result<SmsAccepted, SmsFailure> {
        if b.profile != self.0 {
            return Err(SmsFailure::Rejected(
                AppError::ChannelPlatformSendUnsupported,
            ));
        }
        send_sms(http, b, key, to, text, callback).await
    }
}
impl RelayAdapter {
    pub fn receive(
        &self,
        b: &crate::models::channel_adapter::Binding,
        body: &[u8],
    ) -> AppResult<protocol::Ingress> {
        if self.0 == Profile::TelnyxRelay {
            protocol::telnyx(b, body)
        } else {
            protocol::twilio(b, &protocol::form(body)?)
        }
    }
    pub fn frame(&self, bytes: &[u8]) -> AppResult<protocol::RelayFrame> {
        protocol::relay_frame(bytes)
    }
    pub fn text(&self, text: &str, last: bool) -> Value {
        protocol::text_frame(text, last)
    }
    pub async fn command(
        &self,
        state: &crate::AppState,
        b: &crate::models::channel_adapter::Binding,
        r: &super::binding::Resolved,
        s: &crate::models::channel_adapter::CallSession,
        operation: &str,
        payload: Value,
    ) -> AppResult<()> {
        super::carrier::telnyx_command(state, b, r, s, operation, payload).await
    }
    pub fn end(&self) -> Value {
        protocol::end_frame()
    }
}

pub struct SmsAccepted {
    pub id: String,
    pub parts: Option<i64>,
}
impl std::fmt::Debug for SmsAccepted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsAccepted")
            .field("parts", &self.parts)
            .finish_non_exhaustive()
    }
}

/// A provider error response can prove refusal; a transport error cannot.
#[derive(Debug)]
pub enum SmsFailure {
    Rejected(AppError),
    Uncertain,
}

/// Conservative UCS-2 reservation also covers US/Canada toll-free routing.
/// Twilio documents 66 units per concatenated toll-free segment and 700 total.
/// The UTF-8 bound is independently enforced for the published text contract.
pub fn sms_parts(profile: Profile, text: &str) -> AppResult<i64> {
    let units = text.encode_utf16().count();
    let (max_units, multipart_units) = match profile {
        Profile::TwilioSms => (700, 66),
        Profile::TelnyxSms => (700, 66),
        _ => return Err(AppError::ChannelPlatformSendUnsupported),
    };
    if text.is_empty() || text.len() > 1600 || units > max_units {
        return Err(AppError::ValidationError(
            "Answer exceeds this SMS profile's 1600-byte / 700 UTF-16-unit limit".into(),
        ));
    }
    Ok(if units <= 70 {
        1
    } else {
        units.div_ceil(multipart_units) as i64
    })
}

pub fn sms_payload(
    b: &crate::models::channel_adapter::Binding,
    to: &str,
    text: &str,
    callback: &str,
) -> AppResult<Vec<u8>> {
    sms_parts(b.profile, text)?;
    if !protocol::phone(to) {
        return Err(AppError::ValidationError("Invalid SMS destination".into()));
    }
    if b.profile == Profile::TelnyxSms {
        serde_json::to_vec(&json!({"from":b.phone_number,"to":to,"text":text,"messaging_profile_id":b.application_id,"webhook_url":callback}))
            .map_err(|_|AppError::Internal("SMS request encoding failed".into()))
    } else {
        Ok(url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("From", b.phone_number.as_str()),
                ("To", to),
                ("Body", text),
                ("StatusCallback", callback),
            ])
            .finish()
            .into_bytes())
    }
}

pub async fn send_sms(
    http: &reqwest::Client,
    binding: &crate::models::channel_adapter::Binding,
    key: &str,
    to: &str,
    text: &str,
    callback: &str,
) -> Result<SmsAccepted, SmsFailure> {
    send_sms_at(http, binding, key, to, text, callback, None).await
}
pub(crate) async fn send_sms_at(
    http: &reqwest::Client,
    b: &crate::models::channel_adapter::Binding,
    key: &str,
    to: &str,
    text: &str,
    callback: &str,
    test_origin: Option<&str>,
) -> Result<SmsAccepted, SmsFailure> {
    let payload = sms_payload(b, to, text, callback).map_err(SmsFailure::Rejected)?;
    let origin = if test_origin.is_some() {
        origin(b.profile, test_origin)
    } else {
        binding_origin(b)
    }
    .map_err(SmsFailure::Rejected)?;
    let request = if b.profile == Profile::TelnyxSms {
        http.post(provider_url(b.profile, &origin, "/v2/messages").map_err(SmsFailure::Rejected)?)
            .bearer_auth(key)
            .header("content-type", "application/json")
            .body(payload)
    } else {
        let (sid, secret) =
            protocol::twilio_key(key, &b.account_id).map_err(SmsFailure::Rejected)?;
        let path = format!("/2010-04-01/Accounts/{sid}/Messages.json");
        http.post(provider_url(b.profile, &origin, &path).map_err(SmsFailure::Rejected)?)
            .basic_auth(sid.to_string(), Some(secret))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(payload)
    };
    let response = request
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| SmsFailure::Uncertain)?;
    let status = response.status();
    let bytes = bounded_response(response)
        .await
        .map_err(|_| SmsFailure::Uncertain)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| SmsFailure::Uncertain)?;
    if !status.is_success() {
        // Only a complete documented rejection envelope proves no acceptance.
        let rejection = status.is_client_error()
            && status.as_u16() != 408
            && if b.profile == Profile::TwilioSms {
                value.get("code").is_some_and(Value::is_number) && value.get("sid").is_none()
            } else {
                value.get("errors").is_some_and(Value::is_array)
                    && value.pointer("/data/id").is_none()
            };
        return Err(if rejection {
            SmsFailure::Rejected(AppError::ChannelPlatformError(
                "SMS provider rejected the request".into(),
            ))
        } else {
            SmsFailure::Uncertain
        });
    }
    let (id, parts) = if b.profile == Profile::TelnyxSms {
        (
            value.pointer("/data/id").and_then(Value::as_str),
            value.pointer("/data/parts").and_then(Value::as_i64),
        )
    } else {
        (
            value.get("sid").and_then(Value::as_str),
            value
                .get("num_segments")
                .and_then(Value::as_str)
                .and_then(|n| n.parse().ok()),
        )
    };
    Ok(SmsAccepted {
        id: id
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or(SmsFailure::Uncertain)?
            .into(),
        parts: parts.filter(|n| (1..=16).contains(n)),
    })
}

pub(crate) fn origin(p: Profile, fixture: Option<&str>) -> AppResult<String> {
    #[cfg(test)]
    if let Some(origin) = fixture {
        let url = url::Url::parse(origin)
            .map_err(|_| AppError::Forbidden("Unsupported provider origin".into()))?;
        if url.scheme() != "http"
            || !matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
                && !matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::Forbidden("Unsupported provider origin".into()));
        }
        return Ok(origin.into());
    }
    #[cfg(not(test))]
    if fixture.is_some() {
        return Err(AppError::Forbidden("Unsupported provider origin".into()));
    }
    Ok(match provider(p) {
        "telnyx" => "https://api.telnyx.com",
        "twilio" => "https://api.twilio.com",
        _ => "https://api.elevenlabs.io",
    }
    .into())
}
/// Credentials go only to the compiled HTTPS provider, or an explicit test-only
/// loopback HTTP fixture. A path cannot select another origin or userinfo.
pub(crate) fn provider_url(p: Profile, base: &str, path: &str) -> AppResult<url::Url> {
    let compiled = origin(p, None)?;
    if base != compiled {
        #[cfg(not(test))]
        return Err(AppError::Forbidden("Unsupported provider origin".into()));
        #[cfg(test)]
        origin(p, Some(base))?;
    }
    let base = url::Url::parse(base)
        .map_err(|_| AppError::Forbidden("Unsupported provider origin".into()))?;
    let result = base
        .join(path)
        .map_err(|_| AppError::Forbidden("Unsupported provider path".into()))?;
    if !path.starts_with('/')
        || path.starts_with("//")
        || result.origin() != base.origin()
        || !result.username().is_empty()
        || result.password().is_some()
        || result.fragment().is_some()
    {
        return Err(AppError::Forbidden("Unsupported provider path".into()));
    }
    Ok(result)
}
pub(crate) fn binding_url(
    b: &crate::models::channel_adapter::Binding,
    path: &str,
) -> AppResult<url::Url> {
    provider_url(b.profile, &binding_origin(b)?, path)
}
pub(crate) async fn bounded_response(response: reqwest::Response) -> AppResult<Vec<u8>> {
    use futures::StreamExt;
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|_| AppError::ChannelPlatformError("Provider response unavailable".into()))?;
        if body.len() + chunk.len() > 65536 {
            return Err(AppError::ChannelPlatformError(
                "Provider response exceeded limit".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

impl CustomLlmAdapter {
    pub async fn start(
        &self,
        state: crate::AppState,
        b: &crate::models::channel_adapter::Binding,
        s: &crate::models::channel_adapter::CallSession,
        body: &Value,
    ) -> AppResult<(
        tokio::sync::mpsc::Receiver<super::coordinator::Output>,
        tokio::task::JoinHandle<
            Option<crate::services::assistant_nyxagent_runtime::OwnerTurnSettlement>,
        >,
    )> {
        super::custom_llm::start(state, b, s, body).await
    }
}

pub(crate) fn binding_origin(b: &crate::models::channel_adapter::Binding) -> AppResult<String> {
    #[cfg(test)]
    if let Some(origin) = FIXTURE_ORIGINS
        .lock()
        .expect("fixture origins")
        .get(&b.account_id)
        .cloned()
    {
        return self::origin(b.profile, Some(&origin));
    }
    #[cfg(test)]
    {
        Err(AppError::VoiceProviderUnavailable)
    }
    #[cfg(not(test))]
    {
        origin(b.profile, None)
    }
}
#[cfg(test)]
pub(super) static FIXTURE_ORIGINS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
