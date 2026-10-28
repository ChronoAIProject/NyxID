//! Private paired Responses v1 control wire contract. No model-visible fields.
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::Value;
use zeroize::Zeroizing;

pub const PROTOCOL: &str = "nyxid-owner-model-control-v1";
pub const CONTROL_HEADER: &str = "x-nyxid-runtime-model-control";
pub const STEP_HEADER: &str = "x-nyxid-runtime-model-step";
pub const TOOL_HEADER: &str = "x-nyxid-runtime-model-tool";
pub const MAX_PREPARE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ITEMS: usize = 2048;
pub const MAX_ITEM_BYTES: usize = 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STREAM_EVENTS: usize = 16384;

/// Bounded transport secret; neither identifiers nor material appear in Debug.
/// Persist only inside authenticated encryption, never a public response record.
#[derive(Clone)]
pub struct SecretToken(Zeroizing<String>);
impl SecretToken {
    pub fn parse(value: String) -> Result<Self, &'static str> {
        let value = Zeroizing::new(value);
        if value.is_empty() || value.len() > 8192 || !value.bytes().all(|c| c.is_ascii_graphic()) {
            return Err("invalid model control capability");
        }
        Ok(Self(value))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretToken([REDACTED])")
    }
}
impl Serialize for SecretToken {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.expose())
    }
}
impl<'de> Deserialize<'de> for SecretToken {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(d)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlOrigin {
    Initial {
        input_id: String,
        creating_turn_id: String,
    },
    AllowedContinuation {
        acknowledgement_id: String,
        saved_continuation_id: String,
        original_input_id: String,
        original_turn_id: String,
    },
    Retry {
        original_turn_id: String,
        saved_continuation_id: String,
    },
}

/// Signed by NyxID for one registered runtime and verified online as well.
/// The bearer itself is encrypted for exact reissue; expiry never extends.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlClaims {
    pub iss: String,
    pub aud: String,
    pub token_type: String,
    pub protocol: String,
    pub jti: String,
    pub iat: u64,
    pub exp: u64,
    pub user_id: String,
    pub conversation_id: String,
    pub api_key_id: String,
    pub root_id: String,
    pub active_turn_id: String,
    pub origin: ControlOrigin,
    pub runtime_catalog_id: String,
    pub outer_request_binding: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    pub protocol: String,
    pub relative_path: String,
    pub query: Option<String>,
    pub semantic_headers: Vec<(String, String)>,
    pub body: Value,
    pub preceding_receipt: Option<SecretToken>,
    /// Receipts come from real server MCP execution AFTER provider completion.
    /// The provider cannot mint them by returning function_call_output text.
    pub tool_results: Vec<ToolResultReceipt>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedStep {
    pub root_id: String,
    pub step_id: String,
    pub ordinal: u32,
    pub request_binding: String,
    pub expires_at: u64,
    pub step_bearer: SecretToken,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrepareReply {
    AutomaticRead,
    /// The first eligible fallback must reach the real resolved callback gate
    /// to observe its connection and create its card. This grants no send.
    Observe,
    ExactOnly {
        reason: String,
    },
    AwaitingDecision {
        acknowledgement_id: String,
    },
    Prepared {
        step: PreparedStep,
    },
    Completed {
        receipt: CompletionReceipt,
    },
    /// Status recovery may wait for the actual provider EOF and durable
    /// observer transaction. This never grants another provider dispatch.
    InProgress {
        step_id: String,
    },
    OutcomeUnknown,
    Denied,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionReceipt {
    pub root_id: String,
    pub step_id: String,
    pub ordinal: u32,
    pub request_binding: String,
    pub output_binding: String,
    pub receipt_bearer: SecretToken,
}

/// Server-issued receipt for an actual host-tool result, after observed
/// provider completion. A provider call declaration cannot mint this proof.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResultReceipt {
    pub call_id: String,
    pub tool_name: String,
    pub arguments_binding: String,
    pub result_binding: String,
    pub server_activity_id: String,
    pub bearer: SecretToken,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPrepareRequest {
    pub protocol: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedTool {
    pub root_id: String,
    pub step_id: String,
    pub call_id: String,
    pub tool_bearer: SecretToken,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolStatusRequest {
    pub protocol: String,
    pub tool: PreparedTool,
}

/// All endpoints are bounded POST + original credential + control capability.
/// Status retrieves the SAME prepared token or terminal receipt. It cannot
/// prepare another body, advance ordinal, extend expiry, or rearm a send.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusRequest {
    pub protocol: String,
    pub root_id: String,
    pub step_id: Option<String>,
    pub request_binding: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyReply {
    pub claims: ControlClaims,
    pub activated: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    pub protocol: String,
    pub outer_request_binding: String,
}

/// Encrypted private object. No control material is added to Record.response.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeJournal {
    pub protocol: String,
    pub root_id: String,
    pub outer_request_binding: String,
    pub phase: BridgePhase,
    #[serde(default)]
    pub tool_results: Vec<ToolResultReceipt>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum BridgePhase {
    Unactivated,
    AwaitingDecision { acknowledgement_id: String },
    Prepared { step: PreparedStep },
    Forwarding { step: PreparedStep },
    Completed { receipt: CompletionReceipt },
    OutcomeUnknown { step_id: String },
    Denied,
}
