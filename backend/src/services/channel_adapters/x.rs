//! X user-context Direct Messages. All X protocol and product descriptors live here.

use std::time::Duration;

use axum::http::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_managed::{
    ManagedOnboardingDescriptor, PlatformCredentialBacking, PlatformCredentialDescriptor,
    PlatformCredentialField,
};
use crate::services::channel_platform::{
    BotCredentials, BotIdentity, CredentialResolution, InboundAttachment, InboundMessage,
    Ingestion, OutboundReply, PlatformAdapter, PlatformVerifySecrets, PollOutcome,
    RegistrationDescriptor,
};

pub const REQUIRED_SCOPES: &[&str] = &[
    "tweet.read",
    "users.read",
    "dm.read",
    "dm.write",
    "offline.access",
];
const TEXT_LIMIT: usize = 10_000;
const MAX_PAGES: usize = 10;

#[derive(Default)]
pub struct XAdapter {
    #[cfg(test)]
    pub(crate) api_base: Option<String>,
}

fn base<'a>(adapter: &'a XAdapter, credentials: &'a BotCredentials<'_>) -> &'a str {
    #[cfg(test)]
    if let Some(base) = &adapter.api_base {
        return base;
    }
    #[cfg(test)]
    if let Some(base) = credentials
        .platform_secrets
        .and_then(|s| s.get("test_x_base"))
    {
        return base;
    }
    let _ = (adapter, credentials);
    "https://api.x.com"
}

fn protocol_error() -> AppError {
    AppError::ChannelPlatformError(
        "X API request failed; retry or check the connected account".to_string(),
    )
}

fn numeric_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

fn rate_backoff(headers: &HeaderMap, status: StatusCode) -> Option<Duration> {
    if status != StatusCode::TOO_MANY_REQUESTS
        && headers
            .get("x-rate-limit-remaining")
            .and_then(|h| h.to_str().ok())
            != Some("0")
    {
        return None;
    }
    let reset = headers
        .get("x-rate-limit-reset")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .map(|reset| {
            reset
                .saturating_sub(chrono::Utc::now().timestamp())
                .saturating_add(1)
                .max(1) as u64
        });
    let retry = headers
        .get("retry-after")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    Some(Duration::from_secs(
        reset
            .into_iter()
            .chain(retry)
            .max()
            .unwrap_or(900)
            .clamp(1, 86400),
    ))
}

fn response_error(status: StatusCode, backoff: Option<Duration>) -> AppError {
    let cause = match status {
        StatusCode::UNAUTHORIZED => {
            "authorization expired or revoked; reconnect the account".to_string()
        }
        StatusCode::FORBIDDEN => {
            "DM access denied; check granted scopes and the app's paid API access".to_string()
        }
        StatusCode::TOO_MANY_REQUESTS => format!(
            "rate limited; retry after {}s",
            backoff.unwrap_or(Duration::from_secs(900)).as_secs()
        ),
        _ => "API request failed; retry later".to_string(),
    };
    AppError::ChannelPlatformError(format!("X: {cause} (HTTP {})", status.as_u16()))
}

async fn send(request: reqwest::RequestBuilder) -> AppResult<reqwest::Response> {
    request
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| protocol_error())
}

async fn response_json(response: reqwest::Response) -> AppResult<Value> {
    if !response.status().is_success() {
        return Err(response_error(
            response.status(),
            rate_backoff(response.headers(), response.status()),
        ));
    }
    response.json().await.map_err(|_| protocol_error())
}

fn normalize(event: &Value, includes: &Value, own_id: &str) -> AppResult<Option<InboundMessage>> {
    let sender = event["sender_id"].as_str().ok_or_else(protocol_error)?;
    if sender == own_id {
        return Ok(None);
    }
    let id = event["id"].as_str().ok_or_else(protocol_error)?;
    let conversation = event["dm_conversation_id"]
        .as_str()
        .ok_or_else(protocol_error)?;
    let attachments = event["attachments"]["media_keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|key| {
            includes["media"]
                .as_array()?
                .iter()
                .find(|m| m["media_key"] == *key)
        })
        .filter_map(|media| {
            let kind = match media["type"].as_str() {
                Some("photo") => "image",
                Some("video" | "animated_gif") => "video",
                _ => "file",
            };
            let variant = media["variants"].as_array().and_then(|variants| {
                variants
                    .iter()
                    .filter(|v| v["content_type"] == "video/mp4")
                    .max_by_key(|v| v["bit_rate"].as_u64().unwrap_or(0))
            });
            let url = variant
                .and_then(|v| v["url"].as_str())
                .or_else(|| media["url"].as_str())
                .or_else(|| media["preview_image_url"].as_str())?;
            Some(InboundAttachment {
                content_type: kind.to_string(),
                url: url.to_string(),
                platform_message_id: Some(id.to_string()),
                file_key: media["media_key"].as_str().map(String::from),
                image_key: None,
                filename: None,
                mime_type: variant
                    .and_then(|v| v["content_type"].as_str())
                    .map(String::from),
                size_bytes: None,
            })
        })
        .collect::<Vec<_>>();
    // X's one-to-one IDs are the two user IDs joined with a hyphen; groups have a single ID.
    let private = conversation
        .split_once('-')
        .is_some_and(|(a, b)| numeric_id(a) && numeric_id(b));
    Ok(Some(InboundMessage {
        platform_message_id: id.to_string(),
        conversation_id: conversation.to_string(),
        conversation_type: if private { "private" } else { "group" }.to_string(),
        sender_platform_id: sender.to_string(),
        sender_display_name: includes["users"]
            .as_array()
            .and_then(|users| users.iter().find(|u| u["id"].as_str() == Some(sender)))
            .and_then(|u| u["name"].as_str().or_else(|| u["username"].as_str()))
            .map(String::from),
        content_type: attachments
            .first()
            .map(|a| a.content_type.clone())
            .unwrap_or_else(|| "text".to_string()),
        text: event["text"].as_str().map(String::from),
        attachments,
        // referenced_tweets are shared posts, never DM reply targets. Preserve optional future event references.
        reply_to_platform_message_id: event["referenced_events"]
            .as_array()
            .and_then(|events| events.iter().find(|e| e["type"] == "replied_to"))
            .and_then(|e| e["id"].as_str())
            .map(String::from),
        thread_id: None,
        raw_data: event.clone(),
    }))
}

fn reply_bodies(reply: &OutboundReply) -> AppResult<Vec<Value>> {
    let attachments = reply.metadata.as_ref().and_then(|m| m.get("attachments"));
    if let Some(attachments) = attachments
        && !attachments.as_array().is_some_and(|a| {
            a.len() == 1
                && a.iter().all(|a| {
                    a.as_object().is_some_and(|o| o.len() == 1)
                        && a["media_id"].as_str().is_some_and(numeric_id)
                })
        })
    {
        return Err(AppError::ValidationError(
            "X replies accept one attachment with a numeric media_id".to_string(),
        ));
    }
    let chars = reply
        .text
        .as_deref()
        .unwrap_or_default()
        .chars()
        .collect::<Vec<_>>();
    let mut bodies = chars
        .chunks(TEXT_LIMIT)
        .map(|chunk| json!({"text": chunk.iter().collect::<String>()}))
        .collect::<Vec<_>>();
    if let Some(attachments) = attachments {
        if bodies.is_empty() {
            bodies.push(json!({}));
        }
        bodies[0]["attachments"] = attachments.clone();
    }
    if bodies.is_empty() {
        return Err(AppError::ValidationError(
            "X reply requires text or a media attachment".to_string(),
        ));
    }
    Ok(bodies)
}

#[async_trait::async_trait]
impl PlatformAdapter for XAdapter {
    fn platform_id(&self) -> &str {
        "x"
    }

    fn ingestion(&self) -> Ingestion {
        Ingestion::Poll {
            min_interval_secs: 60,
        }
    }

    fn credential_resolution(&self) -> CredentialResolution {
        CredentialResolution::OAuthConnection {
            provider_slug: "twitter",
            required_scopes: REQUIRED_SCOPES,
        }
    }

    fn registration(&self) -> RegistrationDescriptor {
        RegistrationDescriptor {
            fields: &[],
            extra_fields: &[],
            token_fields: &[],
            managed_only: true,
            webhook_ingestion: false,
            preserve_subscription_on_verify: true,
            ..Default::default()
        }
    }

    fn platform_credentials(&self) -> Option<PlatformCredentialDescriptor> {
        Some(PlatformCredentialDescriptor {
            provider: "x",
            label: "X (Twitter)",
            backing: PlatformCredentialBacking::ProviderOAuth {
                provider_slug: "twitter",
            },
            fields: &[
                PlatformCredentialField {
                    name: "client_id",
                    label: "Client ID",
                    secret: true,
                    required: true,
                    numeric: false,
                    help: "X Developer Console > App > OAuth 2.0 Client ID.",
                },
                PlatformCredentialField {
                    name: "client_secret",
                    label: "Client Secret",
                    secret: true,
                    required: true,
                    numeric: false,
                    help: "X Developer Console > App > OAuth 2.0 Client Secret.",
                },
            ],
            webhook_secret_field: None,
            setup_checklist: &[
                "Enable OAuth 2.0 user authentication with a confidential Web App and PKCE in the X Developer Console. Use the OAuth callback URL below.",
                "Allow tweet.read users.read dm.read dm.write offline.access. Existing accounts must consent again to grant DM access.",
                "Fund NyxID's app with paid API credits. All customers' DM traffic consumes this app's credits and limits. Current pricing: https://docs.x.com/x-api/getting-started/pricing (pay-per-usage replaces Basic/Pro subscriptions).",
                "Automated replies require an inbound DM and user consent. NyxID never initiates DM conversations. Publish an opt-out policy for your agent.",
            ],
        })
    }

    fn managed_onboarding(&self) -> Option<ManagedOnboardingDescriptor> {
        Some(ManagedOnboardingDescriptor {
            flow: "oauth_connection",
            provider: "x",
            bootstrap_fields: &[],
            completion_fields: &["connection_id"],
            graph_version: "",
            signup_version: "",
            signup_extras: |_| json!({}),
            feature_types: &[],
        })
    }

    fn dedup_inbound_by_platform_message_id(&self) -> bool {
        true
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        let body = response_json(
            send(
                http.get(format!("{}/2/users/me", base(self, credentials)))
                    .query(&[("user.fields", "username,name")])
                    .bearer_auth(credentials.token),
            )
            .await?,
        )
        .await?;
        let id = body["data"]["id"]
            .as_str()
            .filter(|id| numeric_id(id))
            .ok_or_else(protocol_error)?;
        let username = body["data"]["username"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(protocol_error)?;
        Ok(BotIdentity {
            platform_bot_id: id.to_string(),
            platform_bot_username: username.to_string(),
        })
    }

    async fn poll_inbound(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        cursor: Option<&str>,
    ) -> AppResult<PollOutcome> {
        let own_id = credentials.platform_bot_id.ok_or_else(protocol_error)?;
        if !numeric_id(own_id) || cursor.is_some_and(|id| !numeric_id(id)) {
            return Err(protocol_error());
        }
        let mut pagination: Option<String> = None;
        let mut newest = cursor.map(String::from);
        let mut messages = Vec::new();
        for page in 0..MAX_PAGES {
            let mut request = http.get(format!("{}/2/dm_events", base(self, credentials)))
                .bearer_auth(credentials.token).query(&[
                    ("event_types", "MessageCreate"),
                    ("dm_event.fields", "id,text,sender_id,dm_conversation_id,created_at,attachments,referenced_tweets"),
                    ("expansions", "sender_id,attachments.media_keys"),
                    ("user.fields", "username,name"),
                    ("media.fields", "url,preview_image_url,type,variants"), ("max_results", "100"),
                ]);
            if let Some(token) = &pagination {
                request = request.query(&[("pagination_token", token)]);
            }
            let response = send(request).await?;
            let backoff = rate_backoff(response.headers(), response.status());
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                return Ok(PollOutcome {
                    messages: vec![],
                    cursor: cursor.map(String::from),
                    backoff,
                });
            }
            let body = response_json(response).await?;
            if body.get("errors").is_some() {
                return Err(protocol_error());
            }
            let empty = Vec::new();
            let events = match body.get("data") {
                None if body["meta"]["result_count"] == 0 => &empty,
                Some(Value::Array(events)) => events,
                _ => return Err(protocol_error()),
            };
            if page == 0 {
                if let Some(event) = events.first() {
                    let id = event["id"]
                        .as_str()
                        .filter(|id| numeric_id(id))
                        .ok_or_else(protocol_error)?;
                    if cursor.is_none_or(|cursor| (id.len(), id) > (cursor.len(), cursor)) {
                        newest = Some(id.to_string());
                    }
                }
                if cursor.is_none() {
                    return Ok(PollOutcome {
                        messages: vec![],
                        cursor: Some(newest.unwrap_or_else(|| "0".to_string())),
                        backoff,
                    });
                }
            }
            let mut reached_cursor = false;
            for event in events {
                let id = event["id"]
                    .as_str()
                    .filter(|id| numeric_id(id))
                    .ok_or_else(protocol_error)?;
                if cursor.is_some_and(|cursor| (id.len(), id) <= (cursor.len(), cursor)) {
                    reached_cursor = true;
                    break;
                }
                if let Some(message) = normalize(event, &body["includes"], own_id)? {
                    messages.push(message);
                }
            }
            let next = body["meta"]["next_token"]
                .as_str()
                .filter(|s| !s.is_empty());
            if reached_cursor || next.is_none() {
                messages.reverse();
                return Ok(PollOutcome {
                    messages,
                    cursor: newest,
                    backoff,
                });
            }
            if backoff.is_some() {
                // Never advance past an unfinished page walk or acknowledge a partial batch.
                return Ok(PollOutcome {
                    messages: vec![],
                    cursor: cursor.map(String::from),
                    backoff,
                });
            }
            let next = next
                .filter(|s| s.len() <= 4096 && Some(*s) != pagination.as_deref())
                .ok_or_else(protocol_error)?;
            pagination = Some(next.to_string());
        }
        Err(AppError::ChannelPlatformError("X DM backlog exceeds the bounded polling window; reconnect to resume from current messages".to_string()))
    }

    fn supports_reply_metadata(&self, metadata: &Value) -> bool {
        metadata.get("attachments").is_some()
    }

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        if !numeric_id(conversation_id)
            && !conversation_id
                .split_once('-')
                .is_some_and(|(a, b)| numeric_id(a) && numeric_id(b))
        {
            return Err(AppError::ValidationError(
                "Invalid X DM conversation ID".to_string(),
            ));
        }
        let mut last = None;
        for body in reply_bodies(reply)? {
            let response = response_json(
                send(
                    http.post(format!(
                        "{}/2/dm_conversations/{conversation_id}/messages",
                        base(self, credentials)
                    ))
                    .bearer_auth(credentials.token)
                    .json(&body),
                )
                .await?,
            )
            .await?;
            last = Some(
                response["data"]["dm_event_id"]
                    .as_str()
                    .ok_or_else(protocol_error)?
                    .to_string(),
            );
        }
        Ok(last)
    }

    async fn verify_webhook(
        &self,
        _bot: &ChannelBot,
        _secrets: Option<&PlatformVerifySecrets>,
        _headers: &HeaderMap,
        _body: &[u8],
    ) -> AppResult<()> {
        Err(AppError::ChannelWebhookVerificationFailed(
            "This channel uses polling".to_string(),
        ))
    }
    async fn parse_inbound(&self, _body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        Err(protocol_error())
    }
    async fn register_webhook(
        &self,
        _http: &reqwest::Client,
        _token: &str,
        _url: &str,
        _secret: &str,
    ) -> AppResult<()> {
        Err(protocol_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    fn credentials() -> BotCredentials<'static> {
        BotCredentials {
            token: "private-test-token",
            platform_bot_id: Some("10"),
            platform_secrets: None,
        }
    }

    fn event(id: &str, sender: &str) -> Value {
        json!({ "id": id, "event_type": "MessageCreate", "sender_id": sender, "dm_conversation_id": "10-20", "text": "private DM" })
    }

    fn adapter(server: &MockServer) -> XAdapter {
        XAdapter {
            api_base: Some(server.uri()),
        }
    }

    #[tokio::test]
    async fn initialization_never_replays_history_and_empty_accounts_get_baseline() {
        for (body, expected) in [
            (
                json!({"data": [event("100", "20")], "meta": {"next_token": "older"}}),
                "100",
            ),
            (json!({"meta": {"result_count": 0}}), "0"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/2/dm_events"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(&server)
                .await;
            let outcome = adapter(&server)
                .poll_inbound(&reqwest::Client::new(), &credentials(), None)
                .await
                .unwrap();
            assert_eq!(outcome.cursor.as_deref(), Some(expected));
            assert!(outcome.messages.is_empty());
        }
    }

    #[tokio::test]
    async fn pagination_stops_at_cursor_skips_self_and_orders_oldest_first() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/2/dm_events"))
            .and(header("authorization", "Bearer private-test-token"))
            .and(query_param("event_types", "MessageCreate")).and(query_param("max_results", "100"))
            .and(|r: &wiremock::Request| !r.url.query_pairs().any(|(k, _)| k == "pagination_token" || k == "since_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [event("105", "20"), event("104", "10")], "meta": {"next_token": "older"}})))
            .expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path("/2/dm_events")).and(query_param("pagination_token", "older"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [event("103", "20"), event("100", "20"), event("99", "20")], "meta": {"next_token": "must-not-fetch"}})))
            .expect(1).mount(&server).await;
        let outcome = adapter(&server)
            .poll_inbound(&reqwest::Client::new(), &credentials(), Some("100"))
            .await
            .unwrap();
        assert_eq!(outcome.cursor.as_deref(), Some("105"));
        assert_eq!(
            outcome
                .messages
                .iter()
                .map(|m| m.platform_message_id.as_str())
                .collect::<Vec<_>>(),
            ["103", "105"]
        );
        assert!(!format!("{outcome:?}").contains("private DM"));
    }

    #[tokio::test]
    async fn rate_limits_preserve_cursor_until_whole_page_walk_completes() {
        for (status, partial, expected) in
            [(429, false, "100"), (200, true, "100"), (200, false, "105")]
        {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/2/dm_events"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("x-rate-limit-remaining", "0")
                        .insert_header("retry-after", "120")
                        .set_body_json(if partial {
                            json!({"data": [event("105", "20")], "meta": {"next_token": "older"}})
                        } else {
                            json!({"data": [event("105", "20")]})
                        }),
                )
                .expect(1)
                .mount(&server)
                .await;
            let outcome = adapter(&server)
                .poll_inbound(&reqwest::Client::new(), &credentials(), Some("100"))
                .await
                .unwrap();
            assert_eq!(outcome.cursor.as_deref(), Some(expected));
            assert_eq!(outcome.backoff, Some(Duration::from_secs(120)));
            assert_eq!(
                outcome.messages.len(),
                usize::from(status == 200 && !partial)
            );
        }
    }

    #[test]
    fn normalized_media_and_group_references_preserve_protocol_meaning() {
        let mut event = event("105", "20");
        event["dm_conversation_id"] = json!("999");
        event["referenced_tweets"] = json!([{"id": "30"}]);
        event["attachments"] = json!({"media_keys": ["3_media"]});
        let includes = json!({"users": [{"id": "20", "name": "Sender", "username": "sender"}],
            "media": [{"media_key": "3_media", "type": "video", "preview_image_url": "https://media.example/preview",
                "variants": [{"bit_rate": 100, "content_type": "video/mp4", "url": "https://media.example/video"}]}]});
        let message = normalize(&event, &includes, "10").unwrap().unwrap();
        assert_eq!(message.conversation_type, "group");
        assert_eq!(message.sender_display_name.as_deref(), Some("Sender"));
        assert_eq!(message.attachments[0].url, "https://media.example/video");
        assert_eq!(
            message.attachments[0].mime_type.as_deref(),
            Some("video/mp4")
        );
        assert!(message.reply_to_platform_message_id.is_none());
        event["referenced_events"] = json!([{"type": "replied_to", "id": "104"}]);
        assert_eq!(
            normalize(&event, &includes, "10")
                .unwrap()
                .unwrap()
                .reply_to_platform_message_id
                .as_deref(),
            Some("104")
        );
    }

    #[tokio::test]
    async fn replies_chunk_unicode_and_attach_media_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/2/dm_conversations/10-20/messages"))
            .and(header("authorization", "Bearer private-test-token"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"data": {"dm_event_id": "110"}})),
            )
            .expect(2)
            .mount(&server)
            .await;
        let text = "\u{1f642}".repeat(TEXT_LIMIT + 1);
        let reply = OutboundReply {
            text: Some(text.clone()),
            metadata: Some(json!({"attachments": [{"media_id": "123"}]})),
            reply_to_platform_message_id: None,
        };
        let id = adapter(&server)
            .send_reply(&reqwest::Client::new(), &credentials(), "10-20", &reply)
            .await
            .unwrap();
        assert_eq!(id.as_deref(), Some("110"));
        let requests = server.received_requests().await.unwrap();
        let bodies = requests
            .iter()
            .map(|r| r.body_json::<Value>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            bodies[0]["text"].as_str().unwrap().chars().count(),
            TEXT_LIMIT
        );
        assert_eq!(bodies[1]["text"].as_str().unwrap().chars().count(), 1);
        assert_eq!(
            format!(
                "{}{}",
                bodies[0]["text"].as_str().unwrap(),
                bodies[1]["text"].as_str().unwrap()
            ),
            text
        );
        assert!(bodies[0].get("attachments").is_some());
        assert!(bodies[1].get("attachments").is_none());
    }

    #[tokio::test]
    async fn reply_errors_are_local_and_include_retry_after() {
        for (status, expected) in [
            (401, "reconnect"),
            (403, "scopes"),
            (429, "retry after 45s"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("retry-after", "45")
                        .set_body_json(json!({"error": "UPSTREAM-SECRET private DM"})),
                )
                .mount(&server)
                .await;
            let reply = OutboundReply {
                text: Some("reply".into()),
                metadata: None,
                reply_to_platform_message_id: None,
            };
            let error = adapter(&server)
                .send_reply(&reqwest::Client::new(), &credentials(), "999", &reply)
                .await
                .unwrap_err();
            assert!(matches!(error, AppError::ChannelPlatformError(_)));
            assert!(error.to_string().contains(expected));
            assert!(!error.to_string().contains("UPSTREAM-SECRET"));
        }
    }

    #[tokio::test]
    async fn cursor_never_moves_backwards_or_accepts_malformed_initial_event() {
        for (id, cursor, success) in [("99", Some("100"), true), ("not-an-id", None, false)] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"data": [event(id, "20")]})),
                )
                .mount(&server)
                .await;
            let result = adapter(&server)
                .poll_inbound(&reqwest::Client::new(), &credentials(), cursor)
                .await;
            if success {
                let outcome = result.unwrap();
                assert_eq!(outcome.cursor.as_deref(), cursor);
                assert!(outcome.messages.is_empty());
            } else {
                assert!(result.is_err());
            }
        }
    }
}
