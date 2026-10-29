//! Closed metadata only: never retain request values or provider error prose.
use super::*;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    NoAdmission,
    SharedChat,
    RequestShape,
    UnknownField,
    HandlePresent,
    ToolShape,
    MethodOverride,
    Pool,
    AgentUnavailable,
    ProfileUnknown,
    ConnectionNotPlatform,
    ConnectionOverride,
    CatalogInactiveOrNotMaster,
    TargetMismatch,
    HeaderMismatch,
    RouteMismatch,
}
impl Reason {
    pub(crate) fn fix(self) -> &'static str {
        match self {
            Self::ConnectionNotPlatform
            | Self::ConnectionOverride
            | Self::ProfileUnknown
            | Self::RouteMismatch
            | Self::TargetMismatch
            | Self::HeaderMismatch => {
                "Send one message to this assistant yourself to confirm its model route, then ask the visitor to try again."
            }
            Self::CatalogInactiveOrNotMaster => {
                "Ask your NyxID administrator to restore this assistant's platform model connection."
            }
            Self::AgentUnavailable => {
                "Restore this assistant in its settings before inviting visitors."
            }
            _ => {
                "Ask your NyxID administrator to update the NyxAgent model bridge to the supported request contract."
            }
        }
    }
    pub(crate) fn setup(self) -> bool {
        !matches!(
            self,
            Self::NoAdmission
                | Self::SharedChat
                | Self::MethodOverride
                | Self::Pool
                | Self::HandlePresent
        )
    }
    pub(crate) fn setup_code(code: &str) -> bool {
        serde::Deserialize::deserialize(
            serde::de::value::StrDeserializer::<serde::de::value::Error>::new(code),
        )
        .is_ok_and(Self::setup)
    }
    pub(crate) fn code(self) -> String {
        serde_json::to_value(self)
            .expect("closed enum")
            .as_str()
            .expect("string")
            .into()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Diagnostic {
    pub reason: Reason,
    pub unknown_fields: Vec<String>,
}
impl From<Reason> for Diagnostic {
    fn from(reason: Reason) -> Self {
        Self {
            reason,
            unknown_fields: Vec::new(),
        }
    }
}
impl Diagnostic {
    pub(super) fn unknown<'a>(names: impl Iterator<Item = &'a String>) -> Self {
        Self {
            reason: Reason::UnknownField,
            unknown_fields: names
                .filter(|name| {
                    !name.is_empty()
                        && name.len() <= 64
                        && name.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                        })
                })
                .take(8)
                .cloned()
                .collect(),
        }
    }
}

pub(super) async fn record(
    db: &Database,
    chat: &acks::ChatAuthority,
    diagnostic: Diagnostic,
) -> AppResult<()> {
    // The current callback attempt is assigned by ModelRequest before admission.
    // Only its own diagnostic may explain the eventual upstream failure.
    let changed = db.collection::<Document>("assistant_conversations").update_one(
        doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"credential_api_key_id":&chat.api_key_id,
            "active_turn.turn_id":&chat.turn_id,"active_turn.stop_requested":false},
        vec![doc! {"$set":{"active_turn.model_route_diagnostic":{
            "reason": {"$literal":diagnostic.reason.code()},
            "unknown_fields":{"$literal":bson::to_bson(&diagnostic.unknown_fields).expect("keys")},
            "attempt_id":"$active_turn.model_attempt_id", "at":bson::DateTime::now()
        }}}]
    ).await?;
    if changed.modified_count > 0 {
        if chat.guest && diagnostic.reason.setup() {
            super::readiness_notice::enqueue(db, chat, diagnostic.reason).await?;
        }
        tracing::warn!(target:"nyxid::assistant_model_route", conversation_id=%chat.conversation_id,
            agent_id=%chat.agent_id, guest=chat.guest, reason=%diagnostic.reason.code(),
            unknown_fields=?diagnostic.unknown_fields, "Model route admission diagnostic");
    }
    Ok(())
}

pub(super) async fn refuse<T>(
    db: &Database,
    chat: &acks::ChatAuthority,
    reason: impl Into<Diagnostic>,
) -> AppResult<T> {
    record(db, chat, reason.into()).await?;
    Err(AppError::GuestModelRouteRequired)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_2092_unknown_field_diagnostics_are_bounded_names_only() {
        let names: Vec<_> = [
            "prompt with secrets",
            "UPPERCASE",
            "",
            "a.b",
            "safe_field",
            "field-2",
            "one",
            "two",
            "three",
            "four",
            "five",
            "six",
            "seven",
            "eight",
            "nine",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let diagnostic = Diagnostic::unknown(names.iter());
        assert_eq!(diagnostic.reason, Reason::UnknownField);
        assert_eq!(diagnostic.unknown_fields.len(), 8);
        assert_eq!(diagnostic.unknown_fields[0], "safe_field");
    }
    #[test]
    fn readiness_2092_real_codex_contract_is_stateless_and_keeps_body() {
        for fixture in [
            include_str!("../../../specs/assistant/codex/0.164.0-chat.json"),
            include_str!("../../../specs/assistant/codex/0.164.0-research.json"),
            include_str!("../../../specs/assistant/codex/0.164.0-channel.json"),
            include_str!("../../../specs/assistant/codex/0.165.0-chat.json"),
            include_str!("../../../specs/assistant/codex/0.165.0-research.json"),
            include_str!("../../../specs/assistant/codex/0.165.0-channel.json"),
        ] {
            let body: Value = serde_json::from_str(fixture).unwrap();
            let before = body.clone();
            assert!(super::super::request_identity::stateless_inference(
                "responses",
                &body
            ));
            assert_eq!(body, before);
            for (key, value, reason) in [
                (
                    "unrecognized_flag",
                    json!("private-value"),
                    Reason::UnknownField,
                ),
                ("store", json!(true), Reason::RequestShape),
                ("background", json!(true), Reason::RequestShape),
                (
                    "tools",
                    json!([{"type":"computer_use_preview"}]),
                    Reason::ToolShape,
                ),
                (
                    "input",
                    json!([{"file_id":"private-handle"}]),
                    Reason::HandlePresent,
                ),
            ] {
                let mut rejected = body.clone();
                rejected[key] = value;
                assert_eq!(
                    super::super::request_identity::model_shape("responses", &rejected)
                        .unwrap_err()
                        .reason,
                    reason
                );
            }
        }
    }
}

#[cfg(test)]
mod transport_tests {
    #[tokio::test]
    async fn readiness_2092_bridge_headers_are_only_json_and_accept() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let body: serde_json::Value = serde_json::from_str(include_str!(
            "../../../specs/assistant/codex/0.165.0-chat.json"
        ))
        .unwrap();
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .post(format!("{}/responses", server.uri()))
            .bearer_auth("synthetic-test-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        let calls = server.received_requests().await.unwrap();
        let forwarded = crate::services::proxy_service::collect_forward_headers(&calls[0].headers);
        let expected: std::collections::BTreeMap<String, String> =
            serde_json::from_str(include_str!("../../../specs/assistant/codex/headers.json"))
                .unwrap();
        assert_eq!(
            forwarded
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
            expected
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&calls[0].body).unwrap(),
            body
        );
    }
}
