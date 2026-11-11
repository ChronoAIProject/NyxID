//! Versioned capability contract derived from the executable channel registry.
use super::channel_platform::PlatformAdapter;
use serde::Serialize;

pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionTiming {
    Unsupported,
    BetweenTurns,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Unsupported,
    Accepted,
    ProviderReceipt,
    InterruptedPrefix,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    ReceiveMessage,
    SendMessage,
    EditMessage,
    ReceiveCall,
    EndSession,
    ReceiveDtmf,
    StreamReply,
    Interrupt,
    Dial,
    Transfer,
    PlayAudio,
}
#[derive(Clone, Debug, Serialize)]
pub struct Messaging {
    pub reply_to: bool,
    pub initiated_send: bool,
    pub inbound_media: &'static [super::channel_platform::MediaKind],
    pub outbound_media: &'static [super::channel_platform::MediaKind],
    pub max_attachment_bytes: Option<usize>,
    pub threads: super::channel_platform::ThreadCapabilities,
    pub inbound_text: bool,
    pub outbound_text: bool,
    pub edit: bool,
    pub max_text_bytes: Option<usize>,
    pub delivery: Evidence,
}
#[derive(Clone, Debug, Serialize)]
pub struct Calls {
    pub inbound: bool,
    pub outbound: bool,
    pub transfer: bool,
    pub dtmf_input: bool,
    pub end_session: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Conversation {
    pub final_transcripts: bool,
    pub cumulative_interim_transcripts: bool,
    pub streamed_output: bool,
    pub interruption: bool,
    pub playback_evidence: Evidence,
    pub explicit_cancel: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Instructions {
    pub external_nyxbot: bool,
    pub nyxbot_refresh: InstructionTiming,
    pub provider_prompt_replacement: InstructionTiming,
}
#[derive(Clone, Debug, Serialize)]
pub struct Lifecycle {
    pub reconnect: bool,
    pub automatic_effect_replay: bool,
    pub durable_ingress_dedup: bool,
    pub max_session_seconds: Option<u32>,
    pub max_frame_bytes: Option<usize>,
    pub output_queue_capacity: Option<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Usage {
    pub units: &'static [&'static str],
    pub evidence: &'static str,
    pub configured_billing: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct CredentialModeSupport {
    pub kind: &'static str,
    pub implemented: bool,
    pub prerequisite: Option<&'static str>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Setup {
    pub credential_modes: Vec<CredentialModeSupport>,
    pub credential_alternatives: &'static [&'static str],
    pub required_resources: &'static [&'static str],
    pub carrier: &'static str,
    pub speech: &'static str,
    pub agent: &'static str,
    pub documentation: Option<&'static str>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Descriptor {
    pub adapter_id: String,
    pub contract_version: u32,
    pub operations: Vec<Operation>,
    pub messaging: Messaging,
    pub calls: Calls,
    pub conversation: Conversation,
    pub instructions: Instructions,
    pub lifecycle: Lifecycle,
    pub usage: Usage,
    pub setup: Setup,
}

pub fn describe(adapter: &dyn PlatformAdapter) -> Descriptor {
    let out = adapter.outbound_capabilities();
    let inbound = adapter.supports_text_ingress();
    let outbound = adapter.supports_text_reply();
    let mut operations = Vec::new();
    if inbound {
        operations.push(Operation::ReceiveMessage);
    }
    if outbound {
        operations.push(Operation::SendMessage);
    }
    if out.edit {
        operations.push(Operation::EditMessage);
    }
    Descriptor {
        adapter_id: adapter.platform_id().to_owned(),
        contract_version: CONTRACT_VERSION,
        operations,
        messaging: Messaging {
            reply_to: out.reply_to,
            initiated_send: out.initiated_send,
            inbound_media: adapter.media_capabilities().inbound,
            outbound_media: adapter.media_capabilities().outbound,
            max_attachment_bytes: None,
            threads: adapter.thread_capabilities(),
            inbound_text: inbound,
            outbound_text: outbound,
            edit: out.edit,
            max_text_bytes: None,
            delivery: if outbound {
                Evidence::Accepted
            } else {
                Evidence::Unsupported
            },
        },
        calls: Calls {
            inbound: false,
            outbound: false,
            transfer: false,
            dtmf_input: false,
            end_session: false,
        },
        conversation: Conversation {
            final_transcripts: false,
            cumulative_interim_transcripts: false,
            streamed_output: false,
            interruption: false,
            playback_evidence: Evidence::Unsupported,
            explicit_cancel: false,
        },
        instructions: Instructions {
            external_nyxbot: adapter.registration().enabled && (inbound || outbound),
            nyxbot_refresh: if adapter.registration().enabled && (inbound || outbound) {
                InstructionTiming::BetweenTurns
            } else {
                InstructionTiming::Unsupported
            },
            provider_prompt_replacement: InstructionTiming::Unsupported,
        },
        lifecycle: Lifecycle {
            reconnect: false,
            automatic_effect_replay: false,
            durable_ingress_dedup: adapter.atomic_inbound_admission(),
            max_session_seconds: None,
            max_frame_bytes: None,
            output_queue_capacity: None,
        },
        usage: Usage {
            units: &[],
            evidence: "adapter_specific",
            configured_billing: adapter.platform_id() == "x",
        },
        setup: Setup {
            credential_modes: vec![CredentialModeSupport {
                kind: "channel_bot",
                implemented: true,
                prerequisite: None,
            }],
            credential_alternatives: &["channel_bot"],
            required_resources: &[],
            carrier: "none",
            speech: "none",
            agent: "nyxbot",
            documentation: adapter.registration().documentation_url,
        },
    }
}
