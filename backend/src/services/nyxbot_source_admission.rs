//! Pure validation and canonicalization of the dormant Discord policy.
use crate::models::nyxbot_source_admission::{
    DiscordSourceAdmission, IdentityRule, SourceAdmissionInput,
};
use crate::services::discord_gateway::valid_message_id;

impl TryFrom<SourceAdmissionInput> for DiscordSourceAdmission {
    type Error = &'static str;

    fn try_from(input: SourceAdmissionInput) -> Result<Self, Self::Error> {
        if input.schema_version != 1 {
            return Err("unsupported_source_admission_version");
        }
        if input.revision <= 0 {
            return Err("invalid_source_admission_revision");
        }
        if !valid_message_id(&input.bot_id) {
            return Err("invalid_source_admission_bot_id");
        }
        let senders = normalize(input.senders, |id| valid_message_id(id))?;
        let chats = normalize(input.chats, |chat| valid_message_id(&chat.id))?;
        if let IdentityRule::Allowlist(ids) = &chats
            && ids.windows(2).any(|pair| pair[0].id == pair[1].id)
        {
            return Err("conflicting_source_admission_conversation_kind");
        }
        Ok(Self {
            schema_version: input.schema_version,
            revision: input.revision,
            enabled: input.enabled,
            bot_id: input.bot_id,
            senders,
            chats,
        })
    }
}

fn normalize<T: Ord>(
    rule: IdentityRule<T>,
    valid: impl Fn(&T) -> bool,
) -> Result<IdentityRule<T>, &'static str> {
    match rule {
        IdentityRule::Allowlist(mut ids) => {
            if !ids.iter().all(valid) {
                return Err("invalid_source_admission_id");
            }
            ids.sort();
            ids.dedup();
            if ids.is_empty() {
                Ok(IdentityRule::Deny)
            } else {
                Ok(IdentityRule::Allowlist(ids))
            }
        }
        rule => Ok(rule),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        nyxbot_channel::NyxbotChannel,
        nyxbot_source_admission::{ConversationId, ConversationKind},
    };
    use bson::{Document, doc};
    use serde_json::{Value, json};

    fn policy() -> Value {
        json!({
            "schema_version": 1, "revision": 7, "enabled": false, "bot_id": "42",
            "senders": {"type": "allowlist", "ids": ["23", "22", "23"]},
            "chats": {"type": "allowlist", "ids": [
                {"id": "101", "kind": "channel"}, {"id": "102", "kind": "thread"},
                {"id": "100", "kind": "private"}, {"id": "103", "kind": "group"},
                {"id": "101", "kind": "channel"}
            ]}
        })
    }

    fn legacy_channel() -> Document {
        doc! {
            "_id": "connection", "user_id": "owner", "channel_bot_id": "bot",
            "platform": "discord", "bot_label": "bot", "transport": "gateway",
            "status": "active", "route_api_key_id": "route",
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()
        }
    }

    #[test]
    fn legacy_and_new_connections_have_no_policy_by_default() {
        let row: NyxbotChannel = bson::from_document(legacy_channel()).unwrap();
        assert!(row.discord_source_admission.is_none());
        assert!(
            !bson::to_document(&row)
                .unwrap()
                .contains_key("discord_source_admission")
        );
    }

    #[test]
    fn persisted_rules_round_trip_with_explicit_ids_and_revision() {
        let mut document = legacy_channel();
        document.insert(
            "discord_source_admission",
            bson::to_bson(&policy()).unwrap(),
        );
        let row: NyxbotChannel = bson::from_document(document).unwrap();
        let rule = row.discord_source_admission.clone().unwrap();
        assert_eq!(rule.revision, 7);
        assert!(!rule.enabled);
        assert_eq!(
            rule.senders,
            IdentityRule::Allowlist(vec!["22".into(), "23".into()])
        );
        assert_eq!(
            rule.chats,
            IdentityRule::Allowlist(vec![
                ConversationId {
                    id: "100".into(),
                    kind: ConversationKind::Private
                },
                ConversationId {
                    id: "101".into(),
                    kind: ConversationKind::Channel
                },
                ConversationId {
                    id: "102".into(),
                    kind: ConversationKind::Thread
                },
                ConversationId {
                    id: "103".into(),
                    kind: ConversationKind::Group
                },
            ])
        );
        let restored: NyxbotChannel =
            bson::from_document(bson::to_document(&row).unwrap()).unwrap();
        assert_eq!(restored.discord_source_admission, Some(rule));
    }

    #[test]
    fn enabled_empty_lists_are_denied_and_open_is_explicit() {
        let mut value = policy();
        value["enabled"] = json!(true);
        value["senders"]["ids"] = json!([]);
        value["chats"]["ids"] = json!([]);
        let rule: DiscordSourceAdmission = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(rule.senders, IdentityRule::Deny);
        assert_eq!(rule.chats, IdentityRule::Deny);
        value["chats"] = json!({"type": "open"});
        let rule: DiscordSourceAdmission = serde_json::from_value(value).unwrap();
        assert_eq!(rule.senders, IdentityRule::Deny);
        assert_eq!(rule.chats, IdentityRule::Open);
    }

    #[test]
    fn invalid_or_future_policy_is_rejected_on_read() {
        for (field, bad) in [
            ("schema_version", json!(2)),
            ("revision", json!(0)),
            ("bot_id", json!("same-display-name")),
            ("senders", json!({"type": "allowlist","ids":["01"]})),
            (
                "chats",
                json!({"type":"allowlist","ids":[{"id":"0","kind":"channel"}]}),
            ),
            (
                "chats",
                json!({"type":"allowlist","ids":[{"id":"1","kind":"guild"}]}),
            ),
            (
                "chats",
                json!({"type":"allowlist","ids":[{"id":"1","kind":"channel"},{"id":"1","kind":"thread"}]}),
            ),
        ] {
            let mut value = policy();
            value[field] = bad;
            let mut document = legacy_channel();
            document.insert("discord_source_admission", bson::to_bson(&value).unwrap());
            assert!(
                bson::from_document::<NyxbotChannel>(document).is_err(),
                "{field}"
            );
        }
        let mut value = policy();
        value.as_object_mut().unwrap().remove("enabled");
        assert!(serde_json::from_value::<DiscordSourceAdmission>(value).is_err());
    }
}
