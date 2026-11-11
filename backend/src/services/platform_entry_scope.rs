//! One identity projection for admission, stored proofs, cleanup and delivery.
//! Native IDs remain opaque and every record retains its identity version.
use crate::{
    errors::AppResult,
    models::platform_entry_projection::{
        StoredEntryIdentity as EntryIdentity, StoredEventIdentity as EventIdentity,
        StoredSenderIdentity as SenderIdentity,
    },
};
use bson::{Bson, Document, doc};

pub(crate) fn platform(entry: &EntryIdentity) -> &'static str {
    use crate::models::platform_entry_identity::EntryPlatform;
    match entry {
        EntryIdentity::TelegramV1 { .. } => "telegram",
        EntryIdentity::NativeV2 { platform_entry } => match platform_entry.platform {
            EntryPlatform::Slack => "slack",
            EntryPlatform::Discord => "discord",
            EntryPlatform::Whatsapp => "whatsapp",
            EntryPlatform::Lark => "lark",
            EntryPlatform::Feishu => "feishu",
            EntryPlatform::X => "x",
        },
    }
}

pub(crate) fn version(entry: &EntryIdentity) -> u32 {
    match entry {
        EntryIdentity::TelegramV1 { .. } => 1,
        EntryIdentity::NativeV2 { .. } => 2,
    }
}

pub(crate) fn validate(
    entry: &EntryIdentity,
    manager: &str,
    sender: Option<&SenderIdentity>,
) -> AppResult<()> {
    use super::platform_entry_identity::{opaque, refused, validate_entry};
    match entry {
        EntryIdentity::TelegramV1 { telegram_bot_id } => {
            if *telegram_bot_id <= 0 || manager != telegram_bot_id.to_string() {
                return Err(refused());
            }
        }
        EntryIdentity::NativeV2 { platform_entry } => {
            validate_entry(platform_entry)?;
            if manager != platform_entry.platform_bot_id {
                return Err(refused());
            }
        }
    }
    match (entry, sender) {
        (_, None) => {}
        (
            EntryIdentity::TelegramV1 { .. },
            Some(SenderIdentity::TelegramV1 {
                telegram_sender_id,
                telegram_chat_id,
            }),
        ) if *telegram_sender_id > 0 && telegram_sender_id == telegram_chat_id => {}
        (EntryIdentity::NativeV2 { .. }, Some(SenderIdentity::NativeV2 { platform_sender })) => {
            opaque(&platform_sender.sender_id)?;
            opaque(&platform_sender.private_conversation_id)?;
        }
        _ => return Err(refused()),
    }
    Ok(())
}

pub(crate) fn entry_fields(entry: &EntryIdentity) -> Document {
    match entry {
        EntryIdentity::TelegramV1 { telegram_bot_id } => doc! {"telegram_bot_id":telegram_bot_id},
        EntryIdentity::NativeV2 { platform_entry } => {
            doc! {"platform_entry":bson::to_bson(platform_entry).expect("plain identity")}
        }
    }
}

pub(crate) fn entry_key(entry: &EntryIdentity) -> &'static str {
    match entry {
        EntryIdentity::TelegramV1 { .. } => "telegram_bot_id",
        EntryIdentity::NativeV2 { .. } => "platform_entry",
    }
}
pub(crate) fn entry_value(entry: &EntryIdentity) -> Bson {
    entry_fields(entry)
        .remove(entry_key(entry))
        .expect("entry projection")
}
pub(crate) fn actor_key(sender: &SenderIdentity, ingress: bool) -> &'static str {
    match sender {
        SenderIdentity::TelegramV1 { .. } if ingress => "sender_id",
        SenderIdentity::TelegramV1 { .. } => "telegram_sender_id",
        SenderIdentity::NativeV2 { .. } => "platform_sender.sender_id",
    }
}
pub(crate) fn destination_key(sender: &SenderIdentity, ingress: bool) -> &'static str {
    match sender {
        SenderIdentity::TelegramV1 { .. } if ingress => "chat_id",
        SenderIdentity::TelegramV1 { .. } => "telegram_chat_id",
        SenderIdentity::NativeV2 { .. } => "platform_sender.private_conversation_id",
    }
}
pub(crate) fn actor_value(sender: &SenderIdentity) -> Bson {
    match sender {
        SenderIdentity::TelegramV1 {
            telegram_sender_id, ..
        } => Bson::Int64(*telegram_sender_id),
        SenderIdentity::NativeV2 { platform_sender } => {
            Bson::String(platform_sender.sender_id.clone())
        }
    }
}

pub(crate) fn actor_string(sender: &SenderIdentity) -> String {
    match sender {
        SenderIdentity::TelegramV1 {
            telegram_sender_id, ..
        } => telegram_sender_id.to_string(),
        SenderIdentity::NativeV2 { platform_sender } => platform_sender.sender_id.clone(),
    }
}
pub(crate) fn destination_value(sender: &SenderIdentity) -> Bson {
    match sender {
        SenderIdentity::TelegramV1 {
            telegram_chat_id, ..
        } => Bson::Int64(*telegram_chat_id),
        SenderIdentity::NativeV2 { platform_sender } => {
            Bson::String(platform_sender.private_conversation_id.clone())
        }
    }
}

pub(crate) fn sender_fields(sender: &SenderIdentity, ingress: bool) -> Document {
    match sender {
        SenderIdentity::TelegramV1 {
            telegram_sender_id,
            telegram_chat_id,
        } => {
            if ingress {
                doc! {"sender_id":telegram_sender_id,"chat_id":telegram_chat_id}
            } else {
                doc! {"telegram_sender_id":telegram_sender_id,"telegram_chat_id":telegram_chat_id}
            }
        }
        SenderIdentity::NativeV2 { platform_sender } => {
            doc! {"platform_sender":bson::to_bson(platform_sender).expect("plain identity")}
        }
    }
}

/// The account-generation lookup is sender scoped. A resolved row is still
/// compared against the complete private destination before it can act.
pub(crate) fn sender_key_fields(sender: &SenderIdentity, ingress: bool) -> Document {
    match sender {
        SenderIdentity::TelegramV1 {
            telegram_sender_id, ..
        } => {
            doc! {if ingress {"sender_id"} else {"telegram_sender_id"}:telegram_sender_id}
        }
        SenderIdentity::NativeV2 { platform_sender } => {
            doc! {"platform_sender.sender_id":&platform_sender.sender_id}
        }
    }
}

pub(crate) fn event_fields(event: &EventIdentity) -> Document {
    match event {
        EventIdentity::TelegramV1 {
            update_id,
            message_id,
        } => doc! {"update_id":update_id,"message_id":message_id},
        EventIdentity::NativeV2 {
            platform_event_id,
            platform_message_id,
        } => doc! {"platform_event_id":platform_event_id,"platform_message_id":platform_message_id},
    }
}

pub(crate) fn prefix(fields: Document, prefix: &str) -> Document {
    fields
        .into_iter()
        .map(|(key, value)| (format!("{prefix}{key}"), value))
        .collect()
}

pub(crate) fn sender_parts(sender: &SenderIdentity) -> (String, String) {
    match sender {
        SenderIdentity::TelegramV1 {
            telegram_sender_id,
            telegram_chat_id,
        } => (telegram_sender_id.to_string(), telegram_chat_id.to_string()),
        SenderIdentity::NativeV2 { platform_sender } => (
            platform_sender.sender_id.clone(),
            platform_sender.private_conversation_id.clone(),
        ),
    }
}

pub(crate) fn budget_actor(entry: &EntryIdentity, sender: &SenderIdentity) -> AppResult<String> {
    match (entry, sender) {
        (
            EntryIdentity::TelegramV1 { .. },
            SenderIdentity::TelegramV1 {
                telegram_sender_id, ..
            },
        ) => Ok(telegram_sender_id.to_string()),
        (
            EntryIdentity::NativeV2 { platform_entry },
            SenderIdentity::NativeV2 { platform_sender },
        ) => Ok(format!(
            "v2:{}",
            super::mcp_service::canonical_sha256(serde_json::json!([
                platform_entry,
                platform_sender.sender_id
            ]))
        )),
        _ => Err(super::platform_entry_identity::refused()),
    }
}

pub(crate) fn budget_identity(
    entry: &EntryIdentity,
    sender: &SenderIdentity,
) -> AppResult<crate::models::platform_entry_projection::StoredBudgetIdentity> {
    use crate::models::platform_entry_projection::StoredBudgetIdentity;
    match (entry, sender) {
        (
            EntryIdentity::TelegramV1 { .. },
            SenderIdentity::TelegramV1 {
                telegram_sender_id, ..
            },
        ) => Ok(StoredBudgetIdentity::TelegramV1 {
            sender_id: *telegram_sender_id,
        }),
        (
            EntryIdentity::NativeV2 { platform_entry },
            SenderIdentity::NativeV2 { platform_sender },
        ) => Ok(StoredBudgetIdentity::NativeV2 {
            platform_entry: platform_entry.clone(),
            platform_sender_id: platform_sender.sender_id.clone(),
        }),
        _ => Err(super::platform_entry_identity::refused()),
    }
}

/// v1 keeps its original decimal-string HMAC parts. v2 binds the complete
/// platform/channel namespace in addition to the opaque actor and destination.
pub(crate) fn hmac_identity(entry: &EntryIdentity, sender: &SenderIdentity) -> Vec<String> {
    let (actor, destination) = sender_parts(sender);
    match entry {
        EntryIdentity::TelegramV1 { .. } => vec![actor, destination],
        EntryIdentity::NativeV2 { platform_entry } => vec![
            "native-entry:v2".into(),
            platform(entry).into(),
            platform_entry.channel_bot_id.clone(),
            platform_entry.platform_bot_id.clone(),
            actor,
            destination,
        ],
    }
}

pub(crate) fn exclude_other_version(filter: &mut Document, entry: &EntryIdentity) {
    match entry {
        EntryIdentity::TelegramV1 { .. } => {
            filter.insert("platform_entry", Bson::Null);
        }
        EntryIdentity::NativeV2 { .. } => {
            filter.insert("telegram_bot_id", doc! {"$exists":false});
        }
    }
}

#[cfg(test)]
pub(crate) fn legacy_bot_id(entry: &EntryIdentity) -> i64 {
    let EntryIdentity::TelegramV1 { telegram_bot_id } = entry else {
        panic!("legacy fixture entry required");
    };
    *telegram_bot_id
}

#[cfg(test)]
pub(crate) fn legacy_sender_id(sender: &SenderIdentity) -> i64 {
    let SenderIdentity::TelegramV1 {
        telegram_sender_id, ..
    } = sender
    else {
        panic!("legacy fixture sender required");
    };
    *telegram_sender_id
}

#[cfg(test)]
pub(crate) fn legacy_chat_id(sender: &SenderIdentity) -> i64 {
    let SenderIdentity::TelegramV1 {
        telegram_chat_id, ..
    } = sender
    else {
        panic!("legacy fixture sender required");
    };
    *telegram_chat_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_entry_identity::{
        EntryPlatform, NativeEntryIdentity, NativeSenderIdentity,
    };
    #[test]
    fn old_decimal_hmac_parts_and_native_namespaces_remain_distinct() {
        let legacy = EntryIdentity::TelegramV1 {
            telegram_bot_id: 99,
        };
        let sender = SenderIdentity::TelegramV1 {
            telegram_sender_id: 123,
            telegram_chat_id: 123,
        };
        assert_eq!(hmac_identity(&legacy, &sender), ["123", "123"]);
        assert_eq!(entry_fields(&legacy), doc! {"telegram_bot_id":99_i64});
        assert_eq!(
            sender_fields(&sender, true),
            doc! {"sender_id":123_i64,"chat_id":123_i64}
        );
        let mut entry = EntryIdentity::NativeV2 {
            platform_entry: NativeEntryIdentity {
                version: 2,
                platform: EntryPlatform::Slack,
                channel_bot_id: uuid::Uuid::new_v4().to_string(),
                platform_bot_id: "123".into(),
            },
        };
        let sender = SenderIdentity::NativeV2 {
            platform_sender: NativeSenderIdentity {
                sender_id: "123".into(),
                private_conversation_id: "123".into(),
            },
        };
        validate(&entry, "123", Some(&sender)).unwrap();
        let first = hmac_identity(&entry, &sender);
        if let EntryIdentity::NativeV2 { platform_entry } = &mut entry {
            platform_entry.platform = EntryPlatform::Discord;
        }
        assert_ne!(first, hmac_identity(&entry, &sender));
        if let EntryIdentity::NativeV2 { platform_entry } = &mut entry {
            platform_entry.channel_bot_id = uuid::Uuid::new_v4().to_string();
        }
        assert_ne!(first, hmac_identity(&entry, &sender));
        assert!(validate(&legacy, "99", Some(&sender)).is_err());
    }
}
