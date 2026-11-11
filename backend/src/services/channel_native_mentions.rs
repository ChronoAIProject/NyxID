//! Bounded native addressing facts shared by legacy and followed channel chats.
//! Display names never identify a target or grant execution authority.
use crate::models::channel_bot::ChannelBot;
use serde_json::Value;

pub const MAX_ENTITIES: usize = 128;
const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Default)]
pub struct Mentions {
    pub invalid: bool,
    pub bot: bool,
    pub other: bool,
    pub everyone: bool,
}

fn span(entity: &Value, units: &[u16]) -> Option<(usize, usize)> {
    let start = usize::try_from(entity["offset"].as_u64()?).ok()?;
    let len = usize::try_from(entity["length"].as_u64()?).ok()?;
    let end = start.checked_add(len)?;
    if len == 0 || start >= units.len() || end > units.len() {
        return None;
    }
    // Both boundaries must be Unicode scalar boundaries, not half a surrogate.
    if (start > 0 && (0xDC00..=0xDFFF).contains(&units[start]))
        || (end < units.len() && (0xDC00..=0xDFFF).contains(&units[end]))
    {
        return None;
    }
    Some((start, end))
}

pub fn telegram(message: &Value, bot: &ChannelBot) -> Mentions {
    let mut result = Mentions::default();
    // Pair entities with their own content. A text field must not borrow caption offsets.
    let (text, entities) = if let Some(text) = message["text"].as_str() {
        (text, message["entities"].as_array())
    } else if let Some(caption) = message["caption"].as_str() {
        (caption, message["caption_entities"].as_array())
    } else {
        return result;
    };
    let field = if message["text"].is_string() {
        "entities"
    } else {
        "caption_entities"
    };
    if message
        .get(field)
        .is_some_and(|v| !v.is_null() && !v.is_array())
    {
        return Mentions {
            invalid: true,
            ..Default::default()
        };
    }
    let Some(entities) = entities else {
        return result;
    };
    if entities.len() > MAX_ENTITIES {
        return Mentions {
            invalid: true,
            ..Default::default()
        };
    }
    if text.len() > MAX_TEXT_BYTES {
        return Mentions {
            invalid: true,
            ..Default::default()
        };
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut addressed = Vec::new();
    let mut excluded = Vec::new();
    for entity in entities {
        let kind = entity["type"].as_str().unwrap_or_default();
        if matches!(kind, "mention" | "text_mention" | "bot_command") {
            let Some((start, end)) = span(entity, &units).filter(|(a, b)| b - a <= 256) else {
                return Mentions {
                    invalid: true,
                    ..Default::default()
                };
            };
            addressed.push((start, end, entity));
        } else if matches!(
            kind,
            "code" | "pre" | "blockquote" | "expandable_blockquote" | "text_link" | "url" | "email"
        ) {
            let Some(range) = span(entity, &units) else {
                return Mentions {
                    invalid: true,
                    ..Default::default()
                };
            };
            excluded.push(range);
        }
    }
    addressed.sort_by_key(|(start, end, _)| (*start, *end));
    if addressed.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return Mentions {
            invalid: true,
            ..Default::default()
        };
    }
    for (start, end, entity) in addressed {
        if excluded.iter().any(|(a, b)| start < *b && *a < end) {
            continue;
        }
        let Ok(value) = String::from_utf16(&units[start..end]) else {
            return Mentions {
                invalid: true,
                ..Default::default()
            };
        };
        if entity["type"] != "text_mention" {
            // Native ranges cannot select a username prefix/suffix inside a
            // larger token, email or quoted identifier.
            let joined = |unit: u16| {
                unit < 128 && (unit as u8).is_ascii_alphanumeric()
                    || unit == b'_' as u16
                    || unit == b'@' as u16
            };
            if (start > 0 && joined(units[start - 1])) || (end < units.len() && joined(units[end]))
            {
                continue;
            }
        }
        if entity["type"] == "text_mention" {
            if let Some(id) = entity["user"]["id"].as_i64().filter(|id| *id > 0) {
                if id.to_string() == bot.platform_bot_id {
                    result.bot = true;
                } else {
                    result.other = true;
                }
            }
        } else {
            let username = if entity["type"] == "bot_command" {
                let Some((command, username)) =
                    value.strip_prefix('/').and_then(|v| v.split_once('@'))
                else {
                    continue;
                };
                if command.is_empty()
                    || command.len() > 32
                    || !command
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                {
                    continue;
                }
                username
            } else {
                let Some(username) = value.strip_prefix('@') else {
                    continue;
                };
                username
            };
            if !(5..=32).contains(&username.len())
                || !username
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            {
                continue;
            }
            if !bot.platform_bot_username.is_empty()
                && username.eq_ignore_ascii_case(bot.platform_bot_username.trim_start_matches('@'))
            {
                result.bot = true;
            } else {
                result.other = true;
            }
        }
    }
    result
}

pub fn lark(message: &Value, own_id: Option<&str>) -> Mentions {
    let mut result = Mentions::default();
    if message
        .get("mentions")
        .is_some_and(|v| !v.is_null() && !v.is_array())
    {
        return Mentions {
            invalid: true,
            ..Default::default()
        };
    }
    let Some(mentions) = message["mentions"].as_array() else {
        return result;
    };
    if mentions.len() > MAX_ENTITIES {
        return Mentions {
            invalid: true,
            ..Default::default()
        };
    }
    let mut keys = std::collections::HashMap::new();
    for mention in mentions {
        let key = mention["key"].as_str().unwrap_or_default();
        let id = mention["id"]["open_id"].as_str().unwrap_or_default();
        if key.len() > 64 || id.len() > 256 {
            return Mentions {
                invalid: true,
                ..Default::default()
            };
        }
        if matches!(key, "@_all" | "_all") || id == "all" {
            result.everyone = true;
            continue;
        }
        if !id.starts_with("ou_") {
            continue;
        }
        if keys.insert(key, id).is_some_and(|previous| previous != id) {
            return Mentions {
                invalid: true,
                ..Default::default()
            };
        }
        if own_id
            .filter(|own| own.starts_with("ou_"))
            .is_some_and(|own| own == id)
        {
            result.bot = true;
        } else {
            result.other = true;
        }
    }
    result
}

#[cfg(test)]
mod directed_command_tests {
    use super::*;
    use serde_json::json;
    fn bot() -> ChannelBot {
        bson::from_document(bson::doc! {
            "_id":"bot", "user_id":"owner", "platform":"telegram", "label":"Support",
            "platform_bot_id":"123", "platform_bot_username":"Helper_bot",
            "webhook_registered":true,"webhook_secret_hash":"hash","status":"active","is_active":true,
            "bot_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).unwrap()
    }
    #[test]
    fn standard_start_command_arguments_wrong_target_and_unicode_entities() {
        // Entity parsing only: Telegram privacy/delivery is not simulated.
        for (text, offset, target) in [
            ("/start@Helper_bot", 0, true),
            ("/start@Helper_bot invite-code", 0, true),
            ("😀 /start@Helper_bot 参数", 3, true),
            ("/start@Wrong_bot invite-code", 0, false),
        ] {
            let token = text
                .split_whitespace()
                .find(|s| s.starts_with('/'))
                .unwrap();
            let facts = telegram(
                &json!({"text":text,"entities":[{"type":"bot_command","offset":offset,"length":token.encode_utf16().count()}]}),
                &bot(),
            );
            assert!(!facts.invalid, "{text}");
            assert_eq!(facts.bot, target, "{text}");
            assert_eq!(facts.other, !target);
        }
        let spaced = telegram(
            &json!({"text":"/start @Helper_bot","entities":[
                {"type":"bot_command","offset":0,"length":6},
                {"type":"mention","offset":7,"length":11}
            ]}),
            &bot(),
        );
        assert!(spaced.bot && !spaced.other && !spaced.invalid);
        let invalid = telegram(
            &json!({"text":"😀 /start@Helper_bot", "entities":[{"type":"bot_command","offset":1,"length":17}]}),
            &bot(),
        );
        assert!(invalid.invalid);
        let plain = telegram(
            &json!({"text":"/start","entities":[{"type":"bot_command","offset":0,"length":6}]}),
            &bot(),
        );
        assert!(!plain.bot && !plain.other && !plain.invalid);
    }
}
