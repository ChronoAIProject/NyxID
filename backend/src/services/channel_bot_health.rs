//! Channel-bot health: stable failure codes, the one recovery recommendation
//! shared by the health endpoints, the failure audit and the owner notice,
//! best-effort failure notifications, and throttled counters for webhook
//! deliveries that fail before a bot is authenticated.
//!
//! Diagnostics are metadata only: codes, timestamps and counts. Never store
//! message bodies, provider error prose with user content, or credentials.

use super::channel_platform::platform_name;
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::push_service::{ApnsAuth, FcmAuth};

/// Stable `diagnostics.failure_reason` and `last_outbound_error.reason` codes.
pub mod codes {
    /// The account connection was revoked, expired or lacks a scope locally.
    pub const RECONNECT_REQUIRED: &str = "reconnect_required";
    /// The platform rejected the account token (HTTP 401).
    pub const AUTH_REVOKED: &str = "auth_revoked";
    /// The platform reports a missing OAuth scope (HTTP 403).
    pub const SCOPES_MISSING: &str = "scopes_missing";
    /// The platform app's access level or enrollment refuses the endpoint (HTTP 403).
    pub const APP_ACCESS_DENIED: &str = "app_access_denied";
    /// Any other platform refusal (HTTP 403).
    pub const ACCESS_DENIED: &str = "access_denied";
    /// The platform app's own API credits are exhausted (HTTP 402).
    pub const PROVIDER_CREDITS: &str = "provider_credits";
    /// The platform rate limited NyxID (HTTP 429).
    pub const RATE_LIMITED: &str = "rate_limited";
    /// The platform is unavailable (HTTP 5xx).
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    /// Any other platform error.
    pub const PROVIDER_ERROR: &str = "provider_error";
    /// A reply's recipient cannot receive messages from this account.
    pub const RECIPIENT_UNREACHABLE: &str = "recipient_unreachable";
    /// NyxID billing (wallet, price or Lago) stopped the channel.
    pub const BILLING_SUSPENDED: &str = "billing_suspended";
    /// Webhook delivery is required (paid channels cannot poll).
    pub const WEBHOOK_REQUIRED: &str = "webhook_required";
    /// Webhook subscription setup failed.
    pub const WEBHOOK_SETUP_FAILED: &str = "webhook_setup_failed";
    /// The platform-level webhook credentials are not configured.
    pub const WEBHOOK_UNCONFIGURED: &str = "webhook_unconfigured";
    /// The live check found no valid webhook for NyxID's callback URL.
    pub const WEBHOOK_MISSING: &str = "webhook_missing";
    /// The live check found no subscription for this bot's account.
    pub const SUBSCRIPTION_MISSING: &str = "subscription_missing";
    /// The post-write live subscription read could not be completed.
    pub const SUBSCRIPTION_CHECK_FAILED: &str = "subscription_check_failed";
    /// An authenticated delivery arrived for a bot whose subscription is not recorded.
    pub const UNREGISTERED: &str = "unregistered";
    /// Unexpected internal error.
    pub const INTERNAL: &str = "internal";
}

/// Stable code for an error surfaced by a channel-platform call.
pub fn failure_code(error: &AppError) -> &'static str {
    use crate::services::channel_adapters::x::{X_FORBIDDEN_ACCESS, X_FORBIDDEN_SCOPES};
    match error {
        AppError::ChannelPlatformError(message) => {
            if message.contains(X_FORBIDDEN_SCOPES) {
                return codes::SCOPES_MISSING;
            }
            if message.contains(X_FORBIDDEN_ACCESS) {
                return codes::APP_ACCESS_DENIED;
            }
            match http_status(message) {
                Some(401) => codes::AUTH_REVOKED,
                Some(402) => codes::PROVIDER_CREDITS,
                Some(403) => codes::ACCESS_DENIED,
                Some(429) => codes::RATE_LIMITED,
                Some(500..=599) => codes::PROVIDER_UNAVAILABLE,
                _ => codes::PROVIDER_ERROR,
            }
        }
        AppError::ChannelConversationNotReachable(_) => codes::RECIPIENT_UNREACHABLE,
        AppError::InsufficientCredits
        | AppError::BillingNotConfigured(_)
        | AppError::BillingProviderUnavailable(_) => codes::BILLING_SUSPENDED,
        AppError::ValidationError(_) => codes::RECONNECT_REQUIRED,
        _ => codes::INTERNAL,
    }
}

/// Status suffix written by platform adapters: `... (HTTP 429)`.
fn http_status(message: &str) -> Option<u16> {
    let start = message.rfind("(HTTP ")? + "(HTTP ".len();
    message[start..]
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Facts the recommendation depends on. Build with [`HealthFacts::for_bot`].
#[derive(Clone, Debug)]
pub struct HealthFacts<'a> {
    pub platform: &'a str,
    pub status: &'a str,
    pub webhook_registered: bool,
    pub failure_reason: Option<&'a str>,
    pub outbound_reason: Option<&'a str>,
    pub subscription_registered: Option<bool>,
    pub backoff_active: bool,
    /// The bot is delivered through the Agent Event Gateway.
    pub gateway: bool,
}

impl<'a> HealthFacts<'a> {
    pub fn for_bot(bot: &'a ChannelBot, now: DateTime<Utc>, gateway: bool) -> Self {
        Self {
            platform: &bot.platform,
            status: &bot.status,
            webhook_registered: bot.webhook_registered,
            failure_reason: bot.diagnostics.failure_reason.as_deref(),
            outbound_reason: bot
                .diagnostics
                .last_outbound_error
                .as_ref()
                .and_then(|error| error.reason.as_deref()),
            subscription_registered: bot
                .diagnostics
                .subscription_state
                .as_ref()
                .map(|state| state.registered),
            backoff_active: bot.poll_backoff_until.is_some_and(|until| until > now),
            gateway,
        }
    }

    /// X polls only while its webhook subscription is not registered.
    pub fn mode(&self) -> &'static str {
        if is_x(self.platform) && !self.webhook_registered {
            "poll"
        } else {
            "webhook"
        }
    }
}

fn is_x(platform: &str) -> bool {
    matches!(platform, "x" | "twitter")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Recommendation {
    /// `none`, `reconnect`, `resubscribe`, `verify`, `wait_rate_limit`,
    /// `enable_billing`, `contact_admin` or `gateway`.
    pub action: &'static str,
    pub text: String,
}

fn advice(action: &'static str, text: String) -> Recommendation {
    Recommendation { action, text }
}

fn for_code(code: &str, facts: &HealthFacts<'_>, outbound: bool) -> Option<Recommendation> {
    let name = platform_name(facts.platform);
    let subscription_action = if is_x(facts.platform) {
        "resubscribe"
    } else {
        "verify"
    };
    Some(match code {
        codes::BILLING_SUSPENDED => advice(
            "enable_billing",
            "NyxID billing stopped this channel. Restore credits or the service's billing configuration, then select Verify.".into(),
        ),
        codes::RECONNECT_REQUIRED | codes::AUTH_REVOKED | codes::SCOPES_MISSING | codes::ACCESS_DENIED => advice(
            "reconnect",
            format!("Reconnect the {name} account and grant every requested permission, then select Verify."),
        ),
        codes::APP_ACCESS_DENIED => advice(
            "contact_admin",
            format!("The {name} app's access level does not allow this. An administrator must upgrade the {name} app."),
        ),
        codes::PROVIDER_CREDITS => advice(
            "contact_admin",
            format!("The {name} app has run out of {name} API credits. An administrator must restore them; this is separate from NyxID credits."),
        ),
        codes::WEBHOOK_UNCONFIGURED => advice(
            "contact_admin",
            format!("{name} webhook credentials are not configured. An administrator must add them in Admin > Platform Credentials."),
        ),
        codes::WEBHOOK_REQUIRED
        | codes::WEBHOOK_SETUP_FAILED
        | codes::WEBHOOK_MISSING
        | codes::SUBSCRIPTION_MISSING
        | codes::UNREGISTERED => advice(
            subscription_action,
            format!("The {name} webhook subscription is missing or incomplete. Register it again."),
        ),
        codes::RATE_LIMITED if outbound => advice(
            "wait_rate_limit",
            format!("{name} rate-limited the last reply. Later replies resume automatically."),
        ),
        codes::RATE_LIMITED if facts.backoff_active => advice(
            "wait_rate_limit",
            format!("{name} rate limiting is active. NyxID resumes when the backoff expires."),
        ),
        codes::PROVIDER_UNAVAILABLE => advice(
            "verify",
            format!("{name} is unavailable. NyxID retries automatically; select Verify once it recovers."),
        ),
        codes::RECIPIENT_UNREACHABLE if outbound => advice(
            "none",
            "The last reply's recipient does not accept messages from this account.".into(),
        ),
        _ => return None,
    })
}

/// The single recovery recommendation used by health, audit and notices.
pub fn recommend(facts: &HealthFacts<'_>) -> Recommendation {
    if facts.gateway {
        return advice(
            "gateway",
            "This bot is delivered through the Agent Event Gateway; check the NyxBot channel's status there.".into(),
        );
    }
    if let Some(found) = facts
        .failure_reason
        .and_then(|code| for_code(code, facts, false))
    {
        return found;
    }
    if facts.status == "failed" {
        return advice(
            "reconnect",
            format!(
                "Reconnect the {} account, then select Verify.",
                platform_name(facts.platform)
            ),
        );
    }
    if facts.subscription_registered == Some(false) {
        return for_code(codes::SUBSCRIPTION_MISSING, facts, false).expect("mapped code");
    }
    if let Some(found) = facts
        .outbound_reason
        .and_then(|code| for_code(code, facts, true))
    {
        return found;
    }
    if facts.backoff_active {
        return for_code(codes::RATE_LIMITED, facts, false).expect("mapped code");
    }
    advice("none", "Healthy.".into())
}

/// Delivers failure notices. Installed once at startup; tests record per bot.
#[derive(Clone)]
pub struct ChannelBotFailureNotifier {
    config: Arc<AppConfig>,
    http_client: reqwest::Client,
    fcm_auth: Option<Arc<FcmAuth>>,
    apns_auth: Option<Arc<ApnsAuth>>,
}

impl ChannelBotFailureNotifier {
    pub fn new(
        config: Arc<AppConfig>,
        http_client: reqwest::Client,
        fcm_auth: Option<Arc<FcmAuth>>,
        apns_auth: Option<Arc<ApnsAuth>>,
    ) -> Self {
        Self {
            config,
            http_client,
            fcm_auth,
            apns_auth,
        }
    }
}

static NOTIFIER: OnceLock<ChannelBotFailureNotifier> = OnceLock::new();

pub fn install_failure_notifier(notifier: ChannelBotFailureNotifier) {
    let _ = NOTIFIER.set(notifier);
}

#[cfg(test)]
pub(crate) type RecordedNotice = (
    String,
    crate::services::notification_service::ChannelBotFailedNotificationContext,
);

#[cfg(test)]
pub(crate) static TEST_NOTICES: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::mpsc::UnboundedSender<RecordedNotice>>,
    >,
> = std::sync::LazyLock::new(Default::default);

/// Best-effort failure notice to the bot owner, or to every admin of an
/// org-owned bot. Called once per failure transition; never fails the caller.
pub fn notify_failure(db: &mongodb::Database, bot: &ChannelBot, recommendation: &Recommendation) {
    let context = crate::services::notification_service::ChannelBotFailedNotificationContext {
        bot_id: bot.id.clone(),
        bot_label: bot.label.clone(),
        platform_name: platform_name(&bot.platform),
        recommended_action: recommendation.action.to_string(),
        recommended_action_text: recommendation.text.clone(),
    };
    #[cfg(test)]
    let recorder = TEST_NOTICES.lock().unwrap().get(&bot.id).cloned();
    #[cfg(not(test))]
    let recorder: Option<()> = None;
    let notifier = NOTIFIER.get().cloned();
    if recorder.is_none() && notifier.is_none() {
        return;
    }
    let db = db.clone();
    let owner = bot.user_id.clone();
    tokio::spawn(async move {
        let mut recipients = match crate::services::org_service::list_admin_user_ids(&db, &owner)
            .await
        {
            Ok(recipients) => recipients,
            Err(error) => {
                tracing::warn!(%error, "channel bot failure notice could not resolve org admins");
                Vec::new()
            }
        };
        if recipients.is_empty() {
            recipients.push(owner);
        }
        for recipient in recipients {
            #[cfg(test)]
            if let Some(recorder) = &recorder {
                let _ = recorder.send((recipient.clone(), context.clone()));
                continue;
            }
            let Some(notifier) = &notifier else { continue };
            if let Err(error) =
                crate::services::notification_service::send_channel_bot_failed_notification(
                    &db,
                    &notifier.config,
                    &notifier.http_client,
                    notifier.fcm_auth.as_deref(),
                    notifier.apns_auth.as_deref(),
                    &recipient,
                    &context,
                )
                .await
            {
                tracing::warn!(%error, bot_id = %context.bot_id, "channel bot failure notice failed");
            }
        }
    });
}

pub const REJECTIONS_COLLECTION: &str = "channel_webhook_rejections";

/// Webhook deliveries refused before any bot was authenticated, counted per
/// platform and reason. Rows are counters only and never name a bot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookRejectionCounter {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform: String,
    pub reason: String,
    pub count: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_at: DateTime<Utc>,
}

/// Pending increments per (platform, reason) and the last flush time.
static PENDING_REJECTIONS: std::sync::LazyLock<DashMap<String, (i64, std::time::Instant)>> =
    std::sync::LazyLock::new(DashMap::new);

/// At most one counter write per key per interval per replica.
const REJECTION_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Count an unauthenticated rejection. Writes are throttled so a flood of
/// forged deliveries cannot amplify into database load; increments between
/// flushes are carried to the next write.
pub async fn record_webhook_rejection(db: &mongodb::Database, platform: &str, reason: &str) {
    let key = format!("{platform}:{reason}");
    let now = std::time::Instant::now();
    let flush = {
        let mut entry = PENDING_REJECTIONS
            .entry(format!("{}\0{key}", db.name()))
            .or_insert((0, now.checked_sub(REJECTION_FLUSH_INTERVAL).unwrap_or(now)));
        entry.0 += 1;
        if now.duration_since(entry.1) >= REJECTION_FLUSH_INTERVAL {
            let count = entry.0;
            *entry = (0, now);
            Some(count)
        } else {
            None
        }
    };
    let Some(count) = flush else { return };
    if let Err(error) = db
        .collection::<bson::Document>(REJECTIONS_COLLECTION)
        .update_one(
            doc! {"_id": &key},
            doc! {
                "$inc": {"count": count},
                "$set": {"platform": platform, "reason": reason, "last_at": bson::DateTime::now()},
            },
        )
        .upsert(true)
        .await
    {
        tracing::warn!(%error, platform, reason, "webhook rejection counter write failed");
    }
}

pub async fn list_webhook_rejections(
    db: &mongodb::Database,
) -> AppResult<Vec<WebhookRejectionCounter>> {
    Ok(db
        .collection::<WebhookRejectionCounter>(REJECTIONS_COLLECTION)
        .find(doc! {})
        .sort(doc! {"_id": 1})
        .await?
        .try_collect()
        .await?)
}

#[cfg(test)]
pub(crate) fn reset_rejection_throttle() {
    PENDING_REJECTIONS.clear();
}

#[cfg(test)]
#[path = "channel_bot_health_tests.rs"]
mod tests;
