//! Bounded provider observations, joined only to an authorized outbound page.

use crate::errors::AppResult;
use crate::models::channel_delivery::{COLLECTION_NAME, DeliveryReceipt, ReceiptTimes};
use crate::models::{channel_bot::ChannelBot, channel_message::ChannelMessage};
use bson::{Document, doc};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};

pub const MAX_COMPONENTS: usize = 64;
pub const MAX_RECEIPTS: usize = 128;
pub const RETENTION_DAYS: i64 = 30;

pub fn valid_message_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && id.bytes().all(|b| b.is_ascii_graphic())
}
pub fn valid_recipient(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Clone, Copy)]
pub enum ReceiptStatus {
    Sent,
    Delivered,
    Read,
    Played,
    Failed,
}
impl ReceiptStatus {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "sent" => Self::Sent,
            "delivered" => Self::Delivered,
            "read" => Self::Read,
            "played" => Self::Played,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
    fn field(self) -> &'static str {
        match self {
            Self::Sent => "sent_at",
            Self::Delivered => "delivered_at",
            Self::Read => "read_at",
            Self::Played => "played_at",
            Self::Failed => "failed_at",
        }
    }
}

pub struct ReceiptObservation {
    pub message_id: String,
    pub recipient_id: String,
    pub status: ReceiptStatus,
    pub provider_at: DateTime<Utc>,
    pub error_codes: Vec<i64>,
}

fn identity(
    bot: &str,
    owner: &str,
    phone: &str,
    waba: Option<&str>,
    recipient: &str,
    message: &str,
) -> String {
    let mut hash = Sha256::new();
    for part in [
        bot,
        owner,
        phone,
        waba.unwrap_or_default(),
        recipient,
        message,
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hex::encode(hash.finalize())
}

pub async fn ensure_indexes(db: &mongodb::Database) -> Result<(), mongodb::error::Error> {
    db.collection::<Document>(COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"identity_key": 1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .name("channel_receipt_identity_unique".to_string())
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

/// Caller supplies events parsed from an authenticated, phone/WABA-filtered webhook.
pub(crate) async fn observe(
    db: &mongodb::Database,
    bot: &ChannelBot,
    observations: &[ReceiptObservation],
) -> AppResult<()> {
    if bot.platform != "whatsapp" || !valid_recipient(&bot.platform_bot_id) {
        return Ok(());
    }
    for event in observations.iter().take(MAX_RECEIPTS) {
        if !valid_message_id(&event.message_id)
            || !valid_recipient(&event.recipient_id)
            || event.provider_at < Utc::now() - chrono::Duration::days(RETENTION_DAYS)
            || event.provider_at > Utc::now() + chrono::Duration::days(1)
        {
            continue;
        }
        let codes: Vec<_> = event
            .error_codes
            .iter()
            .copied()
            .filter(|code| (0..=i64::from(i32::MAX)).contains(code))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(8)
            .collect();
        let id = identity(
            &bot.id,
            &bot.user_id,
            &bot.platform_bot_id,
            bot.app_id.as_deref(),
            &event.recipient_id,
            &event.message_id,
        );
        let mut set = doc! {
            "_id": {"$ifNull": ["$_id", {"$literal": uuid::Uuid::new_v4().to_string()}]},
            "bot_id": {"$literal": &bot.id}, "user_id": {"$literal": &bot.user_id},
            "phone_number_id": {"$literal": &bot.platform_bot_id}, "waba_id": {"$literal": &bot.app_id},
            "recipient_id": {"$literal": &event.recipient_id}, "platform_message_id": {"$literal": &event.message_id},
            "expires_at": {"$ifNull": ["$expires_at", {"$dateAdd": {"startDate":"$$NOW", "unit":"day", "amount":RETENTION_DAYS}}]},
            "error_codes": {"$slice": [{"$sortArray": {"input": {"$setUnion": [{"$ifNull":["$error_codes", []]}, codes]}, "sortBy":1}}, 8]},
        };
        let field = format!("times.{}", event.status.field());
        set.insert(
            &field,
            doc! {"$max": [format!("${field}"), bson::DateTime::from_chrono(event.provider_at)]},
        );
        // Logical expiry applies even before the TTL monitor removes the row.
        let update = vec![
            doc! {"$set": {
                "times": {"$cond": [{"$lte": ["$expires_at", "$$NOW"]}, {}, {"$ifNull": ["$times", {}]}]},
                "error_codes": {"$cond": [{"$lte": ["$expires_at", "$$NOW"]}, [], {"$ifNull": ["$error_codes", []]}]},
                "expires_at": {"$cond": [{"$lte": ["$expires_at", "$$NOW"]}, null, "$expires_at"]},
            }},
            doc! {"$set": set},
        ];
        let col = db.collection::<Document>(COLLECTION_NAME);
        for attempt in 0..3 {
            match col
                .update_one(doc! {"identity_key":&id}, update.clone())
                .upsert(true)
                .await
            {
                Ok(result) if result.matched_count == 1 || result.upserted_id.is_some() => break,
                Ok(_) => {
                    return Err(crate::errors::AppError::Internal(
                        "Receipt update did not persist an observation".into(),
                    ));
                }
                Err(error)
                    if attempt < 2
                        && matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000) =>
                {
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

pub struct ComponentSummary {
    pub id: String,
    pub status: &'static str,
    pub times: ReceiptTimes,
    pub error_codes: Vec<i64>,
}
pub struct DeliverySummary {
    pub status: &'static str,
    pub complete: bool,
    pub expected_components: Option<u32>,
    pub recipient_only: bool,
    pub failure_code: Option<u32>,
    pub components: Vec<ComponentSummary>,
}

fn receipt_status(times: &ReceiptTimes) -> &'static str {
    if times.played_at.is_some() {
        "played"
    } else if times.read_at.is_some() {
        "read"
    } else if times.delivered_at.is_some() {
        "delivered"
    } else if times.failed_at.is_some() {
        "failed"
    } else if times.sent_at.is_some() {
        "sent"
    } else {
        "accepted"
    }
}

/// One bounded query for the already-authorized page, never an unmatched receipt API.
pub async fn summaries(
    db: &mongodb::Database,
    messages: &[ChannelMessage],
    owner: &str,
    individual: bool,
) -> AppResult<HashMap<String, DeliverySummary>> {
    let messages: Vec<_> = messages
        .iter()
        .take(100)
        .filter(|m| m.user_id == owner && m.platform == "whatsapp" && m.direction == "outbound")
        .collect();
    let mut keys = Vec::new();
    for m in &messages {
        if let (Some(bot), Some(send)) = (&m.channel_bot_id, &m.platform_send) {
            for id in send.component_ids.iter().take(MAX_COMPONENTS) {
                keys.push(identity(
                    bot,
                    owner,
                    &send.phone_number_id,
                    send.waba_id.as_deref(),
                    &send.recipient_id,
                    id,
                ));
            }
        }
    }
    let receipts: HashMap<_, _> = if keys.is_empty() {
        HashMap::new()
    } else {
        db.collection::<DeliveryReceipt>(COLLECTION_NAME).find(doc! {"identity_key":{"$in":keys}, "user_id":owner, "expires_at":{"$gt":bson::DateTime::now()}})
            .limit((100 * MAX_COMPONENTS) as i64).await?.try_collect::<Vec<_>>().await?.into_iter().map(|r| (r.identity_key.clone(), r)).collect()
    };
    let mut result = HashMap::new();
    for m in messages {
        let Some(send) = &m.platform_send else {
            result.insert(
                m.id.clone(),
                DeliverySummary {
                    status: if m
                        .platform_message_id
                        .as_deref()
                        .is_some_and(valid_message_id)
                    {
                        "legacy_final_only"
                    } else {
                        "unknown"
                    },
                    complete: false,
                    expected_components: None,
                    recipient_only: !individual,
                    failure_code: None,
                    components: Vec::new(),
                },
            );
            continue;
        };
        let mut components = Vec::new();
        for id in send.component_ids.iter().take(MAX_COMPONENTS) {
            let key = identity(
                m.channel_bot_id.as_deref().unwrap_or_default(),
                owner,
                &send.phone_number_id,
                send.waba_id.as_deref(),
                &send.recipient_id,
                id,
            );
            let receipt = receipts.get(&key).filter(|r| {
                r.bot_id == m.channel_bot_id.as_deref().unwrap_or_default()
                    && r.user_id == owner
                    && r.phone_number_id == send.phone_number_id
                    && r.waba_id == send.waba_id
                    && r.recipient_id == send.recipient_id
                    && r.platform_message_id == *id
            });
            let mut times = receipt.map(|r| r.times.clone()).unwrap_or_default();
            if times.delivered_at.is_none() {
                times.delivered_at = times.read_at.or(times.played_at);
            }
            components.push(ComponentSummary {
                id: id.clone(),
                status: receipt_status(&times),
                times,
                error_codes: receipt.map(|r| r.error_codes.clone()).unwrap_or_default(),
            });
        }
        let complete = send.complete
            && !send.uncertain
            && send.expected_components > 0
            && components.len() == send.expected_components as usize
            && send.component_ids.iter().collect::<BTreeSet<_>>().len() == components.len();
        let status = if !complete {
            if !components.is_empty() {
                "partial"
            } else if send.uncertain {
                "unknown"
            } else {
                "failed"
            }
        } else if !individual {
            "recipient_only"
        } else if components.iter().all(|c| c.status == "played") {
            "played"
        } else if components
            .iter()
            .all(|c| matches!(c.status, "read" | "played"))
        {
            "read"
        } else if components
            .iter()
            .all(|c| matches!(c.status, "delivered" | "read" | "played"))
        {
            "delivered"
        } else if components.iter().any(|c| c.status == "failed") {
            "failed"
        } else if components.iter().all(|c| c.status != "accepted") {
            "sent"
        } else {
            "accepted"
        };
        result.insert(
            m.id.clone(),
            DeliverySummary {
                status,
                complete,
                expected_components: Some(send.expected_components),
                recipient_only: !individual,
                failure_code: send.failure_code,
                components,
            },
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::channel_delivery::PlatformSendRecord;

    fn bot() -> ChannelBot {
        bson::from_document(doc! {
            "_id":uuid::Uuid::new_v4().to_string(), "user_id":uuid::Uuid::new_v4().to_string(),
            "platform":"whatsapp", "label":"test", "platform_bot_id":"123", "app_id":"456",
            "platform_bot_username":"test", "bot_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic, bytes:vec![]},
            "webhook_secret_hash":"hash", "webhook_registered":true, "status":"active", "is_active":true,
            "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now()
        }).unwrap()
    }
    fn event(id: &str, status: ReceiptStatus, at: DateTime<Utc>) -> ReceiptObservation {
        ReceiptObservation {
            message_id: id.into(),
            recipient_id: "789".into(),
            status,
            provider_at: at,
            error_codes: vec![],
        }
    }
    async fn row(db: &mongodb::Database, bot: &ChannelBot, ids: &[&str]) -> ChannelMessage {
        super::super::channel_relay_service::store_outbound_message(
            db,
            &bot.id,
            "conversation",
            &bot.user_id,
            "whatsapp",
            "agent",
            None,
            ids.last().copied(),
            Some("789"),
            "text",
            Some(PlatformSendRecord {
                phone_number_id: bot.platform_bot_id.clone(),
                waba_id: bot.app_id.clone(),
                recipient_id: "789".into(),
                component_ids: ids.iter().map(|id| (*id).into()).collect(),
                expected_components: ids.len() as u32,
                complete: true,
                uncertain: false,
                failure_code: None,
            }),
        )
        .await
        .unwrap()
    }
    async fn summary(db: &mongodb::Database, message: &ChannelMessage) -> DeliverySummary {
        summaries(db, std::slice::from_ref(message), &message.user_id, true)
            .await
            .unwrap()
            .remove(&message.id)
            .unwrap()
    }

    #[tokio::test]
    async fn receipt_before_row_concurrent_creation_reordering_and_indexes() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_receipt_convergence")
                .await;
        ensure_indexes(&db).await.unwrap();
        let bot = bot();
        let now = DateTime::from_timestamp(Utc::now().timestamp(), 0).unwrap();
        let read = [event("wamid.one", ReceiptStatus::Read, now)];
        let mut failed = [event(
            "wamid.one",
            ReceiptStatus::Failed,
            now - chrono::Duration::seconds(1),
        )];
        failed[0].error_codes = vec![131026, -1, i64::MAX, 131026];
        let (a, b, c) = tokio::join!(
            observe(&db, &bot, &read),
            observe(&db, &bot, &failed),
            observe(&db, &bot, &read)
        );
        a.unwrap();
        b.unwrap();
        c.unwrap();
        observe(
            &db,
            &bot,
            &[event(
                "wamid.one",
                ReceiptStatus::Sent,
                now - chrono::Duration::seconds(2),
            )],
        )
        .await
        .unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let stored = db
            .collection::<DeliveryReceipt>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            uuid::Uuid::parse_str(&stored.id).unwrap().get_version_num(),
            4
        );
        assert_eq!(stored.times.read_at, Some(now));
        assert_eq!(stored.error_codes, vec![131026]);
        let raw = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert!(
            raw.get_document("times")
                .unwrap()
                .get_datetime("read_at")
                .is_ok()
        );
        assert!(raw.get_datetime("expires_at").is_ok());
        let indexes = db
            .collection::<Document>(COLLECTION_NAME)
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert!(
            indexes
                .iter()
                .any(|index| index.keys == doc! {"identity_key":1}
                    && index.options.as_ref().unwrap().unique == Some(true))
        );
        assert!(
            indexes
                .iter()
                .any(|index| index.keys == doc! {"expires_at":1}
                    && index.options.as_ref().unwrap().expire_after
                        == Some(std::time::Duration::ZERO))
        );
        let message = row(&db, &bot, &["wamid.one"]).await;
        let result = summary(&db, &message).await;
        assert_eq!(result.status, "read");
        assert_eq!(result.components[0].times.delivered_at, Some(now));
        assert_eq!(
            result.components[0].times.failed_at,
            Some(now - chrono::Duration::seconds(1))
        );
        // A newer duplicate advances only its own provider timestamp.
        observe(
            &db,
            &bot,
            &[event(
                "wamid.one",
                ReceiptStatus::Read,
                now + chrono::Duration::seconds(1),
            )],
        )
        .await
        .unwrap();
        observe(&db, &bot, &read).await.unwrap();
        assert_eq!(
            summary(&db, &message).await.components[0].times.read_at,
            Some(now + chrono::Duration::seconds(1))
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn receipt_matching_is_owner_bot_phone_waba_recipient_and_message_scoped() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_receipt_scope").await;
        ensure_indexes(&db).await.unwrap();
        let bot = bot();
        observe(
            &db,
            &bot,
            &[event("wamid.one", ReceiptStatus::Delivered, Utc::now())],
        )
        .await
        .unwrap();
        let message = row(&db, &bot, &["wamid.one"]).await;
        for field in ["owner", "bot", "phone", "waba", "recipient", "message"] {
            let mut wrong = message.clone();
            match field {
                "owner" => wrong.user_id = uuid::Uuid::new_v4().to_string(),
                "bot" => wrong.channel_bot_id = Some(uuid::Uuid::new_v4().to_string()),
                "phone" => wrong.platform_send.as_mut().unwrap().phone_number_id = "999".into(),
                "waba" => wrong.platform_send.as_mut().unwrap().waba_id = Some("999".into()),
                "recipient" => wrong.platform_send.as_mut().unwrap().recipient_id = "999".into(),
                _ => wrong.platform_send.as_mut().unwrap().component_ids[0] = "wamid.other".into(),
            }
            let result = summary(&db, &wrong).await;
            assert_eq!(result.status, "accepted", "{field}");
            assert!(result.components[0].times.delivered_at.is_none(), "{field}");
        }
        assert!(
            summaries(&db, std::slice::from_ref(&message), "other-owner", true)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            summaries(&db, &[], &bot.user_id, true)
                .await
                .unwrap()
                .is_empty()
        );
        let group = summaries(&db, std::slice::from_ref(&message), &bot.user_id, false)
            .await
            .unwrap()
            .remove(&message.id)
            .unwrap();
        assert_eq!(group.status, "recipient_only");
        assert!(group.recipient_only);
        // Message inventory itself also excludes corrupt cross-owner rows.
        let (page, count) = super::super::channel_relay_service::list_messages(
            &db,
            "conversation",
            "other-owner",
            1,
            50,
        )
        .await
        .unwrap();
        assert!(page.is_empty());
        assert_eq!(count, 0);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn all_components_required_partial_unknown_and_legacy_never_claim_delivery() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_receipt_parts").await;
        ensure_indexes(&db).await.unwrap();
        let bot = bot();
        let mut message = row(&db, &bot, &["wamid.one", "wamid.two"]).await;
        observe(
            &db,
            &bot,
            &[event("wamid.one", ReceiptStatus::Read, Utc::now())],
        )
        .await
        .unwrap();
        assert_eq!(summary(&db, &message).await.status, "accepted");
        observe(
            &db,
            &bot,
            &[event("wamid.two", ReceiptStatus::Failed, Utc::now())],
        )
        .await
        .unwrap();
        assert_eq!(summary(&db, &message).await.status, "failed");
        observe(
            &db,
            &bot,
            &[event("wamid.two", ReceiptStatus::Delivered, Utc::now())],
        )
        .await
        .unwrap();
        assert_eq!(summary(&db, &message).await.status, "delivered");
        let send = message.platform_send.as_mut().unwrap();
        send.complete = false;
        send.uncertain = true;
        send.expected_components = 3;
        send.failure_code = Some(10005);
        assert_eq!(summary(&db, &message).await.status, "partial");
        message
            .platform_send
            .as_mut()
            .unwrap()
            .component_ids
            .clear();
        assert_eq!(summary(&db, &message).await.status, "unknown");
        message.platform_send = None;
        let legacy = summary(&db, &message).await;
        assert_eq!(legacy.status, "legacy_final_only");
        assert!(!legacy.complete);
        assert!(legacy.components.is_empty());
        message.platform_message_id = None;
        assert_eq!(summary(&db, &message).await.status, "unknown");
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn receipt_bounds_logical_expiry_and_redaction() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_receipt_bounds").await;
        ensure_indexes(&db).await.unwrap();
        let bot = bot();
        let now = Utc::now();
        for id in [
            String::new(),
            "x".repeat(513),
            "has space".into(),
            "private\ntext".into(),
        ] {
            observe(&db, &bot, &[event(&id, ReceiptStatus::Read, now)])
                .await
                .unwrap();
        }
        for recipient in ["", "group:123", "123\n", &"1".repeat(33)] {
            let mut invalid = event("bad-recipient", ReceiptStatus::Read, now);
            invalid.recipient_id = recipient.into();
            observe(&db, &bot, &[invalid]).await.unwrap();
        }
        observe(
            &db,
            &bot,
            &[
                event("old", ReceiptStatus::Read, now - chrono::Duration::days(31)),
                event(
                    "future",
                    ReceiptStatus::Read,
                    now + chrono::Duration::days(2),
                ),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        observe(&db, &bot, &[event("wamid.one", ReceiptStatus::Read, now)])
            .await
            .unwrap();
        let message = row(&db, &bot, &["wamid.one"]).await;
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {},
                doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert_eq!(summary(&db, &message).await.status, "accepted");
        observe(&db, &bot, &[event("wamid.one", ReceiptStatus::Sent, now)])
            .await
            .unwrap();
        assert_eq!(summary(&db, &message).await.status, "sent");
        let receipt = db
            .collection::<DeliveryReceipt>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.times.read_at.is_none());
        assert!(!format!("{receipt:?}").contains("wamid.one"));
        db.drop().await.unwrap();
    }
}
