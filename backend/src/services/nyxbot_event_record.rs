//! Contextual storage for the independent encrypted fields retained on a NyxBot event.
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    models::nyxbot_channel::{ChannelTurnDelivery, EVENTS_COLLECTION_NAME, NyxbotEvent},
};
use bson::{Bson, Document, doc};
use zeroize::Zeroizing;

pub(crate) const CONTEXT: &str = "event_context_ciphertext";
pub(crate) const TARGET: &str = "delivery.target_ciphertext";
pub(crate) const GUEST: &str = "guest_setup_payload";
pub(crate) const GUEST_VERSION: &str = "guest_setup_context_version";
pub(crate) const CHANNEL_GUEST: &str = "channel_guest_setup_payload";
pub(crate) const CHANNEL_GUEST_VERSION: &str = "channel_guest_setup_context_version";
pub(crate) const REPLY: &str = "response_reply.ciphertext";

pub(crate) async fn seal(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    field: &[&str],
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    let context = RecordContext::new(EVENTS_COLLECTION_NAME, id, owner, field)?;
    record_field::encrypt_context(keys, &context, bytes).await
}

pub(crate) fn version(keys: &EncryptionKeys) -> Option<u8> {
    keys.contextual_record_writes().then_some(3)
}

pub(crate) fn set_material(mut fields: Document, marker: &str, version: Option<u8>) -> Document {
    if let Some(version) = version {
        fields.insert(marker, i32::from(version));
        doc! {"$set":fields}
    } else {
        doc! {"$set":fields, "$unset":{marker:""}}
    }
}

pub(crate) async fn event_context(
    keys: &EncryptionKeys,
    event: &NyxbotEvent,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let field = [CONTEXT, &event.channel_id];
    record_field::decrypt_context(
        keys,
        &RecordContext::new(EVENTS_COLLECTION_NAME, &event.id, &event.user_id, &field)?,
        event.event_context_version,
        ciphertext,
    )
    .await
}

pub(crate) async fn delivery_target(
    keys: &EncryptionKeys,
    event: &NyxbotEvent,
    delivery: &ChannelTurnDelivery,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let field = [TARGET, &event.channel_id];
    record_field::decrypt_context(
        keys,
        &RecordContext::new(EVENTS_COLLECTION_NAME, &event.id, &event.user_id, &field)?,
        delivery.target_context_version,
        ciphertext,
    )
    .await
}

pub(crate) async fn guest_setup(
    keys: &EncryptionKeys,
    event: &bson::Document,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let invalid = || AppError::Internal("Guest queue unavailable".into());
    let (payload, marker) = match (event.contains_key(GUEST), event.contains_key(CHANNEL_GUEST)) {
        (true, false) if !event.contains_key(CHANNEL_GUEST_VERSION) => (GUEST, GUEST_VERSION),
        (false, true) if !event.contains_key(GUEST_VERSION) => {
            (CHANNEL_GUEST, CHANNEL_GUEST_VERSION)
        }
        _ => return Err(invalid()),
    };
    let field = [payload, event.get_str("channel_id").map_err(|_| invalid())?];
    let context = RecordContext::new(
        EVENTS_COLLECTION_NAME,
        event.get_str("_id").map_err(|_| invalid())?,
        event.get_str("user_id").map_err(|_| invalid())?,
        &field,
    )?;
    let version = event
        .get(marker)
        .cloned()
        .map(bson::from_bson::<Option<u8>>)
        .transpose()
        .map_err(|_| invalid())?
        .flatten();
    record_field::decrypt_context(
        keys,
        &context,
        version,
        event.get_binary_generic(payload).map_err(|_| invalid())?,
    )
    .await
}

fn invalid() -> AppError {
    AppError::Internal("Invalid encrypted channel event".into())
}

pub(in crate::services) fn stored<'a>(
    row: &'a Document,
    path: &str,
) -> AppResult<Option<&'a Bson>> {
    let value = match path.split_once('.') {
        None => row.get(path),
        Some((parent, leaf)) => match row.get(parent) {
            None | Some(Bson::Null) => None,
            Some(Bson::Document(fields)) => fields.get(leaf),
            _ => return Err(invalid()),
        },
    };
    Ok(value.filter(|v| **v != Bson::Null))
}

pub(in crate::services) fn validate_plain(
    row: &Document,
    field: &str,
    bytes: &[u8],
) -> AppResult<()> {
    match field {
        CONTEXT => {
            let context: serde_json::Value =
                serde_json::from_slice(bytes).map_err(|_| invalid())?;
            if !context.is_object()
                || context["activity"]["event_id"]
                    .as_str()
                    .is_some_and(|id| row.get_str("event_id") != Ok(id))
                || (row.get_str("custom_binding_id").is_err()
                    && context["conversation_id"]
                        .as_str()
                        .is_some_and(|id| row.get_str("partition") != Ok(id)))
            {
                return Err(invalid());
            }
        }
        TARGET => {
            if bytes.is_empty()
                || bytes.len() > 4096
                || std::str::from_utf8(bytes).is_err()
                || !matches!(
                    stored(row, "delivery.target_expires_at")?,
                    Some(Bson::DateTime(_))
                )
            {
                return Err(invalid());
            }
        }
        GUEST | CHANNEL_GUEST => {
            let guest: crate::models::nyxbot_channel::WaitingGuest =
                serde_json::from_slice(bytes).map_err(|_| invalid())?;
            let add_on = guest
                .channel_context
                .as_ref()
                .and_then(|context| context.channel_guest.as_ref());
            let valid_lane = match (field, add_on) {
                (CHANNEL_GUEST, Some(turn)) => {
                    row.get_str("_id") == Ok(turn.source_event_id.as_str())
                        && row.get_str("channel_id") == Ok(turn.binding_id.as_str())
                        && turn.hosted_conversation_id() == guest.conversation
                }
                (GUEST, None) => {
                    !super::channel_guest_access::is_hosted_conversation(&guest.conversation)
                }
                _ => false,
            };
            if row.get_str("guest_setup_conversation_id") != Ok(guest.conversation.as_str())
                || row.get_str("channel_id") != Ok(guest.origin.nyxbot_channel_id.as_str())
                || !valid_lane
            {
                return Err(invalid());
            }
        }
        REPLY => {
            let reply = row.get_document("response_reply").map_err(|_| invalid())?;
            let id = reply.get_str("response_id").map_err(|_| invalid())?;
            let body = super::nyxbot_response_recovery::decode(bytes)?;
            let mapping = super::nyxbot_response_read::mapping(row)?.ok_or_else(invalid)?;
            let command = mapping
                .commands
                .iter()
                .find(|c| c.response_id == id)
                .ok_or_else(invalid)?;
            if reply.get_i32("version") != Ok(1)
                || body.snapshot["id"] != id
                || body.snapshot["status"] != "completed"
                || body.snapshot["conversation"]
                    != serde_json::json!({"id":mapping.conversation_id})
                || body.snapshot["metadata"] != serde_json::json!(command.metadata)
            {
                return Err(invalid());
            }
            if let Some(notice) = row.get("org_notice").and_then(Bson::as_document)
                && notice.get_str("response_id") == Ok(id)
            {
                let text = body.snapshot["output"][0]["content"][0]["text"]
                    .as_str()
                    .ok_or_else(invalid)?;
                if notice.get_str("text_sha256")
                    != Ok(crate::crypto::token::hash_token(text).as_str())
                {
                    return Err(invalid());
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

pub(in crate::services) const FIELDS: &[(&str, &str)] = &[
    (TARGET, "delivery.target_context_version"),
    (CONTEXT, "event_context_version"),
    (GUEST, GUEST_VERSION),
    (REPLY, "response_reply.context_version"),
    (CHANNEL_GUEST, CHANNEL_GUEST_VERSION),
];

async fn inspect(keys: &EncryptionKeys, row: &Document, rewrap: bool) -> AppResult<Document> {
    if (stored(row, GUEST)?.is_some() || stored(row, GUEST_VERSION)?.is_some())
        && (stored(row, CHANNEL_GUEST)?.is_some() || stored(row, CHANNEL_GUEST_VERSION)?.is_some())
    {
        return Err(invalid());
    }
    let mut set = Document::new();
    for &(field, marker) in FIELDS {
        let version = stored(row, marker)?
            .map(|v| bson::from_bson::<u8>(v.clone()))
            .transpose()
            .map_err(|_| invalid())?;
        let Some(value) = stored(row, field)? else {
            if version.is_some() || (field == REPLY && stored(row, "response_reply")?.is_some()) {
                return Err(invalid());
            }
            continue;
        };
        let channel = row.get_str("channel_id").map_err(|_| invalid())?;
        let mut parts = vec![field, channel];
        if field == REPLY {
            parts.push(
                stored(row, "response_reply.response_id")?
                    .and_then(Bson::as_str)
                    .ok_or_else(invalid)?,
            );
        }
        let ctx = RecordContext::new(
            EVENTS_COLLECTION_NAME,
            row.get_str("_id").map_err(|_| invalid())?,
            row.get_str("user_id").map_err(|_| invalid())?,
            &parts,
        )?;
        let bytes = super::record_field_service::stored_bytes(value)?;
        let current =
            version == Some(3) && keys.authenticate_current_record(&bytes, &ctx).await.is_ok();
        if !current && !rewrap {
            return Err(invalid());
        }
        let plain = record_field::decrypt_context(keys, &ctx, version, &bytes).await?;
        validate_plain(row, field, &plain)?;
        if !current {
            let sealed = keys.encrypt_record(&plain, &ctx).await?;
            keys.authenticate_current_record(&sealed, &ctx).await?;
            set.insert(
                field,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: sealed,
                },
            );
            set.insert(marker, 3_i32);
        }
    }
    Ok(set)
}

pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    inspect(keys, source, false).await.map(drop)
}

pub(crate) async fn migrate(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let set = inspect(keys, source, true).await?;
    if set.is_empty() {
        return Ok(false);
    }
    let result = super::transaction_service::durable_database(db)
        .collection::<Document>(EVENTS_COLLECTION_NAME)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set":set},
        )
        .await?;
    Ok(result.matched_count == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::encrypted_record_migration as migration;

    #[tokio::test]
    async fn channel_guest_wait_migration_keeps_source_owner_channel_and_field_bound() {
        use crate::models::channel_guest_access::{
            AnswerAudience, GuestActor, GuestAssurance, GuestTurn,
        };
        let db =
            crate::test_utils::connect_transaction_test_database("channel_guest_wait_record").await;
        let keys = crate::test_utils::test_encryption_keys();
        let turn = GuestTurn {
            bot_id: "bot".into(),
            binding_id: "channel".into(),
            platform: "telegram".into(),
            conversation_id: "provider-thread".into(),
            source_event_id: "event".into(),
            actor: GuestActor {
                id: "sender".into(),
                assurance: GuestAssurance::ProviderVerified,
            },
            audience: AnswerAudience::Private,
            shared_room_verified: false,
            policy_revision: 0,
            activation_epoch: 1,
            bot_revision: 1,
            meta_grant_id: None,
            hosted_assignment: None,
        };
        let conversation = turn.hosted_conversation_id();
        let plain = serde_json::to_vec(&serde_json::json!({
            "agent": "agent", "profile": "profile", "text": "text",
            "conversation": conversation, "origin": {
                "nyxbot_channel_id": "channel", "partition": "provider-thread", "platform": "telegram",
            }, "channel_context": {
                "channel_guest": turn, "source_event_id": "event", "channel_id": "channel",
                "channel_bot_id": "bot", "conversation_kind": "private",
                "sender_kind": "human", "guest": true, "original_message": true,
            },
        })).unwrap();
        let mut legacy = crate::test_utils::test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        let bytes = seal(
            &legacy,
            "event",
            "owner",
            &[CHANNEL_GUEST, "channel"],
            &plain,
        )
        .await
        .unwrap();
        let source = doc! {
            "_id": "event", "user_id": "owner", "channel_id": "channel",
            "guest_setup_conversation_id": &conversation,
            CHANNEL_GUEST: bson::Binary { subtype:bson::spec::BinarySubtype::Generic, bytes },
        };
        assert_eq!(guest_setup(&keys, &source).await.unwrap().as_slice(), plain);
        let rows = db.collection::<Document>(EVENTS_COLLECTION_NAME);
        rows.insert_one(&source).await.unwrap();
        assert!(verify(&keys, &source).await.is_err());
        assert!(migrate(&db, &keys, &source).await.unwrap());
        let saved = rows.find_one(doc! {"_id": "event"}).await.unwrap().unwrap();
        assert_eq!(saved.get_i32(CHANNEL_GUEST_VERSION), Ok(3));
        assert_eq!(guest_setup(&keys, &saved).await.unwrap().as_slice(), plain);
        verify(&keys, &saved).await.unwrap();

        for field in ["_id", "user_id", "channel_id"] {
            let mut substituted = saved.clone();
            substituted.insert(field, "other");
            assert!(guest_setup(&keys, &substituted).await.is_err());
            assert!(verify(&keys, &substituted).await.is_err());
        }
        let mut substituted = saved;
        let ciphertext = substituted.remove(CHANNEL_GUEST).unwrap();
        substituted.remove(CHANNEL_GUEST_VERSION);
        substituted.insert(GUEST, ciphertext);
        substituted.insert(GUEST_VERSION, 3);
        assert!(guest_setup(&keys, &substituted).await.is_err());
        assert!(verify(&keys, &substituted).await.is_err());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn event_record_generation_resumes_after_rewrap_and_counts_each_field() {
        let db = crate::test_utils::connect_transaction_test_database("event_generation").await;
        let keys = crate::test_utils::test_encryption_keys();
        let rows = db.collection::<Document>(EVENTS_COLLECTION_NAME);
        let mut legacy = crate::test_utils::test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        let bytes = seal(
            &legacy,
            "retained",
            "owner",
            &[CONTEXT, "channel"],
            br#"{"activity":{"event_id":"message"}}"#,
        )
        .await
        .unwrap();
        let source = doc! {
            "_id":"retained", "channel_id":"channel", "user_id":"owner", "event_id":"message",
            CONTEXT: bson::Binary {subtype:bson::spec::BinarySubtype::Generic, bytes},
        };
        let mut other = source.clone();
        other.insert("_id", "second");
        rows.insert_many([
            source.clone(),
            other,
            doc! {"_id":"cleared", "delivery":null},
            doc! {"_id":"damaged", "delivery":{"target_context_version":3}},
        ])
        .await
        .unwrap();
        let run = migration::start(&db, &keys, migration::Mode::Migrate, None)
            .await
            .unwrap();
        assert!(migrate(&db, &keys, &source).await.unwrap());
        loop {
            let report = migration::status(&db, &keys).await.unwrap().unwrap();
            if report.phase == "observed" {
                assert_eq!((report.rewritten, report.unresolved), (1, 1));
                let fields: Vec<_> = report
                    .field_outcomes
                    .iter()
                    .filter(|field| field.collection == EVENTS_COLLECTION_NAME)
                    .collect();
                assert_eq!(fields.len(), 5);
                for field in fields {
                    let present = if field.field == CONTEXT { 2 } else { 0 };
                    assert_eq!(
                        (
                            field.authenticated_current,
                            field.no_material_observed,
                            field.verification_blocked
                        ),
                        (present, 3 - present, 1)
                    );
                }
                break;
            }
            Box::pin(migration::advance(&db, &keys, &run))
                .await
                .unwrap();
        }
        db.drop().await.unwrap();
    }
}
