use super::*;
use crate::models::channel_thread::{ChannelThreadFacts, ThreadKind, ThreadSenderKind};
use crate::services::channel_adapters::thread_support as support;
use crate::services::channel_platform::BotCredentials;
use crate::services::channel_thread_service::{self as threads, *};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

impl DiscordAdapter {
    async fn thread_message(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        chat: &str,
        id: &str,
    ) -> AppResult<Value> {
        if !support::number(chat) || !support::number(id) {
            return Err(threads::unavailable());
        }
        let message = support::json(
            http.get(format!("{}/channels/{chat}/messages/{id}", self.base_url))
                .header("Authorization", format!("Bot {}", credentials.token)),
        )
        .await?;
        if message["id"].as_str() != Some(id)
            || message["channel_id"].as_str() != Some(chat)
            || message["message_reference"]["channel_id"]
                .as_str()
                .is_some_and(|id| id != chat)
        {
            return Err(threads::unavailable());
        }
        Ok(message)
    }

    pub(super) async fn resolve_native_thread(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        facts: &ChannelThreadFacts,
        ancestors: &[ChannelThreadFacts],
    ) -> AppResult<Option<ChannelThreadFacts>> {
        if !support::number(&facts.chat_id) || !support::number(&facts.message_id) {
            return Ok(None);
        }
        // A gateway event need not contain channel metadata. Always prove the
        // actual channel type, rather than interpreting an embedded thread object.
        let channel = support::json(
            http.get(format!("{}/channels/{}", self.base_url, facts.chat_id))
                .header("Authorization", format!("Bot {}", credentials.token)),
        )
        .await?;
        if channel["id"].as_str() != Some(&facts.chat_id) {
            return Ok(None);
        }
        let mut resolved = facts.clone();
        match channel["type"].as_u64() {
            Some(10..=12) => {
                if channel["thread_metadata"]["archived"] == true
                    || channel["thread_metadata"]["locked"] == true
                {
                    return Ok(None);
                }
                let Some(parent) = channel["parent_id"]
                    .as_str()
                    .filter(|id| support::number(id))
                else {
                    return Ok(None);
                };
                resolved.kind = ThreadKind::Native;
                resolved.root_id = Some(facts.chat_id.clone());
                resolved.native_thread_id = resolved.root_id.clone();
                resolved.parent_chat_id = Some(parent.into());
            }
            Some(0 | 5) => {
                resolved.kind = ThreadKind::ReplyChain;
                resolved.native_thread_id = None;
                resolved.parent_chat_id = None;
                if facts.kind != ThreadKind::ReplyChain {
                    resolved.root_id = None;
                }
                // A previously unresolved kind has no root; standalone messages
                // seed their own chain, replies must prove ancestry.
                resolved.root_id = if resolved.parent_message_id.is_none() {
                    Some(resolved.message_id.clone())
                } else {
                    support::chain_root(&resolved, ancestors)
                };
                let mut next = resolved.parent_message_id.clone();
                let mut seen = std::collections::HashSet::from([resolved.message_id.clone()]);
                for _ in 0..ANCESTORS {
                    if resolved.root_id.is_some() {
                        break;
                    }
                    let Some(id) = next else { break };
                    if !seen.insert(id.clone()) {
                        return Ok(None);
                    }
                    let message = self
                        .thread_message(http, credentials, &facts.chat_id, &id)
                        .await?;
                    next = message["message_reference"]["message_id"]
                        .as_str()
                        .map(str::to_owned);
                    if next.is_none() {
                        resolved.root_id = Some(id);
                    }
                }
            }
            _ => return Ok(None),
        }
        Ok(resolved
            .root_id
            .as_deref()
            .filter(|id| support::number(id))
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
        if !support::number(&facts.chat_id)
            || !support::number(&facts.message_id)
            || !matches!(facts.kind, ThreadKind::Native | ThreadKind::ReplyChain)
        {
            return Err(threads::unavailable());
        }
        // No interaction metadata is consulted on the bound path. Fail if the
        // anchor is gone; do not silently send a top-level channel message.
        let mut body = json!({"content": reply.text.as_deref().unwrap_or_default(),
            "message_reference": {"message_id": facts.message_id, "channel_id": facts.chat_id, "fail_if_not_exists": true},
            "allowed_mentions": {"parse": [], "replied_user": false}});
        let request = http
            .post(format!(
                "{}/channels/{}/messages",
                self.base_url, facts.chat_id
            ))
            .header("Authorization", format!("Bot {}", credentials.token));
        let request = if reply.attachments.is_empty() {
            request.json(&body)
        } else {
            body["attachments"] = json!(reply.attachments.iter().enumerate().map(|(i,a)|
                json!({"id": i, "filename": media::safe_filename(a.filename.as_deref())})).collect::<Vec<_>>());
            let mut form = reqwest::multipart::Form::new().text("payload_json", body.to_string());
            for (i, a) in reply.attachments.iter().enumerate() {
                form = form.part(format!("files[{i}]"), media::multipart_part(a)?);
            }
            request.multipart(form)
        };
        let response = support::json(request).await?;
        response["id"]
            .as_str()
            .filter(|id| support::number(id))
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
        let mut history = ThreadHistory::default();
        let mut cursor = if facts.kind == ThreadKind::Native {
            Some(facts.message_id.clone())
        } else {
            history.partial = true;
            facts.parent_message_id.clone()
        };
        let mut seen = std::collections::HashSet::new();
        for _ in 0..HISTORY_REQUESTS {
            let Some(id) = cursor.take().filter(|id| support::number(id)) else {
                break;
            };
            if !seen.insert(id.clone()) {
                history.partial = true;
                break;
            }
            let page = if facts.kind == ThreadKind::Native {
                support::json(
                    http.get(format!(
                        "{}/channels/{}/messages",
                        self.base_url, facts.chat_id
                    ))
                    .header("Authorization", format!("Bot {}", credentials.token))
                    .query(&[("before", id.as_str()), ("limit", "20")]),
                )
                .await
            } else {
                self.thread_message(http, credentials, &facts.chat_id, &id)
                    .await
                    .map(|m| json!([m]))
            };
            let Ok(page) = page else {
                history.partial = true;
                break;
            };
            let Some(messages) = page.as_array() else {
                history.partial = true;
                break;
            };
            for m in messages {
                if m["channel_id"].as_str() != Some(&facts.chat_id)
                    || m.get("webhook_id").is_some()
                    || !matches!(m["type"].as_u64(), Some(0 | 19))
                {
                    continue;
                }
                let (Some(mid), Some(sender), Some(text), Some(time)) = (
                    m["id"].as_str(),
                    m["author"]["id"].as_str(),
                    m["content"].as_str(),
                    support::timestamp(&m["timestamp"]),
                ) else {
                    continue;
                };
                // Snowflakes order messages; provider times alone cannot prove
                // that a delayed webhook's trigger precedes a history message.
                if mid.parse::<u64>().ok() >= facts.message_id.parse::<u64>().ok() {
                    continue;
                }
                let kind = match m["author"].get("bot") {
                    None | Some(Value::Bool(false)) => ThreadSenderKind::Human,
                    Some(Value::Bool(true)) => ThreadSenderKind::Bot,
                    _ => continue,
                };
                support::push(
                    &mut history,
                    ThreadHistoryMessage {
                        message_id: mid.into(),
                        sender_id: sender.into(),
                        sender_kind: kind,
                        created_at: time,
                        text: text.into(),
                        participant_hashes: Vec::new(),
                    },
                    before,
                );
            }
            if facts.kind == ThreadKind::Native {
                if messages.len() < HISTORY_MESSAGES {
                    break;
                }
                cursor = messages
                    .last()
                    .and_then(|m| m["id"].as_str())
                    .map(str::to_owned);
            } else {
                cursor = (Some(id.as_str()) != facts.root_id.as_deref())
                    .then(|| {
                        messages
                            .first()
                            .and_then(|m| m["message_reference"]["message_id"].as_str())
                            .map(str::to_owned)
                    })
                    .flatten();
            }
            if history.messages.len() >= HISTORY_MESSAGES {
                history.partial |= cursor.is_some();
                break;
            }
        }
        history.partial |= cursor.is_some();
        Ok(history)
    }
}
