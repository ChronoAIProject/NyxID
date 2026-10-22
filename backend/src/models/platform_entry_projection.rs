//! Additive identity projections keep Telegram v1 field names and omit them
//! completely for native v2. Mixed and explicit-null identities fail decoding.
use super::platform_entry_identity::{NativeEntryIdentity, NativeSenderIdentity};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StoredEntryIdentity {
    TelegramV1 { telegram_bot_id: i64 },
    NativeV2 { platform_entry: NativeEntryIdentity },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StoredSenderIdentity {
    TelegramV1 {
        telegram_sender_id: i64,
        telegram_chat_id: i64,
    },
    NativeV2 {
        platform_sender: NativeSenderIdentity,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StoredEventIdentity {
    TelegramV1 {
        update_id: i64,
        message_id: i64,
    },
    NativeV2 {
        platform_event_id: String,
        platform_message_id: String,
    },
}

/// Budget rows historically retained only the numeric sender. Native rows
/// retain the complete entry namespace and opaque actor, without a fake integer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StoredBudgetIdentity {
    TelegramV1 {
        sender_id: i64,
    },
    NativeV2 {
        platform_entry: NativeEntryIdentity,
        platform_sender_id: String,
    },
}

impl<'de> Deserialize<'de> for StoredBudgetIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            sender_id: Field<i64>,
            #[serde(default)]
            platform_entry: Field<NativeEntryIdentity>,
            #[serde(default)]
            platform_sender_id: Field<String>,
        }
        match Raw::deserialize(deserializer)? {
            Raw {
                sender_id: Field::Present(sender_id),
                platform_entry: Field::Missing,
                platform_sender_id: Field::Missing,
            } if sender_id > 0 => Ok(Self::TelegramV1 { sender_id }),
            Raw {
                sender_id: Field::Missing,
                platform_entry: Field::Present(platform_entry),
                platform_sender_id: Field::Present(platform_sender_id),
            } if entry_matches(
                &StoredEntryIdentity::NativeV2 {
                    platform_entry: platform_entry.clone(),
                },
                &platform_entry.platform_bot_id,
            ) && opaque(&platform_sender_id) =>
            {
                Ok(Self::NativeV2 {
                    platform_entry,
                    platform_sender_id,
                })
            }
            _ => Err(mixed()),
        }
    }
}

fn opaque(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

pub(crate) fn entry_matches(entry: &StoredEntryIdentity, manager_bot_id: &str) -> bool {
    match entry {
        StoredEntryIdentity::TelegramV1 { telegram_bot_id } => {
            *telegram_bot_id > 0 && manager_bot_id == telegram_bot_id.to_string()
        }
        StoredEntryIdentity::NativeV2 { platform_entry } => {
            platform_entry.version == 2
                && uuid::Uuid::parse_str(&platform_entry.channel_bot_id)
                    .is_ok_and(|id| id.get_version_num() == 4)
                && opaque(&platform_entry.platform_bot_id)
                && manager_bot_id == platform_entry.platform_bot_id
        }
    }
}

pub(crate) fn entry_platform(entry: &StoredEntryIdentity) -> &'static str {
    use super::platform_entry_identity::EntryPlatform;
    match entry {
        StoredEntryIdentity::TelegramV1 { .. } => "telegram",
        StoredEntryIdentity::NativeV2 { platform_entry } => match platform_entry.platform {
            EntryPlatform::Slack => "slack",
            EntryPlatform::Discord => "discord",
            EntryPlatform::Whatsapp => "whatsapp",
            EntryPlatform::Lark => "lark",
            EntryPlatform::Feishu => "feishu",
            EntryPlatform::X => "x",
        },
    }
}

/// A record's identity version is one structural choice. Individually valid
/// projections cannot be combined into a different authority version.
pub(crate) fn same_version(
    entry: &StoredEntryIdentity,
    sender: &StoredSenderIdentity,
    event: Option<&StoredEventIdentity>,
) -> bool {
    let matching = match (entry, sender) {
        (
            StoredEntryIdentity::TelegramV1 { .. },
            StoredSenderIdentity::TelegramV1 {
                telegram_sender_id,
                telegram_chat_id,
            },
        ) => *telegram_sender_id > 0 && *telegram_chat_id != 0,
        (
            StoredEntryIdentity::NativeV2 { platform_entry },
            StoredSenderIdentity::NativeV2 { platform_sender },
        ) => {
            platform_entry.version == 2
                && opaque(&platform_sender.sender_id)
                && opaque(&platform_sender.private_conversation_id)
        }
        _ => false,
    };
    matching
        && match (entry, event) {
            (_, None) => true,
            (
                StoredEntryIdentity::TelegramV1 { .. },
                Some(StoredEventIdentity::TelegramV1 {
                    update_id,
                    message_id,
                }),
            ) => *update_id >= 0 && *message_id > 0,
            (
                StoredEntryIdentity::NativeV2 { .. },
                Some(StoredEventIdentity::NativeV2 {
                    platform_event_id,
                    platform_message_id,
                }),
            ) => opaque(platform_event_id) && opaque(platform_message_id),
            _ => false,
        }
}

/// Generate a plain stored model and its private serde input together so the
/// complete identity is checked during decode, before any service can use it.
/// Field order/serde attributes remain identical for v1 serialization.
macro_rules! checked_identity_record {
    (
        $(#[$attribute:meta])* $visibility:vis struct $name:ident, $raw:ident {
            $( $(#[$field_attribute:meta])* $field_visibility:vis $field:ident : $ty:ty ),* $(,)?
        }
        |$record:ident| $valid:expr
    ) => {
        $(#[$attribute])*
        $visibility struct $name {
            $( $(#[$field_attribute])* $field_visibility $field: $ty ),*
        }
        #[derive(serde::Deserialize)]
        struct $raw {
            $( $(#[$field_attribute])* $field: $ty ),*
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = <$raw as serde::Deserialize>::deserialize(deserializer)?;
                let $record = Self { $( $field: raw.$field ),* };
                if !$valid {
                    return Err(serde::de::Error::custom("platform identity versions must match"));
                }
                Ok($record)
            }
        }
    };
}
pub(crate) use checked_identity_record;

// Option<T> would collapse an explicit null into absence and accept a mixed
// projection. Only an absent field uses Missing; a present field must decode T.
#[derive(Default)]
pub(crate) enum Field<T> {
    #[default]
    Missing,
    Present(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::Present(T::deserialize(deserializer)?))
    }
}
fn mixed<E: serde::de::Error>() -> E {
    E::custom("exactly one complete platform identity is required")
}

impl<'de> Deserialize<'de> for StoredEntryIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            telegram_bot_id: Field<i64>,
            #[serde(default)]
            platform_entry: Field<NativeEntryIdentity>,
        }
        match Raw::deserialize(deserializer)? {
            Raw {
                telegram_bot_id: Field::Present(telegram_bot_id),
                platform_entry: Field::Missing,
            } => Ok(Self::TelegramV1 { telegram_bot_id }),
            Raw {
                telegram_bot_id: Field::Missing,
                platform_entry: Field::Present(platform_entry),
            } => Ok(Self::NativeV2 { platform_entry }),
            _ => Err(mixed()),
        }
    }
}
impl<'de> Deserialize<'de> for StoredSenderIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            telegram_sender_id: Field<i64>,
            #[serde(default)]
            telegram_chat_id: Field<i64>,
            #[serde(default)]
            platform_sender: Field<NativeSenderIdentity>,
        }
        match Raw::deserialize(deserializer)? {
            Raw {
                telegram_sender_id: Field::Present(telegram_sender_id),
                telegram_chat_id: Field::Present(telegram_chat_id),
                platform_sender: Field::Missing,
            } => Ok(Self::TelegramV1 {
                telegram_sender_id,
                telegram_chat_id,
            }),
            Raw {
                telegram_sender_id: Field::Missing,
                telegram_chat_id: Field::Missing,
                platform_sender: Field::Present(platform_sender),
            } => Ok(Self::NativeV2 { platform_sender }),
            _ => Err(mixed()),
        }
    }
}
impl<'de> Deserialize<'de> for StoredEventIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            update_id: Field<i64>,
            #[serde(default)]
            message_id: Field<i64>,
            #[serde(default)]
            platform_event_id: Field<String>,
            #[serde(default)]
            platform_message_id: Field<String>,
        }
        match Raw::deserialize(deserializer)? {
            Raw {
                update_id: Field::Present(update_id),
                message_id: Field::Present(message_id),
                platform_event_id: Field::Missing,
                platform_message_id: Field::Missing,
            } => Ok(Self::TelegramV1 {
                update_id,
                message_id,
            }),
            Raw {
                update_id: Field::Missing,
                message_id: Field::Missing,
                platform_event_id: Field::Present(platform_event_id),
                platform_message_id: Field::Present(platform_message_id),
            } => Ok(Self::NativeV2 {
                platform_event_id,
                platform_message_id,
            }),
            _ => Err(mixed()),
        }
    }
}

/// The original ingress/delivery rows used sender_id/chat_id rather than the
/// consent models' telegram_* names. This preserves their exact v1 projection.
pub mod ingress_sender {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        sender: &StoredSenderIdentity,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Telegram {
            sender_id: i64,
            chat_id: i64,
        }
        match sender {
            StoredSenderIdentity::TelegramV1 {
                telegram_sender_id,
                telegram_chat_id,
            } => Telegram {
                sender_id: *telegram_sender_id,
                chat_id: *telegram_chat_id,
            }
            .serialize(serializer),
            StoredSenderIdentity::NativeV2 { .. } => sender.serialize(serializer),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<StoredSenderIdentity, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            sender_id: Field<i64>,
            #[serde(default)]
            chat_id: Field<i64>,
            #[serde(default)]
            platform_sender: Field<NativeSenderIdentity>,
        }
        match Raw::deserialize(deserializer)? {
            Raw {
                sender_id: Field::Present(telegram_sender_id),
                chat_id: Field::Present(telegram_chat_id),
                platform_sender: Field::Missing,
            } => Ok(StoredSenderIdentity::TelegramV1 {
                telegram_sender_id,
                telegram_chat_id,
            }),
            Raw {
                sender_id: Field::Missing,
                chat_id: Field::Missing,
                platform_sender: Field::Present(platform_sender),
            } => Ok(StoredSenderIdentity::NativeV2 { platform_sender }),
            _ => Err(mixed()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    checked_identity_record! {
    #[derive(Serialize)]
    struct Projection, RawProjection {
        profile_id: String,
        #[serde(flatten)]
        entry: StoredEntryIdentity,
        manager_authority_epoch: i64,
        #[serde(flatten, with = "ingress_sender")]
        sender: StoredSenderIdentity,
        #[serde(flatten)]
        event: StoredEventIdentity,
    }
    |record| same_version(&record.entry, &record.sender, Some(&record.event))
    }
    #[derive(Serialize)]
    struct Original {
        profile_id: String,
        telegram_bot_id: i64,
        manager_authority_epoch: i64,
        sender_id: i64,
        chat_id: i64,
        update_id: i64,
        message_id: i64,
    }
    #[test]
    fn telegram_flattening_preserves_exact_json_and_bson_bytes() {
        let old = Original {
            profile_id: "legacy".into(),
            telegram_bot_id: 99,
            manager_authority_epoch: 3,
            sender_id: 123,
            chat_id: 123,
            update_id: 4,
            message_id: 5,
        };
        let new = Projection {
            profile_id: "legacy".into(),
            entry: StoredEntryIdentity::TelegramV1 {
                telegram_bot_id: 99,
            },
            manager_authority_epoch: 3,
            sender: StoredSenderIdentity::TelegramV1 {
                telegram_sender_id: 123,
                telegram_chat_id: 123,
            },
            event: StoredEventIdentity::TelegramV1 {
                update_id: 4,
                message_id: 5,
            },
        };
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
        assert_eq!(bson::to_vec(&old).unwrap(), bson::to_vec(&new).unwrap());
        let decoded: Projection = bson::from_slice(&bson::to_vec(&old).unwrap()).unwrap();
        assert_eq!(decoded.entry, new.entry);
        assert_eq!(decoded.sender, new.sender);
        assert_eq!(decoded.event, new.event);
    }
    #[test]
    fn native_projection_has_no_numeric_telegram_fields_and_rejects_mixed_or_null() {
        let native = json!({"profile_id":"native","platform_entry":{"version":2,"platform":"slack",
            "channel_bot_id":"8f053bdd-cffe-400c-a0fa-93953aa70f13","platform_bot_id":"B-opaque"},
            "manager_authority_epoch":3,"platform_sender":{"sender_id":"U-person","private_conversation_id":"D-chat"},
            "platform_event_id":"Ev-original","platform_message_id":"173.000001"});
        let parsed: Projection = serde_json::from_value(native.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), native);
        for field in [
            "telegram_bot_id",
            "sender_id",
            "chat_id",
            "update_id",
            "message_id",
        ] {
            for value in [json!(99), json!(null)] {
                let mut mixed = native.clone();
                mixed[field] = value;
                assert!(
                    serde_json::from_value::<Projection>(mixed).is_err(),
                    "{field}"
                );
            }
        }
        let mut partial = native.clone();
        partial
            .as_object_mut()
            .unwrap()
            .remove("platform_message_id");
        assert!(serde_json::from_value::<Projection>(partial).is_err());
    }
    #[test]
    fn complete_individually_valid_projections_cannot_mix_record_versions() {
        let telegram = json!({"profile_id":"legacy","telegram_bot_id":99,
            "manager_authority_epoch":3,"sender_id":123,"chat_id":123,
            "update_id":4,"message_id":5});
        let native = json!({"profile_id":"native","platform_entry":{"version":2,"platform":"slack",
            "channel_bot_id":"8f053bdd-cffe-400c-a0fa-93953aa70f13","platform_bot_id":"B-opaque"},
            "manager_authority_epoch":3,"platform_sender":{"sender_id":"U-person","private_conversation_id":"D-chat"},
            "platform_event_id":"Ev-original","platform_message_id":"173.000001"});
        for bits in 1_u8..7 {
            let mut combined = json!({"profile_id":"mixed","manager_authority_epoch":3});
            for (bit, old_fields, new_fields) in [
                (1, vec!["telegram_bot_id"], vec!["platform_entry"]),
                (2, vec!["sender_id", "chat_id"], vec!["platform_sender"]),
                (
                    4,
                    vec!["update_id", "message_id"],
                    vec!["platform_event_id", "platform_message_id"],
                ),
            ] {
                let (source, fields) = if bits & bit == 0 {
                    (&telegram, old_fields)
                } else {
                    (&native, new_fields)
                };
                for field in fields {
                    combined[field] = source[field].clone();
                }
            }
            assert!(
                serde_json::from_value::<Projection>(combined.clone()).is_err(),
                "version mask {bits}"
            );
            let bson = bson::to_document(&combined).unwrap();
            assert!(
                bson::from_document::<Projection>(bson).is_err(),
                "BSON version mask {bits}"
            );
        }
    }

    #[test]
    fn stored_thread_and_origin_require_supported_version_and_exact_manager() {
        use crate::models::assistant_conversation::{PlatformThread, PlatformTurnOrigin};
        let legacy = json!({"platform":"telegram","profile_id":"p","manager_bot_id":"99","telegram_bot_id":99,
            "binding_id":"b","binding_generation":1,"user_id":"u","nyxbot_agent_id":"n"});
        let parsed: PlatformThread = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), legacy);
        let native = json!({"platform":"slack","profile_id":"p","manager_bot_id":"B-opaque",
            "platform_entry":{"version":2,"platform":"slack","channel_bot_id":"8f053bdd-cffe-400c-a0fa-93953aa70f13","platform_bot_id":"B-opaque"},
            "binding_id":"b","binding_generation":1,"user_id":"u","nyxbot_agent_id":"n"});
        let parsed: PlatformThread = serde_json::from_value(native.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), native);
        for (field, value) in [
            ("version", json!(99)),
            ("platform_bot_id", json!("B-conflict")),
            ("channel_bot_id", json!("not-a-channel")),
            ("platform_bot_id", json!("x".repeat(257))),
        ] {
            let mut invalid = native.clone();
            invalid["platform_entry"][field] = value;
            assert!(
                serde_json::from_value::<PlatformThread>(invalid).is_err(),
                "{field}"
            );
        }
        for (thread, sender) in [
            (native.clone(), json!({"sender_id":123,"chat_id":123})),
            (
                legacy.clone(),
                json!({"platform_sender":{"sender_id":"U-user","private_conversation_id":"D-chat"}}),
            ),
        ] {
            let mut origin = json!({"thread":thread,"manager_authority_epoch":1,"personal_control_revision":1,
                "source":{"kind":"ingress","receipt_id":"r","claim_id":"c"}});
            origin
                .as_object_mut()
                .unwrap()
                .extend(sender.as_object().unwrap().clone());
            assert!(serde_json::from_value::<PlatformTurnOrigin>(origin).is_err());
        }
    }

    #[test]
    fn historical_group_handoff_and_delivery_keep_negative_chat_identity() {
        use crate::models::{
            platform_manager_delivery::PlatformManagerDelivery,
            platform_manager_ingress::PlatformManagerIngress,
        };
        let now = bson::DateTime::now();
        let ingress = bson::doc! {"_id":"receipt","claim_id":"claim","profile_id":"profile","manager_bot_id":"99",
        "telegram_bot_id":99_i64,"manager_authority_epoch":1_i64,"observation_id":"observation",
        "sender_id":123_i64,"chat_id":-100987_i64,"update_id":456_i64,"message_id":789_i64,
        "original_private_command":false,"audience":{"kind":"private_chat_handoff"},"phase":"admitted",
        "execution":bson::Bson::Null,"decision":bson::Bson::Null,"reply_id":bson::Bson::Null,
        "delivery_recovery_queued":false,"effect_revision":0_i64,"admitted_at":now,"start_before":now,"settled_at":bson::Bson::Null};
        let parsed: PlatformManagerIngress = bson::from_document(ingress.clone()).unwrap();
        assert_eq!(
            bson::to_vec(&parsed).unwrap(),
            bson::to_vec(&ingress).unwrap()
        );
        let delivery = bson::doc! {"_id":"delivery","dedupe_key":"handoff","profile_id":"profile","manager_bot_id":"99",
        "telegram_bot_id":99_i64,"manager_authority_epoch":1_i64,"sender_id":123_i64,"chat_id":-100987_i64,
        "ingress_id":"receipt","ingress_claim_id":"claim","origin":bson::Bson::Null,"origin_turn_id":bson::Bson::Null,
        "reference":{"kind":"handoff"},"state":"pending","claim_id":bson::Bson::Null,
        "telegram_message_ids":[],"expires_at":now,"retry_at":now,"updated_at":now};
        let parsed: PlatformManagerDelivery = bson::from_document(delivery.clone()).unwrap();
        assert_eq!(
            bson::to_vec(&parsed).unwrap(),
            bson::to_vec(&delivery).unwrap()
        );
    }
}
