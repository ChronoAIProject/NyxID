//! Positive identity and destination evidence from authenticated channel ingress.

use super::AnswerAudience;
use crate::models::{
    channel_bot::ChannelBot, channel_message::ChannelMessage, channel_thread::ChannelThreadFacts,
};
use crate::services::channel_platform::{InboundMessage, canonical_platform};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub(crate) struct TransportFacts<'a> {
    pub sender_id: Option<&'a str>,
    pub audience: AnswerAudience,
    pub shared_room_verified: bool,
}

impl Default for TransportFacts<'_> {
    fn default() -> Self {
        Self {
            sender_id: None,
            audience: AnswerAudience::Shared,
            shared_room_verified: false,
        }
    }
}

/// Canonical identity of one authenticated platform reply address. Hash the
/// tuple because some adapters use a short-lived interaction secret as a selector.
pub(crate) fn destination(chat_id: &str, thread_id: Option<&str>) -> String {
    let mut hash = Sha256::new();
    hash.update(b"nyxid.channel-guest.destination.v1");
    hash.update((chat_id.len() as u64).to_be_bytes());
    hash.update(chat_id.as_bytes());
    if let Some(thread) = thread_id {
        hash.update([1]);
        hash.update(thread.as_bytes());
    } else {
        hash.update([0]);
    }
    hex::encode(hash.finalize())
}

pub(crate) fn inbound_facts<'a>(
    bot: &ChannelBot,
    message: &'a InboundMessage,
    discord: Option<&ChannelThreadFacts>,
) -> TransportFacts<'a> {
    authenticated_facts(
        &bot.platform,
        &bot.platform_bot_id,
        &message.raw_data,
        &message.conversation_id,
        &message.platform_message_id,
        &message.sender_platform_id,
        discord,
    )
}

/// Bind the server's persisted provider evidence to the original local source
/// and the destination the caller resolved independently.
pub(crate) fn stored_facts<'a>(
    bot: &ChannelBot,
    source: &ChannelMessage,
    conversation_id: &str,
    sender_id: &'a str,
    thread_id: Option<&str>,
) -> TransportFacts<'a> {
    if source.direction != "inbound"
        || source.channel_bot_id.as_deref() != Some(&bot.id)
        || source.user_id != bot.user_id
        || source.platform != bot.platform
        || source.platform_conversation_id.as_deref() != Some(conversation_id)
        || sender_id.is_empty()
        || source.sender_platform_id.as_deref() != Some(sender_id)
        || source.thread_id.as_deref() != thread_id
    {
        return TransportFacts::default();
    }
    source
        .guest_transport_audience
        .map_or_else(TransportFacts::default, |audience| TransportFacts {
            sender_id: Some(sender_id),
            audience,
            shared_room_verified: audience == AnswerAudience::Shared
                && source.guest_transport_shared_room_verified,
        })
}

/// Only use with facts derived by the authenticated Discord socket verifier
/// or the bot-authenticated REST message and channel verifier.
fn discord_facts<'a>(
    bot_platform_id: &str,
    chat_id: &str,
    message_id: &str,
    sender_id: &'a str,
    facts: &ChannelThreadFacts,
) -> TransportFacts<'a> {
    use crate::models::channel_thread::ThreadSenderKind;
    use crate::services::discord_gateway::valid_message_id;

    if !valid_message_id(chat_id)
        || !valid_message_id(message_id)
        || !valid_message_id(sender_id)
        || sender_id == bot_platform_id
        || facts.version != 1
        || facts.chat_id != chat_id
        || facts.message_id != message_id
        || facts.sender_kind != ThreadSenderKind::Human
    {
        return TransportFacts::default();
    }
    let Some(discord) = facts.discord_gateway.as_ref().filter(|discord| {
        discord.version == 1 && discord.bot_id == bot_platform_id && !discord.author_is_bot
    }) else {
        return TransportFacts::default();
    };
    discord_audience(sender_id, Some(u64::from(discord.channel_type)))
}

pub(crate) fn discord_shared_room(kind: Option<u64>, guild_id: &Value) -> bool {
    match kind {
        Some(3) => guild_id.is_null(),
        Some(0 | 2 | 5 | 10 | 11 | 12 | 13) => guild_id
            .as_str()
            .is_some_and(crate::services::discord_gateway::valid_message_id),
        _ => false,
    }
}

fn discord_audience(sender_id: &str, kind: Option<u64>) -> TransportFacts<'_> {
    // The caller supplies the authenticated guild/group-DM relationship.
    verified(sender_id, kind == Some(1))
}

fn verified(sender_id: &str, private: bool) -> TransportFacts<'_> {
    TransportFacts {
        sender_id: Some(sender_id),
        shared_room_verified: false,
        audience: if private {
            AnswerAudience::Private
        } else {
            AnswerAudience::Shared
        },
    }
}

fn provider_id(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() > prefix.len()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn authenticated_facts<'a>(
    platform: &str,
    bot_platform_id: &str,
    raw: &Value,
    chat_id: &str,
    message_id: &str,
    sender_id: &'a str,
    discord: Option<&ChannelThreadFacts>,
) -> TransportFacts<'a> {
    let unknown = TransportFacts::default();
    match canonical_platform(platform) {
        "telegram" => {
            let Some(message) = raw.get("message").or_else(|| raw.get("edited_message")) else {
                return unknown;
            };
            let from = &message["from"];
            let chat = &message["chat"];
            if !message["message_id"]
                .as_i64()
                .is_some_and(|id| id > 0 && id.to_string() == message_id)
                || !chat["id"]
                    .as_i64()
                    .is_some_and(|id| id != 0 && id.to_string() == chat_id)
                || !from["id"]
                    .as_i64()
                    .is_some_and(|id| id > 0 && id.to_string() == sender_id)
                || from["is_bot"] != false
                || !message["sender_chat"].is_null()
                || sender_id == bot_platform_id
            {
                return unknown;
            }
            let mut facts = verified(sender_id, chat["type"] == "private" && chat_id == sender_id);
            facts.shared_room_verified =
                matches!(chat["type"].as_str(), Some("group" | "supergroup"))
                    && chat["id"].as_i64().is_some_and(|id| id < 0);
            facts
        }
        "lark" | "feishu" => {
            let message = &raw["event"]["message"];
            let sender = &raw["event"]["sender"];
            if raw["header"]["event_type"] != "im.message.receive_v1"
                || raw["header"]["app_id"].as_str() != Some(bot_platform_id)
                || sender["sender_type"] != "user"
                || sender["sender_id"]["open_id"].as_str() != Some(sender_id)
                || !provider_id(sender_id, "ou_")
                || message["message_id"].as_str() != Some(message_id)
                || !provider_id(message_id, "om_")
                || message["chat_id"].as_str() != Some(chat_id)
                || !provider_id(chat_id, "oc_")
            {
                return unknown;
            }
            let mut facts = verified(sender_id, message["chat_type"] == "p2p");
            facts.shared_room_verified = message["chat_type"] == "group";
            facts
        }
        "discord" => {
            use crate::services::discord_gateway::valid_message_id as snowflake;

            if let Some(facts) = discord {
                let Some(evidence) = facts.discord_gateway.as_ref() else {
                    return unknown;
                };
                if raw["author"]["id"].as_str() != Some(sender_id)
                    || raw["channel"]["id"].as_str() != Some(chat_id)
                    || raw["channel"]["type"].as_u64() != Some(u64::from(evidence.channel_type))
                    || raw["channel"]["guild_id"] != raw["guild_id"]
                    || (matches!(evidence.channel_type, 1 | 3) && !raw["guild_id"].is_null())
                {
                    return unknown;
                }
                let mut result =
                    discord_facts(bot_platform_id, chat_id, message_id, sender_id, facts);
                result.shared_room_verified = result.sender_id.is_some()
                    && discord_shared_room(
                        Some(u64::from(evidence.channel_type)),
                        &raw["channel"]["guild_id"],
                    );
                return result;
            }
            let user = if raw["guild_id"].is_null() {
                &raw["user"]
            } else {
                &raw["member"]["user"]
            };
            if !matches!(raw["type"].as_u64(), Some(2 | 3 | 5))
                || raw["application_id"].as_str() != Some(bot_platform_id)
                || raw["id"].as_str() != Some(message_id)
                || raw["channel_id"].as_str() != Some(chat_id)
                || user["id"].as_str() != Some(sender_id)
                || !matches!(user.get("bot"), None | Some(Value::Bool(false)))
                || !snowflake(chat_id)
                || !snowflake(message_id)
                || !snowflake(sender_id)
                || sender_id == bot_platform_id
            {
                return unknown;
            }
            if raw["channel"]["id"].as_str() != Some(chat_id) {
                return verified(sender_id, false);
            }
            let mut facts = discord_audience(sender_id, raw["channel"]["type"].as_u64());
            if facts.audience == AnswerAudience::Private
                && (!raw["guild_id"].is_null()
                    || !raw["member"].is_null()
                    || !raw["channel"]["guild_id"].is_null())
            {
                return unknown;
            }
            let guild = raw["guild_id"].as_str().is_some_and(snowflake)
                && raw["member"]["user"]["id"].as_str() == Some(sender_id)
                && raw["user"].is_null()
                && (raw["channel"]["guild_id"].is_null()
                    || raw["channel"]["guild_id"] == raw["guild_id"]);
            facts.shared_room_verified =
                discord_shared_room(raw["channel"]["type"].as_u64(), &raw["guild_id"])
                    && (guild
                        || (raw["guild_id"].is_null()
                            && raw["member"].is_null()
                            && raw["channel"]["guild_id"].is_null()));
            facts
        }
        "slack" => {
            let event = &raw["event"];
            if raw["type"] != "event_callback"
                || !matches!(event["type"].as_str(), Some("message" | "app_mention"))
                || event["channel"].as_str() != Some(chat_id)
                || event["ts"].as_str() != Some(message_id)
                || event["user"].as_str() != Some(sender_id)
                || !(provider_id(sender_id, "U") || provider_id(sender_id, "W"))
                || sender_id == bot_platform_id
                || !event["bot_id"].is_null()
                || matches!(
                    event["subtype"].as_str(),
                    Some("bot_message" | "message_changed")
                )
            {
                return unknown;
            }
            let mut facts = verified(
                sender_id,
                event["channel_type"] == "im" && provider_id(chat_id, "D"),
            );
            facts.shared_room_verified = match event["channel_type"].as_str() {
                Some("channel") => provider_id(chat_id, "C"),
                Some("group" | "mpim") => provider_id(chat_id, "G"),
                _ => false,
            };
            facts
        }
        "whatsapp" => {
            // The adapter strips the envelope only after verifying the Meta
            // signature and matching the receiving account and phone number.
            if raw["id"].as_str() != Some(message_id)
                || raw["from"].as_str() != Some(sender_id)
                || sender_id.is_empty()
                || sender_id.len() > 32
                || !sender_id.bytes().all(|b| b.is_ascii_digit())
            {
                return unknown;
            }
            verified(
                sender_id,
                chat_id == sender_id
                    && raw["group_id"].is_null()
                    && (raw["recipient_type"].is_null() || raw["recipient_type"] == "individual"),
            )
        }
        "x" => {
            // Polling is authenticated to the receiving account. The X
            // webhook also authenticates the account/tag and DM recipient
            // before producing this normalized provider event.
            use crate::services::discord_gateway::valid_message_id as account_id;
            if !account_id(sender_id)
                || !account_id(bot_platform_id)
                || sender_id == bot_platform_id
                || raw["sender_id"].as_str() != Some(sender_id)
                || raw["id"].as_str() != Some(message_id)
                || !account_id(message_id)
                || raw["dm_conversation_id"].as_str() != Some(chat_id)
            {
                return unknown;
            }
            if let Some((left, right)) = chat_id.split_once('-') {
                if (left == sender_id && right == bot_platform_id)
                    || (right == sender_id && left == bot_platform_id)
                {
                    return verified(sender_id, true);
                }
                return unknown;
            }
            if account_id(chat_id) {
                verified(sender_id, false)
            } else {
                unknown
            }
        }
        _ => unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        channel_adapters::resolve_adapter, provider_token_exchange_service::TokenExchangeCache,
    };
    use serde_json::json;
    use std::sync::Arc;

    fn discord_bot() -> ChannelBot {
        bson::from_document(bson::doc! {
            "_id":"bot", "user_id":"owner", "platform":"discord", "label":"Fixture",
            "platform_bot_id":"999", "platform_bot_username":"fixture", "status":"active",
            "is_active":true, "webhook_registered":true, "webhook_secret_hash":"fixture",
            "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).unwrap()
    }

    async fn parsed(platform: &str, raw: &Value) -> InboundMessage {
        resolve_adapter(platform, &Arc::new(TokenExchangeCache::new()))
            .unwrap()
            .parse_inbound(&serde_json::to_vec(raw).unwrap())
            .await
            .unwrap()
            .remove(0)
    }

    fn facts<'a>(platform: &str, bot_id: &str, message: &'a InboundMessage) -> TransportFacts<'a> {
        authenticated_facts(
            platform,
            bot_id,
            &message.raw_data,
            &message.conversation_id,
            &message.platform_message_id,
            &message.sender_platform_id,
            None,
        )
    }

    #[tokio::test]
    async fn channel_guest_transport_telegram_requires_exact_native_user_and_private_chat() {
        let mut raw = json!({"update_id":1,"message":{
            "message_id":20,"chat":{"id":123,"type":"private"},
            "from":{"id":123,"is_bot":false},"text":"hello"
        }});
        let direct = parsed("telegram", &raw).await;
        assert_eq!(facts("telegram", "999", &direct).sender_id, Some("123"));
        assert_eq!(
            facts("telegram", "999", &direct).audience,
            AnswerAudience::Private
        );

        raw["message"]["chat"] = json!({"id":-321,"type":"supergroup"});
        let mut group = parsed("telegram", &raw).await;
        assert_eq!(facts("telegram", "999", &group).sender_id, Some("123"));
        assert!(facts("telegram", "999", &group).shared_room_verified);
        assert_eq!(
            facts("telegram", "999", &group).audience,
            AnswerAudience::Shared
        );
        group.sender_platform_id = "333".into();
        assert_eq!(facts("telegram", "999", &group).sender_id, None);

        raw["message"]["sender_chat"] = json!({"id":-321});
        let anonymous = parsed("telegram", &raw).await;
        assert_eq!(facts("telegram", "999", &anonymous).sender_id, None);
    }

    #[tokio::test]
    async fn channel_guest_transport_discord_one_to_one_group_dm_and_forged_author() {
        let mut raw = json!({
            "type": 2, "id": "123", "application_id": "999", "channel_id": "321",
            "channel": {"id": "321", "type": 1},
            "user": {"id": "666", "bot":false},
            "data": {"name": "hello"}
        });
        let direct = parsed("discord", &raw).await;
        assert_eq!(facts("discord", "999", &direct).sender_id, Some("666"));
        assert_eq!(
            facts("discord", "999", &direct).audience,
            AnswerAudience::Private
        );
        raw["channel"]["type"] = json!(3);
        let mut group = parsed("discord", &raw).await;
        assert_eq!(
            facts("discord", "999", &group).audience,
            AnswerAudience::Shared
        );
        assert_eq!(facts("discord", "999", &group).sender_id, Some("666"));
        group.sender_platform_id = "667".into();
        assert_eq!(facts("discord", "999", &group).sender_id, None);
        assert_eq!(facts("discord", "555", &direct).sender_id, None);
    }

    #[test]
    fn discord_shared_room_requires_positive_consistent_channel_evidence() {
        let base = json!({"type":2,"id":"123","application_id":"999","channel_id":"321",
            "user":{"id":"666","bot":false},"channel":{"id":"321","type":3}});
        let check =
            |raw: &Value| authenticated_facts("discord", "999", raw, "321", "123", "666", None);
        assert!(check(&base).shared_room_verified);
        for channel in [
            Value::Null,
            json!({"id":"321"}),
            json!({"id":"321","type":99}),
            json!({"id":"322","type":3}),
            json!({"id":"321","type":0}),
            json!({"id":"321","type":1}),
        ] {
            let mut raw = base.clone();
            raw["channel"] = channel;
            assert!(!check(&raw).shared_room_verified, "{raw}");
        }
        let mut guild = base.clone();
        guild["guild_id"] = json!("444");
        guild["member"] = json!({"user":{"id":"666","bot":false}});
        guild.as_object_mut().unwrap().remove("user");
        guild["channel"] = json!({"id":"321","type":11,"guild_id":"444"});
        assert!(check(&guild).shared_room_verified);
        for channel in [
            json!({"id":"321","type":1}),
            json!({"id":"321","type":3}),
            json!({"id":"321","type":11,"guild_id":"445"}),
        ] {
            let mut raw = guild.clone();
            raw["channel"] = channel;
            assert!(!check(&raw).shared_room_verified, "{raw}");
        }
        guild["member"]["user"]["id"] = json!("667");
        assert!(!check(&guild).shared_room_verified);
    }

    #[test]
    fn discord_socket_guest_proof_rejects_conflicting_normalized_metadata() {
        let bot = discord_bot();
        for kind in [1, 3, 0, 11] {
            let mut raw = json!({"id":"123","channel_id":"321","type":0,"author":{"id":"666","bot":false},
                "content":"hello","mentions":[],"mention_roles":[],"mention_everyone":false});
            let mut channel = json!({"id":"321","type":kind,"parent_id":"322"});
            if !matches!(kind, 1 | 3) {
                raw["guild_id"] = json!("444");
                channel["guild_id"] = json!("444");
            }
            let (inbound, verified) = crate::services::discord_gateway::normalized_test_message(
                &bot,
                &json!({"d":raw}),
                &channel,
            );
            let check = |message: &InboundMessage| {
                let result = inbound_facts(&bot, message, Some(&verified));
                (
                    result.sender_id.is_some(),
                    result.audience,
                    result.shared_room_verified,
                )
            };
            assert_eq!(
                check(&inbound),
                (
                    true,
                    if kind == 1 {
                        AnswerAudience::Private
                    } else {
                        AnswerAudience::Shared
                    },
                    kind != 1
                )
            );
            // Keep the real verifier's facts while altering their raw evidence.
            // Neither private reads nor shared writes can borrow that proof.
            for guild in [json!("445"), json!(false), json!({"id":"444"})] {
                let mut bad = inbound.clone();
                bad.raw_data["channel"]["guild_id"] = guild.clone();
                assert_eq!(check(&bad), (false, AnswerAudience::Shared, false));
                bad = inbound.clone();
                bad.raw_data["guild_id"] = guild.clone();
                assert_eq!(check(&bad), (false, AnswerAudience::Shared, false));
                if matches!(kind, 1 | 3) {
                    bad.raw_data["channel"]["guild_id"] = guild;
                    assert_eq!(check(&bad), (false, AnswerAudience::Shared, false));
                }
            }
            for field in ["id", "type"] {
                let mut bad = inbound.clone();
                bad.raw_data["channel"][field] = if field == "id" {
                    json!("322")
                } else {
                    json!(99)
                };
                assert_eq!(check(&bad), (false, AnswerAudience::Shared, false));
            }
        }
    }

    #[test]
    fn discord_private_interaction_requires_consistent_embedded_room() {
        let base = json!({"type":2,"id":"123","application_id":"999","channel_id":"321",
            "user":{"id":"666","bot":false},"channel":{"id":"321","type":1}});
        let check = |raw: &Value| {
            let result = authenticated_facts("discord", "999", raw, "321", "123", "666", None);
            (
                result.sender_id.is_some(),
                result.audience,
                result.shared_room_verified,
            )
        };
        assert_eq!(check(&base), (true, AnswerAudience::Private, false));
        let mut explicit_null = base.clone();
        explicit_null["channel"]["guild_id"] = Value::Null;
        assert_eq!(
            check(&explicit_null),
            (true, AnswerAudience::Private, false)
        );
        for guild in [json!("444"), json!(false), json!({"id":"444"})] {
            let mut raw = base.clone();
            raw["channel"]["guild_id"] = guild;
            assert_eq!(check(&raw), (false, AnswerAudience::Shared, false));
        }
        let mut group = base.clone();
        group["channel"]["type"] = json!(3);
        assert_eq!(check(&group), (true, AnswerAudience::Shared, true));
    }

    #[test]
    fn shared_room_evidence_never_comes_from_the_private_fallback() {
        assert!(!verified("sender", false).shared_room_verified);
        for (platform, bot, chat, message, sender, raw, positive) in [
            (
                "telegram",
                "999",
                "-321",
                "20",
                "123",
                json!({"message":{"message_id":20,"chat":{"id":-321,"type":"supergroup"},"from":{"id":123,"is_bot":false}}}),
                true,
            ),
            (
                "telegram",
                "999",
                "-321",
                "20",
                "123",
                json!({"message":{"message_id":20,"chat":{"id":-321},"from":{"id":123,"is_bot":false}}}),
                false,
            ),
            (
                "telegram",
                "999",
                "123",
                "20",
                "123",
                json!({"message":{"message_id":20,"chat":{"id":123,"type":"group"},"from":{"id":123,"is_bot":false}}}),
                false,
            ),
            (
                "slack",
                "Ubot",
                "G123",
                "1.2",
                "Uguest",
                json!({"type":"event_callback","event":{"type":"message","channel":"G123","channel_type":"mpim","ts":"1.2","user":"Uguest"}}),
                true,
            ),
            (
                "slack",
                "Ubot",
                "D123",
                "1.2",
                "Uguest",
                json!({"type":"event_callback","event":{"type":"message","channel":"D123","channel_type":"group","ts":"1.2","user":"Uguest"}}),
                false,
            ),
            (
                "lark",
                "cli_bot",
                "oc_chat",
                "om_message",
                "ou_person",
                json!({"header":{"event_type":"im.message.receive_v1","app_id":"cli_bot"},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_person"}},"message":{"message_id":"om_message","chat_id":"oc_chat","chat_type":"unknown"}}}),
                false,
            ),
            (
                "whatsapp",
                "999",
                "group",
                "message",
                "111",
                json!({"from":"111","id":"message","group_id":"group"}),
                false,
            ),
        ] {
            assert_eq!(
                authenticated_facts(platform, bot, &raw, chat, message, sender, None)
                    .shared_room_verified,
                positive,
                "{platform} {raw}"
            );
        }
        for guild in [json!("invalid"), json!(false), json!(42)] {
            assert!(!discord_shared_room(Some(3), &guild));
            assert!(!discord_shared_room(Some(11), &guild));
        }
    }

    #[tokio::test]
    async fn persisted_shared_audience_alone_never_proves_a_shared_room() {
        let mut raw = json!({"type":2,"id":"123","application_id":"999","channel_id":"321",
            "user":{"id":"666","bot":false},"data":{"name":"hello"}});
        let inbound = parsed("discord", &raw).await;
        let evidence = facts("discord", "999", &inbound);
        assert_eq!(evidence.sender_id, Some("666"));
        assert!(!evidence.shared_room_verified);
        let bot = discord_bot();
        let mut source: ChannelMessage = bson::from_document(bson::doc! {
            "_id":"source", "channel_bot_id":&bot.id, "user_id":&bot.user_id,
            "direction":"inbound", "platform":"discord", "conversation_id":"route",
            "platform_conversation_id":"321", "sender_platform_id":"666", "content_type":"text",
            "guest_transport_audience":"shared", "created_at":bson::DateTime::now()
        })
        .unwrap();
        let mut document = bson::to_document(&source).unwrap();
        document.remove("guest_transport_shared_room_verified");
        let legacy: ChannelMessage = bson::from_document(document).unwrap();
        assert!(!stored_facts(&bot, &legacy, "321", "666", None).shared_room_verified);
        raw["channel"] = json!({"id":"321","type":3});
        source.guest_transport_shared_room_verified =
            facts("discord", "999", &parsed("discord", &raw).await).shared_room_verified;
        assert!(stored_facts(&bot, &source, "321", "666", None).shared_room_verified);
        assert!(!stored_facts(&bot, &source, "322", "666", None).shared_room_verified);
        assert!(!stored_facts(&bot, &source, "321", "667", None).shared_room_verified);
        assert!(!stored_facts(&bot, &source, "321", "666", Some("thread")).shared_room_verified);
    }

    #[tokio::test]
    async fn channel_guest_transport_lark_and_feishu_bind_app_and_open_id() {
        let mut raw = json!({
            "header":{"event_type":"im.message.receive_v1","event_id":"event-1","app_id":"cli_bot"},
            "event":{
                "sender":{"sender_type":"user","sender_id":{"open_id":"ou_person"}},
                "message":{"message_id":"om_message","chat_id":"oc_conversation",
                    "chat_type":"p2p","message_type":"text","content":"{\"text\":\"hello\"}"}
            }
        });
        for platform in ["lark", "feishu"] {
            let direct = parsed(platform, &raw).await;
            assert_eq!(
                facts(platform, "cli_bot", &direct).sender_id,
                Some("ou_person")
            );
            assert_eq!(
                facts(platform, "cli_bot", &direct).audience,
                AnswerAudience::Private
            );
            assert_eq!(facts(platform, "cli_other", &direct).sender_id, None);
        }
        raw["event"]["message"]["chat_type"] = json!("group");
        let group = parsed("feishu", &raw).await;
        assert!(facts("feishu", "cli_bot", &group).shared_room_verified);
        assert_eq!(
            facts("feishu", "cli_bot", &group).audience,
            AnswerAudience::Shared
        );
        assert_eq!(
            facts("feishu", "cli_bot", &group).sender_id,
            Some("ou_person")
        );
    }

    #[test]
    fn channel_guest_transport_destination_binds_chat_and_thread() {
        assert_ne!(destination("abc", Some("d")), destination("ab", Some("cd")));
        assert_ne!(destination("abc", None), destination("abc", Some("")));
        assert_ne!(destination("abc", Some("d")), destination("abc", Some("e")));
    }

    #[test]
    fn channel_guest_transport_other_adapters_require_positive_provider_evidence() {
        let check =
            |platform: &str, bot: &str, chat: &str, message: &str, sender: &str, raw: Value| {
                let result = authenticated_facts(platform, bot, &raw, chat, message, sender, None);
                (result.sender_id.is_some(), result.audience)
            };
        assert_eq!(
            check(
                "slack",
                "Ubot",
                "D123",
                "1.2",
                "Uguest",
                json!({
                    "type":"event_callback","event":{"type":"message","channel":"D123",
                        "channel_type":"im","ts":"1.2","user":"Uguest"}
                })
            ),
            (true, AnswerAudience::Private),
        );
        assert_eq!(
            check(
                "slack",
                "Ubot",
                "G123",
                "1.2",
                "Uguest",
                json!({
                    "type":"event_callback","event":{"type":"message","channel":"G123",
                        "channel_type":"mpim","ts":"1.2","user":"Uguest"}
                })
            ),
            (true, AnswerAudience::Shared),
        );
        assert_eq!(
            check(
                "whatsapp",
                "999",
                "111",
                "wamid.fake",
                "111",
                json!({
                    "from":"111", "id":"wamid.fake", "recipient_type":"individual"
                })
            ),
            (true, AnswerAudience::Private),
        );
        assert_eq!(
            check(
                "x",
                "999",
                "111-999",
                "123",
                "111",
                json!({
                    "sender_id":"111","id":"123","dm_conversation_id":"111-999"
                })
            ),
            (true, AnswerAudience::Private),
        );
        for platform in ["aurinko", "openclaw", "device", "custom"] {
            assert_eq!(
                check(
                    platform,
                    "999",
                    "111",
                    "123",
                    "111",
                    json!({
                        "sender_id":"111","conversation_type":"private","actor_kind":"human"
                    })
                ),
                (false, AnswerAudience::Shared),
            );
        }
    }
}
