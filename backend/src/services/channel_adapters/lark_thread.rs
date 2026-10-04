use super::*;
use crate::models::channel_thread::{ChannelThreadFacts, ThreadKind, ThreadSenderKind};
use crate::services::channel_adapters::thread_support as support;
use crate::services::channel_platform::BotCredentials;
use crate::services::channel_thread_service::{self as threads, *};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

impl LarkFamilyAdapter {
    async fn thread_token(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
    ) -> AppResult<zeroize::Zeroizing<String>> {
        let (app, secret) = credentials
            .token
            .split_once(':')
            .ok_or_else(threads::unavailable)?;
        // Reuse the same credential/config-bound cache as legacy sends, but
        // bound the token response on this new history/reply path as well.
        let body = json!({"app_id": app, "app_secret": secret});
        let encoded = zeroize::Zeroizing::new(body.to_string());
        let key = provider_token_exchange_service::cache_key(
            &self.base_url,
            &encoded,
            &lark_family_token_exchange_config(),
        );
        self.token_exchange_cache
            .get_or_fetch(&key, || async {
                let response = support::json(
                    http.post(format!(
                        "{}/open-apis/auth/v3/tenant_access_token/internal",
                        self.base_url
                    ))
                    .json(&body),
                )
                .await?;
                if response["code"] != 0 {
                    return Err(threads::unavailable());
                }
                let token = response["tenant_access_token"]
                    .as_str()
                    .filter(|token| !token.is_empty() && token.len() <= 4096)
                    .ok_or_else(threads::unavailable)?;
                Ok(provider_token_exchange_service::CachedToken {
                    token: token.into(),
                    expires_at: Utc::now()
                        + chrono::Duration::seconds(
                            response["expire"].as_i64().unwrap_or(7200).clamp(60, 86400),
                        ),
                })
            })
            .await
            .map(zeroize::Zeroizing::new)
    }

    async fn thread_message(
        &self,
        http: &reqwest::Client,
        token: &str,
        chat: &str,
        id: &str,
    ) -> AppResult<Value> {
        if !support::path_id(id) {
            return Err(threads::unavailable());
        }
        let page = support::json(
            http.get(format!("{}/open-apis/im/v1/messages/{id}", self.base_url))
                .bearer_auth(token),
        )
        .await?;
        if page["code"] != 0 {
            return Err(threads::unavailable());
        }
        let message = page["data"]["items"]
            .as_array()
            .filter(|items| items.len() == 1)
            .and_then(|items| items.first())
            .ok_or_else(threads::unavailable)?;
        if message["message_id"].as_str() != Some(id)
            || message["chat_id"].as_str() != Some(chat)
            || message["deleted"] == true
        {
            return Err(threads::unavailable());
        }
        Ok(message.clone())
    }

    pub(super) async fn resolve_native_thread(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        facts: &ChannelThreadFacts,
        ancestors: &[ChannelThreadFacts],
    ) -> AppResult<Option<ChannelThreadFacts>> {
        if facts.kind != ThreadKind::Native
            || !support::path_id(&facts.chat_id)
            || !support::path_id(&facts.message_id)
        {
            return Ok(None);
        }
        let mut resolved = facts.clone();
        resolved.root_id = support::chain_root(facts, ancestors);
        if resolved.root_id.is_none() {
            let token = self.thread_token(http, credentials).await?;
            let mut id = facts.message_id.clone();
            let mut seen = std::collections::HashSet::new();
            for _ in 0..ANCESTORS {
                if !seen.insert(id.clone()) {
                    return Ok(None);
                }
                let m = self
                    .thread_message(http, &token, &facts.chat_id, &id)
                    .await?;
                if let Some(alias) = m["thread_id"].as_str().filter(|id| !id.is_empty()) {
                    if resolved
                        .native_thread_id
                        .as_deref()
                        .is_some_and(|known| known != alias)
                    {
                        return Ok(None);
                    }
                    resolved.native_thread_id = Some(alias.into());
                }
                if let Some(root) = m["root_id"].as_str().filter(|id| !id.is_empty()) {
                    resolved.root_id = Some(root.into());
                    break;
                }
                match m["parent_id"].as_str().filter(|id| !id.is_empty()) {
                    Some(parent) => id = parent.into(),
                    None => {
                        // A thread alias without root/parent evidence does not
                        // make this arbitrary message the thread starter.
                        if resolved.native_thread_id.is_some() {
                            return Ok(None);
                        }
                        resolved.root_id = Some(id);
                        break;
                    }
                }
            }
        }
        Ok(resolved
            .root_id
            .as_deref()
            .filter(|id| support::path_id(id))
            .map(|_| resolved.clone()))
    }

    pub(super) async fn reply_in_thread(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        target: &ThreadReplyTarget,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        let facts = target.facts();
        let root = facts
            .root_id
            .as_deref()
            .filter(|id| support::path_id(id))
            .ok_or_else(threads::unavailable)?;
        if facts.kind != ThreadKind::Native {
            return Err(threads::unavailable());
        }
        let token = self.thread_token(http, credentials).await?;
        let (kind, content) = if let Some(a) = reply.attachments.first() {
            // The service splits captions and media into individual components;
            // reject accidental composite calls instead of dropping any content.
            if reply.attachments.len() != 1 || reply.text.is_some() || a.caption.is_some() {
                return Err(threads::unavailable());
            }
            let image = a.kind == MediaKind::Image;
            let (endpoint, field, key) = if image {
                ("images", "image", "image_key")
            } else {
                ("files", "file", "file_key")
            };
            let mut form = reqwest::multipart::Form::new().part(field, media::multipart_part(a)?);
            form = if image {
                form.text("image_type", "message")
            } else {
                form.text("file_type", "stream")
                    .text("file_name", media::safe_filename(a.filename.as_deref()))
            };
            let upload = support::json(
                http.post(format!("{}/open-apis/im/v1/{endpoint}", self.base_url))
                    .bearer_auth(token.as_str())
                    .multipart(form),
            )
            .await?;
            if upload["code"] != 0 {
                return Err(threads::unavailable());
            }
            let handle = upload["data"][key]
                .as_str()
                .ok_or_else(threads::unavailable)?;
            (field, json!({key: handle}).to_string())
        } else {
            build_send_body(reply)
        };
        let response = support::json(
            http.post(format!(
                "{}/open-apis/im/v1/messages/{root}/reply",
                self.base_url
            ))
            .bearer_auth(token.as_str())
            .json(&json!({"msg_type": kind, "content": content, "reply_in_thread": true})),
        )
        .await?;
        if response["code"] != 0 {
            return Err(threads::unavailable());
        }
        response["data"]["message_id"]
            .as_str()
            .filter(|id| support::path_id(id))
            .map(|id| Some(id.into()))
            .ok_or_else(threads::unavailable)
    }

    pub(super) async fn history_in_thread(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        target: &ThreadReplyTarget,
        before: DateTime<Utc>,
    ) -> AppResult<ThreadHistory> {
        let facts = target.facts();
        let token = self.thread_token(http, credentials).await?;
        // Known-message reads are supported without assuming a thread-container
        // listing permission. Reserve one request for a possible cold token
        // exchange, one for the trigger and one for a known ancestor.
        // This is explicitly partial history.
        let mut history = ThreadHistory {
            messages: Vec::new(),
            partial: true,
        };
        let trigger = self
            .thread_message(http, &token, &facts.chat_id, &facts.message_id)
            .await?;
        let before = before.min(message_time(&trigger).ok_or_else(threads::unavailable)?);
        let mut ids = Vec::new();
        if let Some(root) = facts.root_id.as_ref() {
            ids.push(root.clone())
        }
        if let Some(parent) = facts
            .parent_message_id
            .as_ref()
            .filter(|id| !ids.contains(id))
        {
            ids.push(parent.clone())
        }
        for id in ids
            .into_iter()
            .filter(|id| id != &facts.message_id)
            .take(HISTORY_REQUESTS - 2)
        {
            let Ok(m) = self.thread_message(http, &token, &facts.chat_id, &id).await else {
                break;
            };
            if Some(id.as_str()) != facts.root_id.as_deref()
                && m["root_id"].as_str() != facts.root_id.as_deref()
            {
                continue;
            }
            if m["msg_type"] != "text" {
                continue;
            }
            let kind = match m["sender"]["sender_type"].as_str() {
                Some("user") => ThreadSenderKind::Human,
                Some("app") => ThreadSenderKind::Bot,
                _ => continue,
            };
            let (Some(sender), Some(time), Some(content)) = (
                m["sender"]["id"].as_str(),
                message_time(&m),
                m["body"]["content"].as_str(),
            ) else {
                continue;
            };
            let Ok(content) = serde_json::from_str::<Value>(content) else {
                continue;
            };
            let Some(text) = content["text"].as_str() else {
                continue;
            };
            support::push(
                &mut history,
                ThreadHistoryMessage {
                    message_id: id,
                    sender_id: sender.into(),
                    sender_kind: kind,
                    created_at: time,
                    text: text.into(),
                },
                before,
            );
        }
        Ok(history)
    }
}

fn message_time(m: &Value) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(m["create_time"].as_str()?.parse().ok()?)
}
