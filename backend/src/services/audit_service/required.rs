use mongodb::Database;
use sha2::{Digest, Sha256};

use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_log::AuditLog,
        security_effect_intent::{SecurityEffect, SecurityEffectIntent},
    },
    services::{audit_chain_service, security_effect_service},
};

use super::AuditActor;

// These are forensic identifiers and bounded facts, never source documents.
const MAX_INTENT_METADATA_BYTES: usize = 16 * 1024;
const TRUNCATED: &str = "...[truncated]";
pub(crate) const MAX_IDENTIFIER_LIST_BYTES: usize = 8 * 1024;

/// Byte budgets include the visible marker. Input validation and source values
/// are unchanged; only the audit representation is shortened.
#[derive(Clone, Copy)]
pub(crate) enum TextField {
    Identifier,
    ForwardedIp,
    KeyName,
    UserAgent,
    Email,
    DisplayName,
    AvatarUrl,
}

impl TextField {
    fn byte_limit(self) -> usize {
        match self {
            Self::Identifier | Self::ForwardedIp | Self::KeyName => 256,
            Self::UserAgent => 1024,
            Self::Email | Self::DisplayName => 512,
            Self::AvatarUrl => 4096,
        }
    }
}

pub(crate) fn text(field: TextField, value: &str) -> String {
    let limit = field.byte_limit();
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit - TRUNCATED.len();
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATED}", &value[..end])
}

/// Record the original-order prefix of an identifier list within 8 KiB of
/// BSON array bytes, including element/index overhead. Source values and
/// validation are unchanged; the digest covers all submitted IDs, before caps,
/// separated by one newline with no trailing newline.
pub(crate) fn identifier_list(
    name: &'static str,
    ids: &[String],
) -> serde_json::Map<String, serde_json::Value> {
    bounded_identifier_list(name, ids.iter().map(|id| [id.as_str()]), |[id]| {
        let value = text(TextField::Identifier, id);
        let bytes = 5 + value.len(); // BSON string length, bytes, terminator.
        (value.into(), bytes)
    })
}

/// The same list budget for ordered pairs of identifiers, preserving each
/// record's field names. The digest joins all original IDs in record/field
/// order with newlines, just as the scalar form does.
pub(crate) fn identifier_pairs(
    name: &'static str,
    keys: [&'static str; 2],
    pairs: &[(&str, &str)],
) -> serde_json::Map<String, serde_json::Value> {
    bounded_identifier_list(name, pairs.iter().map(|(a, b)| [*a, *b]), |ids| {
        let mut value = serde_json::Map::new();
        let mut bytes = 5; // BSON embedded document length and terminator.
        for (key, id) in keys.into_iter().zip(ids) {
            let id = text(TextField::Identifier, id);
            bytes += key.len() + 7 + id.len(); // Type, key cstring, string length and terminator.
            value.insert(key.into(), id.into());
        }
        (value.into(), bytes)
    })
}

fn bounded_identifier_list<'a, const N: usize>(
    name: &'static str,
    entries: impl IntoIterator<Item = [&'a str; N]>,
    render: impl Fn([&'a str; N]) -> (serde_json::Value, usize),
) -> serde_json::Map<String, serde_json::Value> {
    let mut kept = Vec::new();
    let mut bytes = 5;
    let mut count = 0_usize;
    let mut digest = Sha256::new();
    let mut first_id = true;
    let mut full = false;
    for ids in entries {
        count += 1;
        for id in ids {
            if !first_id {
                digest.update(b"\n");
            }
            first_id = false;
            digest.update(id.as_bytes());
        }
        if !full {
            let (value, value_bytes) = render(ids);
            let element_bytes = 2 + kept.len().to_string().len() + value_bytes;
            if bytes + element_bytes <= MAX_IDENTIFIER_LIST_BYTES {
                bytes += element_bytes;
                kept.push(value);
            } else {
                full = true;
            }
        }
    }
    let shortened = kept.len() != count;
    let mut fields = serde_json::Map::new();
    fields.insert(name.into(), kept.into());
    if shortened {
        fields.insert(format!("{name}_count"), count.into());
        fields.insert(format!("{name}_truncated"), true.into());
        fields.insert(
            format!("{name}_sha256"),
            hex::encode(digest.finalize()).into(),
        );
    }
    fields
}

fn optional_text(field: TextField, value: Option<&str>) -> Option<String> {
    value.map(|value| text(field, value))
}

/// Callers select metadata explicitly; serializing a request/model/provider
/// response into an intent is forbidden. Validation must succeed before the source commits.
pub fn intent(
    actor: Option<&AuditActor>,
    event_type: &str,
    metadata: Option<serde_json::Value>,
) -> AppResult<SecurityEffectIntent> {
    let event = super::build_audit_entry(
        actor.map(|actor| text(TextField::Identifier, &actor.user_id)),
        event_type.to_owned(),
        metadata,
        optional_text(
            TextField::ForwardedIp,
            actor.and_then(|actor| actor.ip_address.as_deref()),
        ),
        optional_text(
            TextField::UserAgent,
            actor.and_then(|actor| actor.user_agent.as_deref()),
        ),
        optional_text(
            TextField::Identifier,
            actor.and_then(|actor| actor.api_key_id.as_deref()),
        ),
        optional_text(
            TextField::KeyName,
            actor.and_then(|actor| actor.api_key_name.as_deref()),
        ),
    );
    checked_intent(event)
}

/// Preserve ordinary actor facts during a durability migration. Values above
/// their audit byte budget retain a prefix and a visible truncation marker.
pub fn intent_preserving_actor(
    actor: &AuditActor,
    event_type: &str,
    metadata: Option<serde_json::Value>,
) -> AppResult<SecurityEffectIntent> {
    intent(Some(actor), event_type, metadata)
}

/// OAuth client requests have network context but no human acting user.
pub fn client_intent(
    event_type: &str,
    metadata: Option<serde_json::Value>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<SecurityEffectIntent> {
    checked_intent(super::build_audit_entry(
        None,
        event_type.into(),
        metadata,
        optional_text(TextField::ForwardedIp, ip_address),
        optional_text(TextField::UserAgent, user_agent),
        None,
        None,
    ))
}

fn checked_intent(event: AuditLog) -> AppResult<SecurityEffectIntent> {
    if event.event_type.len() > 128
        || bson::to_vec(&event)
            .map_err(|_| AppError::Internal("Required audit metadata could not be encoded".into()))?
            .len()
            > MAX_INTENT_METADATA_BYTES
    {
        return Err(AppError::Internal(
            "Required audit metadata exceeds its bound".into(),
        ));
    }
    Ok(security_effect_service::new_intent(
        event.id.clone(),
        SecurityEffect::Audit {
            event: Box::new(event),
        },
    ))
}

/// Replaying an immutable intent preserves its UUID and timestamp. The chain
/// append validates an existing UUID's immutable content before acknowledging.
pub(crate) async fn append_intent(db: &Database, event: &AuditLog) -> AppResult<()> {
    let key = super::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain is not initialized".into()))?;
    let inserted = audit_chain_service::append_chained_entry(db, event.clone(), key).await?;
    #[cfg(test)]
    super::notify_test_audit_write(&inserted);
    #[cfg(not(test))]
    let _ = inserted;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_list_preserves_short_lists_and_hashes_uncapped_input_in_order() {
        let ids = vec!["second".to_owned(), "first".to_owned(), "second".to_owned()];
        assert_eq!(
            serde_json::Value::Object(identifier_list("ids", &ids)),
            serde_json::json!({"ids":ids})
        );
        let ids = vec!["界".repeat(512); 80];
        let data = identifier_list("ids", &ids);
        let kept = data["ids"].as_array().unwrap();
        assert!(kept.len() < ids.len());
        assert!(
            kept.iter()
                .all(|v| v.as_str().unwrap() == text(TextField::Identifier, &ids[0]))
        );
        assert_eq!(data["ids_count"], 80);
        assert_eq!(data["ids_truncated"], true);
        assert_eq!(
            data["ids_sha256"],
            hex::encode(Sha256::digest(ids.join("\n").as_bytes()))
        );
    }

    #[test]
    fn required_user_agent_is_bounded_marked_and_utf8_safe_at_every_entrypoint() {
        for agent in ["a".repeat(1024), "a".repeat(1025), "界".repeat(1024)] {
            let actor = AuditActor {
                user_id: uuid::Uuid::new_v4().to_string(),
                ip_address: None,
                user_agent: Some(agent.clone()),
                api_key_id: None,
                api_key_name: None,
            };
            for result in [
                intent(Some(&actor), "event", None),
                intent_preserving_actor(&actor, "event", None),
                client_intent("event", None, None, Some(&agent)),
            ] {
                let SecurityEffect::Audit { event } = result.unwrap().effect else {
                    panic!("expected audit");
                };
                let stored = event.user_agent.unwrap();
                assert!(stored.len() <= TextField::UserAgent.byte_limit());
                if agent.len() <= TextField::UserAgent.byte_limit() {
                    assert_eq!(stored, agent);
                } else {
                    let prefix = stored.strip_suffix(TRUNCATED).unwrap();
                    assert!(agent.starts_with(prefix));
                    assert!(TextField::UserAgent.byte_limit() - stored.len() < 4);
                }
            }
        }
    }
    #[test]
    fn required_text_caps_preserve_short_values_and_mark_utf8_truncation() {
        for field in [
            TextField::Identifier,
            TextField::ForwardedIp,
            TextField::KeyName,
            TextField::UserAgent,
            TextField::Email,
            TextField::DisplayName,
            TextField::AvatarUrl,
        ] {
            let limit = field.byte_limit();
            for value in ["a".repeat(limit), "a".repeat(limit + 1), "界".repeat(limit)] {
                let stored = text(field, &value);
                assert!(stored.len() <= limit);
                if value.len() <= limit {
                    assert_eq!(stored, value);
                } else {
                    let prefix = stored.strip_suffix(TRUNCATED).unwrap();
                    assert!(value.starts_with(prefix));
                    assert!(limit - stored.len() < 4);
                }
            }
        }
    }

    #[tokio::test]
    async fn required_audit_maximum_capped_occurrences_fit_backstop() {
        use serde_json::json;
        let max = |field| text(field, &"x".repeat(64 * 1024));
        let actor = AuditActor {
            user_id: max(TextField::Identifier),
            ip_address: Some(max(TextField::ForwardedIp)),
            user_agent: Some(max(TextField::UserAgent)),
            api_key_id: Some(max(TextField::Identifier)),
            api_key_name: Some(max(TextField::KeyName)),
        };
        let id = max(TextField::Identifier);
        let rows = [
            (
                "admin.user.created",
                json!({"target_user_id":id,"target_email":max(TextField::Email),"role":"operator"}),
            ),
            (
                "admin.user.updated",
                json!({"target_user_id":id,"target_email":max(TextField::Email),"changes":{
                "display_name":max(TextField::DisplayName),"email":max(TextField::Email),"avatar_url":max(TextField::AvatarUrl)}}),
            ),
            (
                "admin.user.role_changed",
                json!({"target_user_id":id,"role":"operator"}),
            ),
            (
                "admin.user.status_changed",
                json!({"target_user_id":id,"is_active":false}),
            ),
            ("admin.user.password_reset", json!({"target_user_id":id})),
            ("admin.user.email_verified", json!({"target_user_id":id})),
            (
                "admin.user.sessions_revoked",
                json!({"target_user_id":id,"revoked_count":i64::MAX}),
            ),
            (
                "oauth_consent_revoked",
                json!({"client_id":id,"revoked_refresh_tokens":i64::MAX,"revoked_broker_bindings":i64::MAX}),
            ),
            (
                "oauth_broker_binding_revoked",
                json!({"client_id":id,"revoke_source":"client","binding_hash":"h".repeat(16),"reason":"client_revoked"}),
            ),
            (
                "oauth_broker_binding_reuse_detected",
                json!({"client_id":id,"binding_hash":"h".repeat(16),"cascade_revoke_count":i64::MAX}),
            ),
        ];
        for (name, metadata) in rows {
            let turn = uuid::Uuid::new_v4().to_string();
            let result =
                crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                    intent_preserving_actor(&actor, name, Some(metadata.clone()))
                })
                .await
                .unwrap();
            let SecurityEffect::Audit { event } = result.effect else {
                panic!("expected audit");
            };
            let bytes = bson::to_vec(&event).unwrap().len();
            assert!(bytes < MAX_INTENT_METADATA_BYTES);
            println!(
                "REQUIRED_AUDIT_MAX {name} bytes={bytes} margin={}",
                MAX_INTENT_METADATA_BYTES - bytes
            );
            for result in [
                intent(Some(&actor), name, Some(metadata.clone())),
                client_intent(
                    name,
                    Some(metadata),
                    actor.ip_address.as_deref(),
                    actor.user_agent.as_deref(),
                ),
            ] {
                let SecurityEffect::Audit { event } = result.unwrap().effect else {
                    panic!("expected audit");
                };
                assert!(bson::to_vec(&event).unwrap().len() <= bytes);
            }
        }
    }

    #[test]
    fn required_actor_caps_apply_at_every_entrypoint() {
        let raw = "a".repeat(64 * 1024);
        let actor = AuditActor {
            user_id: raw.clone(),
            ip_address: Some(raw.clone()),
            user_agent: Some(raw.clone()),
            api_key_id: Some(raw.clone()),
            api_key_name: Some(raw.clone()),
        };
        for result in [
            intent(Some(&actor), "event", None),
            intent_preserving_actor(&actor, "event", None),
        ] {
            let SecurityEffect::Audit { event } = result.unwrap().effect else {
                panic!("expected audit");
            };
            for value in [
                event.user_id,
                event.ip_address,
                event.api_key_id,
                event.api_key_name,
            ] {
                assert_eq!(value.unwrap(), format!("{}...[truncated]", "a".repeat(242)));
            }
            assert_eq!(
                event.user_agent.unwrap(),
                format!("{}...[truncated]", "a".repeat(1010))
            );
        }
        let SecurityEffect::Audit { event } = client_intent("event", None, Some(&raw), Some(&raw))
            .unwrap()
            .effect
        else {
            panic!("expected audit");
        };
        assert_eq!(
            event.ip_address.unwrap(),
            format!("{}...[truncated]", "a".repeat(242))
        );
        assert_eq!(
            event.user_agent.unwrap(),
            format!("{}...[truncated]", "a".repeat(1010))
        );
    }
}
