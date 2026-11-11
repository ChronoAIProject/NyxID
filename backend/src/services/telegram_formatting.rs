//! Definite formatting refusals may fall back once under the SAME send claim.
//! Receipt scopes nest (a proactive text part also has a relay receipt).
use crate::errors::{AppError, AppResult};
use bson::doc;

pub const FALLBACK: &str = "telegram_formatting_fallback";
pub const REJECTED: &str = "Telegram send failed: telegram_text_rejected";

#[derive(Clone)]
pub struct Receipt {
    pub collection: &'static str,
    pub filter: bson::Document,
    pub field: String,
    pub plain: bool,
}

tokio::task_local! {
    static RECEIPTS: (mongodb::Database, Vec<Receipt>, Option<String>);
}

pub async fn scope<F: std::future::Future>(
    db: &mongodb::Database,
    receipt: Receipt,
    bot: Option<&str>,
    future: F,
) -> F::Output {
    let mut receipts = RECEIPTS.try_with(|(_, r, _)| r.clone()).unwrap_or_default();
    receipts.push(receipt);
    let bot = bot
        .map(str::to_owned)
        .or_else(|| RECEIPTS.try_with(|(_, _, b)| b.clone()).ok().flatten());
    RECEIPTS.scope((db.clone(), receipts, bot), future).await
}

pub fn already_fell_back() -> bool {
    RECEIPTS
        .try_with(|(_, receipts, _)| receipts.iter().any(|receipt| receipt.plain))
        .unwrap_or(false)
}

pub fn parse_refusal(status: reqwest::StatusCode, value: &serde_json::Value) -> bool {
    status == reqwest::StatusCode::BAD_REQUEST
        && value["ok"] == false
        && value["error_code"] == 400
        && value["description"].as_str().is_some_and(|s| {
            let s = s.to_ascii_lowercase();
            s.contains("can't parse entities")
                || s.contains("can't find end of the entity")
                || s.contains("entity offset")
                || s.contains("entity length")
        })
}

/// Persist before retrying; a failed write leaves the original sending fence.
/// No provider description, message text, token or URL crosses this boundary.
pub async fn record_fallback() -> AppResult<()> {
    if let Ok((db, receipts, bot)) = RECEIPTS.try_with(Clone::clone) {
        for receipt in receipts {
            let changed = db
                .collection::<bson::Document>(receipt.collection)
                .update_one(receipt.filter, doc! {"$set": {receipt.field: FALLBACK}})
                .await?;
            if changed.matched_count != 1 {
                return Err(super::channel_reply_delivery::unconfirmed());
            }
        }
        if let Some(bot) = bot {
            // A notice is independent of last_outbound_error: successful fallback
            // clears the error but must not erase evidence of degraded formatting.
            db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
                doc! {"_id": bot},
                doc! {"$set": {"diagnostics.last_outbound_notice": {"code":"10005", "reason":FALLBACK, "at":bson::DateTime::now()}}},
            ).await?;
        }
    }
    tracing::warn!(
        code = FALLBACK,
        "Telegram formatting rejected; sending the same display text without entities"
    );
    Ok(())
}

pub fn definite_refusal(error: &AppError) -> bool {
    matches!(
        error,
        AppError::ChannelConversationNotReachable(_) | AppError::RateLimitedWithRetry { .. }
    ) || matches!(error, AppError::ChannelPlatformError(message) if message.starts_with("Telegram send failed:"))
}
