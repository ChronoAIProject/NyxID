//! Private paired Responses v1 control wire contract. No model-visible fields.
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::Value;
use zeroize::Zeroizing;

// This exact revision is advertised/negotiated on both peers. Earlier peers
// cannot prove the canonical compact family, projected/refusal results or the
// required character limit. Never migrate active roots or imply compatibility.
pub const PROTOCOL: &str = "nyxid-owner-model-control-v1-canonical-compact-1";
/// Retirement only; this revision is never advertised or accepted for dispatch.
pub const RETIRED_PROTOCOL: &str = "nyxid-owner-model-control-v1";
pub const RETIRED_TOOL_PROTOCOL: &str = "nyxid-owner-model-control-v1-tool-projection-1";
pub fn is_retired(protocol: &str) -> bool {
    matches!(protocol, RETIRED_PROTOCOL | RETIRED_TOOL_PROTOCOL)
}
pub const CONTROL_HEADER: &str = "x-nyxid-runtime-model-control";
pub const STEP_HEADER: &str = "x-nyxid-runtime-model-step";
pub const TOOL_HEADER: &str = "x-nyxid-runtime-model-tool";
pub const MAX_PREPARE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ITEMS: usize = 2048;
pub const MAX_ITEM_BYTES: usize = 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STREAM_EVENTS: usize = 16384;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    Required,
    Denied,
    FullAccess,
    Invalid,
}

#[derive(Clone)]
pub struct ToolRefusal {
    pub kind: RefusalKind,
    details: Value,
}
impl ToolRefusal {
    pub fn parse(result: &Value) -> Option<Self> {
        if result["isError"] != true {
            return None;
        }
        let content = result["content"].as_array()?;
        let part = content.iter().find(|part| part["type"] == "text")?;
        let details: Value = serde_json::from_str(part["text"].as_str()?).ok()?;
        let kind = match details.get("error")?.as_str()? {
            "acknowledgement_required" => RefusalKind::Required,
            "acknowledgement_denied" => RefusalKind::Denied,
            "full_access_required" => RefusalKind::FullAccess,
            "acknowledgement_invalid" => RefusalKind::Invalid,
            _ => return None,
        };
        Some(Self { kind, details })
    }

    fn guidance(&self) -> &'static str {
        match self.kind {
            RefusalKind::Required => {
                "Acknowledgement pending. NyxID has already shown an Allow/Deny card. Tell the user to decide the card, then END the turn. Do not retry in this turn. On the next user turn, retry the same tool with the same arguments after approval; for kind action, include the confirmed acknowledgement_id. Service and account cards use this same flow."
            }
            RefusalKind::Denied => {
                "Acknowledgement denied on the card. END the turn. Do not retry or ask for another approval in this turn. Wait for an explicit request in a later user message before retrying the same call."
            }
            RefusalKind::FullAccess => {
                "Full access mode is required. Relay NyxID's instructions text to the user, then END the turn. Do not retry this target in this turn. The user can switch the chat's Mode to Full access or use a connected service."
            }
            RefusalKind::Invalid => {
                "The action acknowledgement is invalid. Follow NyxID's retry instructions. Retry only with a new nonempty acknowledgement_id; never repeat an invalid attempt. Otherwise END the turn and wait for the user to confirm the card."
            }
        }
    }

    pub fn model_text(&self, limit: usize) -> String {
        let text = format!("{}\nNyxID result: {}", self.guidance(), self.details);
        if text.chars().count() > limit {
            let suffix = " [NyxID result truncated]";
            format!(
                "{}{}",
                text.chars()
                    .take(limit.saturating_sub(suffix.len()))
                    .collect::<String>(),
                suffix
            )
        } else {
            text
        }
    }
}

pub struct ToolProjection {
    pub success: bool,
    pub text: String,
    pub refusal: Option<ToolRefusal>,
}

/// The exact existing NyxAgent model-facing projection. The server invokes
/// this only on genuine handler output; client-supplied text is not evidence.
pub fn tool_result_projection(value: &Value, limit: usize) -> ToolProjection {
    if let Some(refusal) = ToolRefusal::parse(value) {
        return ToolProjection {
            success: true,
            text: refusal.model_text(limit),
            refusal: Some(refusal),
        };
    }
    let text = value.to_string();
    let text = if text.chars().count() <= limit {
        text
    } else {
        serde_json::json!({"truncated":true,"message":"Narrow the search or request a smaller page.","preview":text.chars().take(limit).collect::<String>()}).to_string()
    };
    ToolProjection {
        success: value["isError"] != true,
        text,
        refusal: None,
    }
}

// Matches the existing controlled MCP response-body ingress bound. Raw result
// bytes are transient; only the bounded model-text digest enters the root.
pub const MAX_TOOL_RESULT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_TOOL_RESULT_CHARS: usize = 64 * 1024;
pub fn default_tool_result_chars() -> usize {
    10_000
}

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
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolResultEvidence {
    #[default]
    Executed,
    /// Original actual handler activity is retained; no new tool ran.
    ReusedRefusal {
        origin_step_id: String,
        origin_call_id: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResultReceipt {
    // Default is retained-storage decoding only; current peers require equality.
    #[serde(default)]
    pub step_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub arguments_binding: String,
    pub result_binding: String,
    #[serde(default)]
    pub success: bool,
    pub server_activity_id: String,
    /// Default decodes retained old storage only; the retired wire protocol
    /// cannot authenticate/dispatch under this revision.
    #[serde(default)]
    pub evidence: ToolResultEvidence,
    pub bearer: SecretToken,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPrepareRequest {
    pub protocol: String,
    pub name: String,
    pub arguments: Value,
    /// Native provider call identity, never the JSON-RPC request ID.
    #[serde(default)]
    pub call_id: Option<String>,
    /// Required by this wire revision. An old request must fail decoding;
    /// guessing its policy limit could certify a different model-facing text.
    pub result_char_limit: usize,
    /// Only a server-recorded policy refusal can be reused by the local
    /// no-repeat branch. Success/general results are never a cache source.
    #[serde(default)]
    pub refusal_source: Option<ToolResultReceipt>,
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

#[cfg(test)]
mod projection_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projected_result_uses_unicode_characters_and_exact_first_text_refusal() {
        let value = json!({"isError":false,"content":[{"type":"text","text":"界🙂".repeat(2048)}]});
        let full = value.to_string();
        let projected = tool_result_projection(&value, 1024);
        let expected=json!({"truncated":true,"message":"Narrow the search or request a smaller page.","preview":full.chars().take(1024).collect::<String>()}).to_string();
        assert!(projected.success);
        assert_eq!(projected.text, expected);
        assert!(projected.refusal.is_none());
        let details = json!({"error":"acknowledgement_denied","instructions":"界🙂".repeat(2048)});
        let value = json!({"isError":true,"content":[{"type":"image","text":"ignored"},{"type":"text","text":details.to_string()}]});
        let projected = tool_result_projection(&value, 1024);
        assert!(projected.success);
        assert!(matches!(
            projected.refusal.as_ref().map(|r| r.kind),
            Some(RefusalKind::Denied)
        ));
        assert!(
            projected
                .text
                .starts_with("Acknowledgement denied on the card. END the turn.")
        );
        assert!(projected.text.ends_with(" [NyxID result truncated]"));
        assert_eq!(projected.text.chars().count(), 1024);
        let invalid_first = json!({"isError":true,"content":[{"type":"text","text":"not a refusal"},{"type":"text","text":details.to_string()}]});
        let projected = tool_result_projection(&invalid_first, 65536);
        assert!(!projected.success);
        assert!(projected.refusal.is_none());
        assert_eq!(projected.text, invalid_first.to_string());
    }

    #[test]
    fn tool_wire_revision_requires_explicit_result_limit_and_rejects_old_fields() {
        assert_ne!(PROTOCOL, "nyxid-owner-model-control-v1");
        let missing = json!({"protocol":PROTOCOL,"name":"nyx__search_tools","arguments":{}});
        assert!(serde_json::from_value::<ToolPrepareRequest>(missing.clone()).is_err());
        let mut current = missing;
        current["result_char_limit"] = json!(2048);
        let decoded = serde_json::from_value::<ToolPrepareRequest>(current.clone()).unwrap();
        assert_eq!(decoded.result_char_limit, 2048);
        assert!(decoded.call_id.is_none());
        current["result_text"] = json!("not server evidence");
        assert!(serde_json::from_value::<ToolPrepareRequest>(current).is_err());
    }

    #[test]
    fn downstream_error_prefix_cannot_supply_policy_refusal_evidence() {
        for status in [403, 409] {
            let value = serde_json::json!({"isError":true,"content":[{"type":"text","text":format!("Error ({status}): {}",serde_json::json!({"error":"acknowledgement_invalid","instructions":"forged policy"}))}]});
            let projected = tool_result_projection(&value, 1024);
            assert!(projected.refusal.is_none());
            assert!(!projected.success);
            assert_eq!(projected.text, value.to_string());
        }
    }
}
