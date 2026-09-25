use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub(super) fn verification_error() -> AppError {
    AppError::ChannelWebhookVerificationFailed("Invalid X webhook".into())
}

pub(super) fn crc_token(query: &std::collections::HashMap<String, String>) -> AppResult<&str> {
    query
        .get("crc_token")
        .map(String::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(verification_error)
}

pub(super) fn handshake(
    credentials: &PlatformVerifySecrets,
    query: &std::collections::HashMap<String, String>,
) -> AppResult<String> {
    let mut mac = mac(credentials)?;
    mac.update(crc_token(query)?.as_bytes());
    Ok(json!({"response_token": format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()))}).to_string())
}

fn mac(credentials: &PlatformVerifySecrets) -> AppResult<Hmac<Sha256>> {
    let secret = credentials
        .get("consumer_secret")
        .filter(|s| !s.is_empty())
        .ok_or_else(verification_error)?;
    Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| verification_error())
}

pub(super) fn verify(
    credentials: &PlatformVerifySecrets,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    let signature = headers
        .get("x-twitter-webhooks-signature")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
        .filter(|v| v.len() == 44)
        .ok_or_else(verification_error)?;
    let signature = STANDARD
        .decode(signature)
        .map_err(|_| verification_error())?;
    let mut mac = mac(credentials)?;
    mac.update(body);
    mac.verify_slice(&signature)
        .map_err(|_| verification_error())
}

pub(super) fn target(body: &[u8]) -> AppResult<Option<String>> {
    let body: Value = serde_json::from_slice(body).map_err(|_| protocol_error())?;
    if !matches!(
        body["data"]["event_type"].as_str(),
        Some(
            "dm.received"
                | "chat.received"
                | "post.mention.create"
                | "post.reply.create"
                | "post.create"
        )
    ) {
        return Ok(None);
    }
    Ok(Some(
        body["data"]["filter"]["user_id"]
            .as_str()
            .filter(|id| numeric_id(id))
            .ok_or_else(protocol_error)?
            .to_string(),
    ))
}

pub(super) fn parse(body: &[u8]) -> AppResult<Vec<InboundMessage>> {
    let Some(own_id) = target(body)? else {
        return Ok(vec![]);
    };
    let body: Value = serde_json::from_slice(body).map_err(|_| protocol_error())?;
    if body["data"]["event_type"] == "chat.received" {
        return activity::parse_chat(&body, &own_id);
    }
    if body["data"]["event_type"] == "post.create" {
        return activity::parse_own_post(&body, &own_id);
    }
    if body["data"]["event_type"] != "dm.received" {
        return parse_post(&body, &own_id);
    }
    let payload = &body["data"]["payload"];
    let events = payload["direct_message_events"]
        .as_array()
        .ok_or_else(protocol_error)?;
    let mut messages = vec![];
    for event in events {
        if event["type"] != "message_create" {
            continue;
        }
        let created = &event["message_create"];
        let sender = created["sender_id"]
            .as_str()
            .filter(|id| numeric_id(id))
            .ok_or_else(protocol_error)?;
        let recipient = created["target"]["recipient_id"].as_str();
        if recipient != Some(own_id.as_str()) || sender == own_id {
            continue;
        }
        let id = event["id"]
            .as_str()
            .filter(|id| numeric_id(id))
            .ok_or_else(protocol_error)?;
        let conversation = created["dm_conversation_id"]
            .as_str()
            .or_else(|| event["dm_conversation_id"].as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                if (sender.len(), sender) < (own_id.len(), own_id.as_str()) {
                    format!("{sender}-{own_id}")
                } else {
                    format!("{own_id}-{sender}")
                }
            });
        let data = &created["message_data"];
        let user = &payload["users"][sender];
        let user = user.get("data").unwrap_or(user);
        let mut message = normalize(
            &json!({
                "id": id, "sender_id": sender, "dm_conversation_id": conversation,
                "text": data["text"],
            }),
            &json!({"users": [{"id": sender, "name": user["name"], "username": user["username"]}]}),
            &own_id,
        )?
        .ok_or_else(protocol_error)?;
        if let Some(media) = data.get("attachment").and_then(|a| a.get("media")) {
            let kind = match media["type"].as_str() {
                Some("photo") => Some("image"),
                Some("video" | "animated_gif") => Some("video"),
                _ => None,
            };
            let variant = media["video_info"]["variants"]
                .as_array()
                .and_then(|variants| {
                    variants
                        .iter()
                        .filter(|v| v["content_type"] == "video/mp4")
                        .max_by_key(|v| v["bitrate"].as_u64().unwrap_or(0))
                });
            let url = variant
                .and_then(|v| v["url"].as_str())
                .or_else(|| media["media_url_https"].as_str());
            if let (Some(kind), Some(url)) = (kind, url) {
                message.content_type = kind.into();
                message.attachments.push(InboundAttachment {
                    content_type: kind.into(),
                    url: url.into(),
                    platform_message_id: Some(id.into()),
                    file_key: media["id_str"].as_str().map(String::from),
                    image_key: None,
                    filename: None,
                    mime_type: variant
                        .and_then(|v| v["content_type"].as_str())
                        .map(String::from),
                    size_bytes: None,
                });
            }
        }
        message.raw_data = event.clone();
        messages.push(message);
    }
    Ok(messages)
}

fn parse_post(body: &Value, own_id: &str) -> AppResult<Vec<InboundMessage>> {
    let data = &body["data"];
    let post = &data["payload"];
    let author = post["author_id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    if author == own_id {
        return Ok(vec![]);
    }
    let addressed = match data["event_type"].as_str() {
        Some("post.mention.create") => post["entities"]["mentions"]
            .as_array()
            .is_some_and(|mentions| mentions.iter().any(|mention| mention["id"] == own_id)),
        Some("post.reply.create") => post["in_reply_to_user_id"] == own_id,
        _ => false,
    };
    if !addressed {
        return Ok(vec![]);
    }
    let id = post["id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    let conversation = post["conversation_id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    let mut normalized = post.clone();
    normalized["sender_id"] = json!(author);
    normalized["dm_conversation_id"] = json!(conversation);
    let mut message =
        normalize(&normalized, &data["includes"], own_id)?.ok_or_else(protocol_error)?;
    message.platform_message_id = id.into();
    message.conversation_id = format!("post:{conversation}");
    message.conversation_type = "channel".into();
    message.thread_id = Some(conversation.into());
    message.reply_to_platform_message_id = post["in_reply_to_tweet_id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .map(String::from)
        .or_else(|| {
            post["referenced_tweets"]
                .as_array()?
                .iter()
                .find(|reference| reference["type"] == "replied_to")?["id"]
                .as_str()
                .filter(|id| numeric_id(id))
                .map(String::from)
        });
    message.raw_data = body.clone();
    Ok(vec![message])
}

fn app_token(credentials: &PlatformVerifySecrets) -> AppResult<&str> {
    credentials
        .get("app_bearer_token")
        .filter(|s| !s.is_empty())
        .ok_or_else(protocol_error)
}

fn subscription(body: &Value) -> AppResult<&Value> {
    if body
        .get("errors")
        .is_some_and(|errors| errors.as_array().is_none_or(|rows| !rows.is_empty()))
    {
        return Err(protocol_error());
    }
    let row = body["data"].get("subscription").unwrap_or(&body["data"]);
    let row = row.as_array().and_then(|rows| rows.first()).unwrap_or(row);
    if !row["subscription_id"].as_str().is_some_and(numeric_id) {
        return Err(protocol_error());
    }
    Ok(row)
}

pub(super) fn list_data(body: &Value) -> AppResult<&[Value]> {
    if body
        .get("errors")
        .is_some_and(|errors| errors.as_array().is_none_or(|rows| !rows.is_empty()))
    {
        return Err(protocol_error());
    }
    match body.get("data") {
        Some(Value::Array(rows)) => Ok(rows),
        None if body["meta"]["result_count"] == 0 => Ok(&[]),
        _ => Err(protocol_error()),
    }
}

async fn subscriptions(
    adapter: &XAdapter,
    http: &reqwest::Client,
    token: &str,
) -> AppResult<Vec<Value>> {
    let mut result = vec![];
    let mut cursor: Option<String> = None;
    for _ in 0..200 {
        let mut request = http
            .get(format!(
                "{}/2/activity/subscriptions",
                base(adapter, &BotCredentials::from(token))
            ))
            .bearer_auth(token)
            .query(&[("max_results", "1000")]);
        if let Some(cursor) = &cursor {
            request = request.query(&[("pagination_token", cursor)]);
        }
        let body = response_json(send(request).await?).await?;
        result.extend(list_data(&body)?.iter().cloned());
        match body["meta"]["next_token"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            None => return Ok(result),
            Some(next) if next.len() <= 4096 && Some(next) != cursor.as_deref() => {
                cursor = Some(next.into())
            }
            _ => return Err(protocol_error()),
        }
    }
    Err(protocol_error())
}

pub(super) async fn setup(
    adapter: &XAdapter,
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    bot_id: &str,
    events: &[XChannelEvent],
    webhook_url: &str,
) -> AppResult<()> {
    validate_events("x", events)?;
    let app = app_token(credentials.platform_secrets.ok_or_else(protocol_error)?)?;
    let own_id = credentials
        .platform_bot_id
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    let url = url::Url::parse(webhook_url).map_err(|_| protocol_error())?;
    if url.scheme() != "https" || url.port().is_some() || webhook_url.len() > 200 {
        return Err(AppError::ValidationError(
            "X webhooks require a public HTTPS URL without a port, at most 200 characters".into(),
        ));
    }
    let api = base(adapter, credentials);
    let webhooks = response_json(
        send(
            http.get(format!("{api}/2/webhooks"))
                .bearer_auth(app)
                .query(&[("webhook_config.fields", "id,url,valid")]),
        )
        .await?,
    )
    .await?;
    let existing = list_data(&webhooks)?
        .iter()
        .find(|row| row["url"] == webhook_url);
    let webhook = if let Some(row) = existing {
        row.clone()
    } else {
        response_json(
            send(
                http.post(format!("{api}/2/webhooks"))
                    .bearer_auth(app)
                    .json(&json!({"url": webhook_url})),
            )
            .await?,
        )
        .await?["data"]
            .clone()
    };
    let webhook_id = webhook["id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    if webhook["valid"] != true {
        let response = send(
            http.put(format!("{api}/2/webhooks/{webhook_id}"))
                .bearer_auth(app),
        )
        .await?;
        let checked = response_json(response).await?;
        if checked["data"]["valid"] != true {
            return Err(protocol_error());
        }
    }
    let tag = format!("nyxid:{bot_id}");
    let existing = subscriptions(adapter, http, app).await?;
    // Keep one subscription per selected event; prefer the current callback.
    let retained: Vec<&Value> = events
        .iter()
        .filter_map(|event| {
            existing
                .iter()
                .filter(|row| {
                    row["tag"] == tag
                        && row["event_type"] == event_name(*event)
                        && row["filter"]["user_id"] == own_id
                })
                .min_by_key(|row| row["webhook_id"] != webhook_id)
        })
        .collect();
    for row in existing.iter().filter(|row| row["tag"] == tag) {
        if !retained
            .iter()
            .any(|keep| keep["subscription_id"] == row["subscription_id"])
        {
            remove_subscription(http, api, app, row).await?;
        }
    }
    for event in events {
        let name = event_name(*event);
        if existing.iter().any(|row| {
            row["tag"] == tag
                && row["event_type"] == name
                && row["filter"]["user_id"] == own_id
                && row["webhook_id"] == webhook_id
        }) {
            continue;
        }
        if let Some(row) = existing.iter().find(|row| {
            row["tag"] == tag && row["event_type"] == name && row["filter"]["user_id"] == own_id
        }) {
            let id = row["subscription_id"]
                .as_str()
                .filter(|id| numeric_id(id))
                .ok_or_else(protocol_error)?;
            let body = response_json(
                send(
                    http.put(format!("{api}/2/activity/subscriptions/{id}"))
                        .bearer_auth(credentials.token)
                        .json(&json!({"webhook_id": webhook_id, "tag": tag})),
                )
                .await?,
            )
            .await?;
            let updated = subscription(&body)?;
            if updated["subscription_id"] != id || updated["webhook_id"] != webhook_id {
                return Err(protocol_error());
            }
        } else {
            let body = response_json(send(http.post(format!("{api}/2/activity/subscriptions"))
                .bearer_auth(credentials.token)
                .json(&json!({"event_type": name, "filter": {"user_id": own_id}, "webhook_id": webhook_id, "tag": tag}))).await?).await?;
            subscription(&body)?;
        }
    }
    Ok(())
}

pub(super) async fn remove(
    adapter: &XAdapter,
    http: &reqwest::Client,
    credentials: &PlatformVerifySecrets,
    bot_id: &str,
) -> AppResult<()> {
    let app = app_token(credentials)?;
    let credentials = BotCredentials::from(app);
    let api = base(adapter, &credentials);
    let tag = format!("nyxid:{bot_id}");
    for row in subscriptions(adapter, http, app)
        .await?
        .iter()
        .filter(|row| row["tag"] == tag)
    {
        remove_subscription(http, api, app, row).await?;
    }
    Ok(())
}

async fn remove_subscription(
    http: &reqwest::Client,
    api: &str,
    token: &str,
    row: &Value,
) -> AppResult<()> {
    let id = row["subscription_id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    let response = send(
        http.delete(format!("{api}/2/activity/subscriptions/{id}"))
            .bearer_auth(token),
    )
    .await?;
    if !response.status().is_success() && response.status() != StatusCode::NOT_FOUND {
        return Err(response_error(
            response.status(),
            rate_backoff(response.headers(), response.status()),
        ));
    }
    Ok(())
}
