//! Passive credential-denial rendering. No login, refresh or identity fallback.
use serde_json::{Value, json};

pub const CHANNEL_HELP: &str = "Requires a human account session: run `nyxid login` and choose account access, or select an existing human account profile. Personal lists need ownership; --org needs org-admin access. Agent Keys cannot use this management list. For read-only agent discovery use GET /api/v1/channel-relay/conversations with the assigned active Agent Key; only that key's active assignments are returned, including device channels (platform=device). There is no separate channel-event discovery endpoint. Never switch identity or widen permissions automatically.";

const HUMAN_HINT: &str = "This command needs a human account session: ask a human to run `nyxid login` and choose account access, or explicitly select an existing account profile. Do not switch identity automatically.";
const SCOPE_HINT: &str = "Ask the owner to review the required scopes and grant only the access needed. Do not switch identity or widen permissions automatically.";
const AGENT_HINT: &str = "For read-only agent discovery, use GET /api/v1/channel-relay/conversations with the assigned active Agent Key. It includes device channels (platform=device); no bot inventory, management access or separate channel-event discovery is provided.";

pub(crate) fn for_error(error: &crate::api::ApiError) -> Option<Value> {
    if error.status() != reqwest::StatusCode::FORBIDDEN {
        return None;
    }
    let body = error.response()?;
    let details = body.details.as_ref()?;
    let reason = details["reason"].as_str()?;
    let mut guidance = match reason {
        "credential_type_unsupported" => {
            let credential = details["credential_type"].as_str()?;
            if ![
                "api_key",
                "delegated",
                "relay",
                "service_account",
                "oauth_client",
            ]
            .contains(&credential)
                || details["accepted"] != json!(["user_session"])
            {
                return None;
            }
            json!({"reason":reason,"credential_type":credential,"accepted":["user_session"],"hint":HUMAN_HINT})
        }
        "insufficient_scope" => json!({"reason":reason,"hint":SCOPE_HINT}),
        _ => return None,
    };
    // Inspect the path only to select a fixed hint. Leave the legacy error
    // path and complete server body to error_format's existing renderer.
    let channel_path = matches!(
        error.request_path(),
        "/channel-bots"
            | "/api/v1/channel-bots"
            | "/channel-conversations"
            | "/api/v1/channel-conversations"
    );
    if reason == "credential_type_unsupported" && channel_path {
        guidance["agent_alternative"] = AGENT_HINT.into();
    }
    Some(guidance)
}

pub(crate) fn text(guidance: &Value) -> Option<String> {
    let mut text = format!("Reason: {}", guidance["reason"].as_str()?);
    if let Some(credential) = guidance["credential_type"].as_str() {
        text.push_str(&format!(
            "\nCredential: {credential}; accepted for recovery: user_session."
        ));
    }
    text.push_str(&format!("\n{}", guidance["hint"].as_str()?));
    if let Some(alternative) = guidance["agent_alternative"].as_str() {
        text.push_str(&format!("\n{alternative}"));
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiError;

    fn denied(path: &str, details: Value) -> anyhow::Error {
        ApiError::new(path, reqwest::StatusCode::FORBIDDEN,
            json!({"error":"forbidden","error_code":1002,"message":"Forbidden: API keys cannot access this endpoint","details":details}).to_string()).into()
    }

    #[test]
    fn credential_guidance_is_additive_to_legacy_text_and_json() {
        for path in [
            "/channel-bots?org_id=example-org",
            "/api/v1/channel-conversations?bot_id=example-bot",
            "/assistant/nyxagent/conversations/example-thread",
        ] {
            let body = json!({
                "error":"forbidden", "error_code":1002,
                "message":"Forbidden: API keys cannot access this endpoint",
                "details":{"reason":"credential_type_unsupported", "credential_type":"api_key",
                    "accepted":["user_session"], "hint":"server hint", "future_detail":true},
                "future_server_field":{"preserve":"full body"}
            });
            let err: anyhow::Error =
                ApiError::new(path, reqwest::StatusCode::FORBIDDEN, body.to_string()).into();
            // An untyped error uses the legacy renderer without guidance.
            let legacy = anyhow::anyhow!("{err:#}");
            let old_json: Value =
                serde_json::from_str(&crate::error_format::render_error(&legacy, true)).unwrap();
            let mut rendered: Value =
                serde_json::from_str(&crate::error_format::render_error(&err, true)).unwrap();
            let guidance = rendered
                .as_object_mut()
                .unwrap()
                .remove("guidance")
                .unwrap();
            assert_eq!(rendered, old_json);
            assert_eq!(rendered["path"], path);
            assert_eq!(rendered["body"], body);
            assert_eq!(guidance["reason"], "credential_type_unsupported");
            assert_eq!(guidance["accepted"], json!(["user_session"]));
            assert_eq!(
                guidance.get("agent_alternative").is_some(),
                path.contains("/channel-")
            );
            let text = crate::error_format::render_error(&err, false);
            assert!(text.starts_with(&format!(
                "{}\nReason: credential_type_unsupported",
                crate::error_format::render_error(&legacy, false),
            )));
            assert!(text.contains("nyxid login"));
            assert!(text.contains("account access"));
            // Only fixed guidance is added; request identifiers stay solely
            // in the unchanged legacy error, never in the guidance object.
            assert!(!guidance.to_string().contains("example-"));
            assert!(
                !text.contains("nyxid_ag_private_test_key")
                    && !rendered.to_string().contains("nyxid_ag_private_test_key")
            );
        }
    }

    #[test]
    fn scope_denial_and_authentication_failure_are_not_type_denials() {
        let err = denied(
            "/proxy/private-resource",
            json!({"reason":"insufficient_scope"}),
        );
        let text = crate::error_format::render_error(&err, false);
        assert!(text.contains("insufficient_scope"));
        assert!(text.contains("owner to review"));
        assert!(!text.contains("nyxid login"));
        assert!(text.starts_with(&format!("Error: {err:#}\nReason: insufficient_scope")));
        let rendered: Value =
            serde_json::from_str(&crate::error_format::render_error(&err, true)).unwrap();
        assert_eq!(rendered["path"], "/proxy/private-resource");
        for (code, reason) in [
            (1001, "authentication_failed"),
            (2001, "credential_expired"),
        ] {
            let error: anyhow::Error = ApiError::new("/channel-bots",reqwest::StatusCode::UNAUTHORIZED,
                json!({"error":"unauthorized","error_code":code,"message":"Authentication failed","details":{"reason":reason}}).to_string()).into();
            let value: Value =
                serde_json::from_str(&crate::error_format::render_error(&error, true)).unwrap();
            assert_eq!(value["status"], 401);
            assert_eq!(value["body"]["details"]["reason"], reason);
            assert!(value.get("guidance").is_none());
        }
        let err = denied("/channel-bots", json!({}));
        assert!(!crate::error_format::render_error(&err, false).contains("account access"));
    }

    #[test]
    fn channel_list_help_documents_the_human_and_agent_paths() {
        use clap::CommandFactory;
        for parts in [
            vec!["channel-bot", "list"],
            vec!["channel-event", "channel", "list"],
        ] {
            let mut root = crate::cli::Cli::command();
            let mut command = &mut root;
            for part in parts {
                command = command.find_subcommand_mut(part).unwrap();
            }
            let help = command.render_long_help().to_string();
            assert!(help.contains("human account session"));
            assert!(help.contains("/api/v1/channel-relay/conversations"));
            assert!(help.contains("platform=device"));
        }
    }

    #[tokio::test]
    async fn denied_channel_commands_make_one_request_without_identity_fallback() {
        use crate::cli::{ChannelBotCommands, ChannelEventChannelCommands, ChannelEventCommands};
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        };

        for is_bot in [true, false] {
            let server = MockServer::start().await;
            let route = if is_bot {
                "/api/v1/channel-bots"
            } else {
                "/api/v1/channel-conversations"
            };
            Mock::given(method("GET"))
                .and(path(route))
                .and(header("authorization", "Bearer nyxid_ag_private_test_key"))
                .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                    "error":"forbidden", "error_code":1002,
                    "message":"Forbidden: API keys cannot access this endpoint",
                    "details":{"reason":"credential_type_unsupported", "credential_type":"api_key",
                        "accepted":["user_session"]}
                })))
                .expect(1)
                .mount(&server)
                .await;
            let mut auth = crate::test_support::mock_auth(server.uri());
            auth.access_token = Some("nyxid_ag_private_test_key".into());
            let result = if is_bot {
                crate::commands::channel_bot::run(ChannelBotCommands::List { org: None, auth })
                    .await
            } else {
                crate::commands::channel_event::run(ChannelEventCommands::Channel {
                    command: ChannelEventChannelCommands::List { org: None, auth },
                })
                .await
            };
            let err = result.unwrap_err();
            for json_output in [false, true] {
                let output = crate::error_format::render_error(&err, json_output);
                assert!(output.contains("credential_type_unsupported"));
                assert!(output.contains("account access"));
                assert!(output.contains("/api/v1/channel-relay/conversations"));
                assert!(!output.contains("private_test_key"));
            }
            // No refresh, login, credential creation or alternate request.
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
}
