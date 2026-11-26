//! Channel-bot health: stable failure codes, the one recovery recommendation
//! shared by the health endpoints, the failure audit and the owner notice,
//! best-effort failure notifications, and throttled counters for webhook
//! deliveries that fail before a bot is authenticated.
//!
//! Diagnostics are metadata only: codes, timestamps and counts. Never store
//! message bodies, provider error prose with user content, or credentials.

pub mod answer;
pub mod discord;
pub(crate) mod nyxbot;

use super::channel_platform::platform_name;
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{ChannelBot, ChannelInboundOutcome};
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
    /// NyxID refused a native gateway command during intake validation (HTTP 400).
    pub const GATEWAY_INTAKE_REFUSED: &str = "intake_validation_refused";
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
    /// Meta delivered a signed relay message while native WhatsApp owned intake.
    pub const NATIVE_RELAY_CONFLICT: &str = "native_gateway_relay_conflict";
    /// Unexpected internal error.
    pub const INTERNAL: &str = "internal";
    pub const NYXBOT_ORG_RESTORATION_PENDING: &str = "nyxbot_org_restoration_pending";
    pub const NYXBOT_ORG_RESTORATION_WAITING_READINESS: &str =
        "nyxbot_org_restoration_waiting_readiness";
    pub const NYXBOT_ORG_RESTORATION_RETRYING_GATEWAY: &str =
        "nyxbot_org_restoration_retrying_gateway";
    pub const NYXBOT_ORG_ACCESS_LOST: &str = "nyxbot_org_access_lost";
}

/// Stable code for an error surfaced by a channel-platform call.
pub fn failure_code(error: &AppError) -> &'static str {
    use crate::services::channel_adapters::x::{X_FORBIDDEN_ACCESS, X_FORBIDDEN_SCOPES};
    match error {
        AppError::ChannelPlatformError(message) => {
            if message.starts_with("Meta send acceptance is unknown") {
                return "meta_send_uncertain";
            }
            if message == super::telegram_formatting::REJECTED {
                return "telegram_text_rejected";
            }
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
        AppError::ChannelConversationNotReachable(reason) if reason.starts_with("meta:") => {
            match reason.as_str() {
                "meta:window_closed" => "meta_window_closed",
                "meta:reply_token_expired" => "meta_reply_token_expired",
                "meta:inbox_control_lost" => "meta_inbox_control_lost",
                "meta:reconnect_required" => codes::RECONNECT_REQUIRED,
                "meta:platform_suspended" | "meta:inbox_suspended" => "meta_delivery_suspended",
                _ => "meta_authority_changed",
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

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct MissingGatewayEvent {
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub message_at: Option<DateTime<Utc>>,
}

/// Facts the recommendation depends on. Build with [`HealthFacts::for_bot`].
#[derive(Clone, Debug)]
pub struct HealthFacts<'a> {
    pub answer_delivery: Option<answer::Delivery>,
    pub org_guest_billing: Option<&'a str>,
    pub platform: &'a str,
    pub status: &'a str,
    pub webhook_registered: bool,
    pub failure_reason: Option<&'a str>,
    pub outbound_reason: Option<&'a str>,
    pub thread_reason: Option<&'a str>,
    pub inbound_outcome: Option<&'a ChannelInboundOutcome>,
    pub subscription_registered: Option<bool>,
    pub backoff_active: bool,
    /// The bot is delivered through the Agent Event Gateway.
    pub gateway: bool,
    pub gateway_not_received: Option<MissingGatewayEvent>,
    pub discord_intake: Option<discord::Intake>,
}

impl<'a> HealthFacts<'a> {
    pub fn for_bot(bot: &'a ChannelBot, now: DateTime<Utc>, gateway: bool) -> Self {
        Self {
            answer_delivery: None,
            org_guest_billing: None,
            platform: &bot.platform,
            inbound_outcome: bot.diagnostics.last_inbound_outcome.as_ref().filter(|o| {
                !(o.code.starts_with("telegram_group_") || o.code.starts_with("whatsapp_"))
                    || bot.diagnostics.last_inbound_at.is_none_or(|at| o.at >= at)
            }),
            status: &bot.status,
            webhook_registered: bot.webhook_registered,
            failure_reason: bot.diagnostics.failure_reason.as_deref(),
            outbound_reason: bot
                .diagnostics
                .last_outbound_error
                .as_ref()
                .and_then(|error| error.reason.as_deref()),
            thread_reason: bot
                .diagnostics
                .last_outbound_error
                .as_ref()
                .and_then(|error| error.thread_reason.as_deref()),
            subscription_registered: bot
                .diagnostics
                .subscription_state
                .as_ref()
                .map(|state| state.registered),
            backoff_active: bot.poll_backoff_until.is_some_and(|until| until > now),
            gateway,
            gateway_not_received: None,
            // Intake advice requires loaded flag, readiness and transport evidence.
            discord_intake: None,
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
    /// Stable cause selected by this recommendation, never inferred from text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

fn advice(action: &'static str, text: String) -> Recommendation {
    Recommendation {
        action,
        text,
        cause: None,
    }
}

pub(crate) async fn org_guest_billing(
    db: &mongodb::Database,
    org: &str,
) -> AppResult<Option<&'static str>> {
    match super::org_channel_service::budget::require_wallet(db, org).await {
        Ok(()) => Ok(None),
        Err(AppError::BillingNotConfigured(_)) => Ok(Some("org_wallet_not_provisioned")),
        Err(AppError::WalletSuspended) => Ok(Some("org_wallet_suspended")),
        Err(error) => Err(error),
    }
}

pub(crate) fn guest_billing_advice(reason: &str) -> Recommendation {
    advice("enable_billing", match reason {
        "org_wallet_suspended" => "Guest access is on, but guests cannot be served while the organization wallet is suspended.",
        _ => "Guest access is on, but guests cannot be served until the organization's billing wallet and subscription are set up.",
    }.into())
}

pub(crate) async fn load_org_guest_billing_many(
    db: &mongodb::Database,
    bindings: &[(String, String)],
) -> AppResult<std::collections::BTreeMap<String, &'static str>> {
    use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS};
    let mut result = std::collections::BTreeMap::new();
    if bindings.is_empty() {
        return Ok(result);
    }
    let orgs: Vec<_> = bindings.iter().map(|(_, org)| org).collect();
    let wallets: Vec<BillingWallet> = db
        .collection(WALLETS)
        .find(doc! {"owner_id":{"$in":orgs}})
        .await?
        .try_collect()
        .await?;
    for (id, org) in bindings {
        let wallet = wallets.iter().find(|wallet| &wallet.owner_id == org);
        let reason = match wallet {
            None => Some("org_wallet_not_provisioned"),
            Some(wallet)
                if wallet
                    .lago_subscription_id
                    .as_deref()
                    .is_none_or(str::is_empty) =>
            {
                Some("org_wallet_not_provisioned")
            }
            Some(wallet) if wallet.is_suspended() => Some("org_wallet_suspended"),
            Some(_) => None,
        };
        if let Some(reason) = reason {
            result.insert(id.clone(), reason);
        }
    }
    Ok(result)
}

fn for_code(code: &str, facts: &HealthFacts<'_>, outbound: bool) -> Option<Recommendation> {
    let name = platform_name(facts.platform);
    let subscription_action = if is_x(facts.platform) {
        "resubscribe"
    } else {
        "verify"
    };
    Some(match code {
        codes::NYXBOT_ORG_RESTORATION_PENDING => advice(
            "none",
            "NyxBot is restoring this organization connection automatically; it normally returns in under a minute. Members must verify their chat account again when it returns.".into(),
        ),
        codes::NYXBOT_ORG_RESTORATION_WAITING_READINESS => advice(
            "none",
            "NyxBot is waiting for required safety checks to become available before restoring this organization connection. NyxID will retry automatically; no administrator rights were lost.".into(),
        ),
        codes::NYXBOT_ORG_RESTORATION_RETRYING_GATEWAY => advice(
            "none",
            "NyxBot is retrying the channel connection. The retry schedule can take hours; no administrator rights were lost.".into(),
        ),
        codes::NYXBOT_ORG_ACCESS_LOST => advice(
            "reconnect",
            "This organization connection lost live administrator authority. An organization admin must reconnect it after restoring access.".into(),
        ),
        "meta_window_closed"=>advice("none","The Meta 24-hour reply window closed for that message. A newer customer message can authorize a new reply.".into()),
        "meta_reply_token_expired"=>advice("none","The NyxID reply token expired before the Meta window. Reply to a newer inbound message.".into()),
        "meta_send_uncertain"=>advice("none","Meta send acceptance is uncertain. Inspect the delivery journal and native inbox; do not automatically resend.".into()),
        "meta_inbox_control_lost" => if facts.platform == "instagram" {
            advice("reconnect", "Instagram reported that this app lost control of the conversation. Repair routing in Instagram and reconnect this inbox with fresh consent. An ordinary Verify preserves the recorded loss of control.".into())
        } else {
            advice("verify", "This app does not control the customer conversation. Repair routing in Meta and wait for authoritative thread ownership proof before replying.".into())
        },
        "meta_control_unavailable"=>advice("verify","Meta control ownership could not be read after bounded retries. Check Graph availability and Page routing, then verify this inbox; the message was not forwarded.".into()),
        "meta_delivery_suspended"=>advice("verify","Delivery is suspended. Restore the platform gate if needed, then verify and resume this inbox.".into()),
        "meta_authority_changed"=>advice("reconnect","The inbox binding or credential authority changed. Reconnect and answer a new customer message.".into()),
        "discord_event_webhooks_wrong_endpoint" => advice("verify", "Event Webhooks do not deliver ordinary bot chat messages. The Interactions Endpoint URL under General Information is optional and only needed for slash commands/components. Ordinary chat uses NyxBot's Discord gateway when enabled for your account; other relay bots need channel:discord-gateway. Enable Message Content Intent in Discord.".into()),
        "discord_gateway_intents_required" => advice("verify", "Enable Message Content Intent in Discord Developer Portal > Bot > Privileged Gateway Intents (verified apps need approval). NyxID retries automatically within five minutes.".into()),
        "discord_gateway_invalid_token" | "discord_gateway_identity_mismatch" => advice("reconnect", "Replace this Discord bot's token in NyxID with the token for the same bot. The Gateway session will restart automatically.".into()),
        "discord_gateway_capacity" => advice("contact_admin", "Discord ingress capacity is full on this replica. Another replica can claim this bot; add capacity if it keeps waiting.".into()),
        "discord_gateway_sharding_required" => advice("contact_admin", "This Discord bot requires sharding, which this bounded single-session ingress does not support.".into()),
        code if code.starts_with("discord_gateway_") => advice("verify", "Discord Gateway ingress is unavailable or reconnecting. NyxID retries automatically. Check the Gateway status, bot token, channel permissions and privileged Message Content Intent.".into()),
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
        codes::GATEWAY_INTAKE_REFUSED => advice("contact_admin",
            "NyxID refused a dispatch from the Agent Event Gateway. Ask a platform administrator to check the nyxbot_provider_refused audit and the conversation queue in gateway Activity.".into()),
        codes::NATIVE_RELAY_CONFLICT => advice(
            "verify",
            "Meta delivered a relay callback while native WhatsApp gateway intake was active. NyxID kept native intake authoritative; verify the dedicated app callback and gateway status.".into(),
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
        "telegram_text_rejected" => advice(
            "contact_admin",
            "Telegram rejected a reply part. The reply is incomplete; later parts were not sent. Read the saved answer in NyxID and ask an administrator to inspect the delivery receipt before requesting another send.".into(),
        ),
        codes::RECIPIENT_UNREACHABLE if outbound => advice(
            "none",
            "The last reply's recipient does not accept messages from this account.".into(),
        ),
        _ => return None,
    })
}

fn telegram_group_advice(outcome: &ChannelInboundOutcome) -> Option<Recommendation> {
    let (action, text) = match outcome.code.as_str() {
        "telegram_group_route_outside_source" => (
            "verify",
            "The selected route uses the gateway key, but the connected NyxBot only receives its registered route. Check the destination of this group’s route in Conversation Routes.",
        ),
        "telegram_group_other_agent" => (
            "verify",
            "The group message went to another agent key. Open Conversation Routes and check the assigned agent. Choose View messages to see its callback status.",
        ),
        "telegram_group_partition_missing" => (
            "gateway",
            "NyxBot could not find the gateway conversation for this group. Check this bot’s connection status under Channels in NyxBot. If it is active and the problem repeats, contact support.",
        ),
        "telegram_group_source_refused" | "telegram_group_provider_refused" => (
            "gateway",
            "NyxBot declined the gateway request before starting a turn. Check this bot’s connection status under Channels in NyxBot. If it is active and the problem repeats, contact support.",
        ),
        "telegram_group_callback_refused" => (
            "verify",
            "The receiver declined the group message. Under Conversation Routes, choose View messages on the group’s route to review the callback.",
        ),
        "telegram_group_authority_changed" | "telegram_group_callback_key_unavailable" => (
            "verify",
            "NyxID could not verify the route before sending the message. Check the active route and its assigned agent in Conversation Routes.",
        ),
        "telegram_group_owner_unavailable" => (
            "verify",
            "NyxID could not save the group message because its owner account was unavailable. If the account is active and this happens again, contact support.",
        ),
        "telegram_group_storage_unavailable" => (
            "verify",
            "NyxID could not save the group message. Send a new message. If it also fails, contact support.",
        ),
        "telegram_group_reply_token_unavailable" | "telegram_group_callback_failed" => (
            "verify",
            "NyxID could not deliver the group callback. Under Conversation Routes, choose View messages on the group’s route and check the callback status.",
        ),
        "telegram_group_processing_failed" => (
            "verify",
            "NyxBot could not process the group message. Send a new message. If it also fails, contact support.",
        ),
        "telegram_group_owner_absent" => (
            "verify",
            "This group is waiting for its verified owner or organization admin to speak. Address the bot in the group from the verified account.",
        ),
        "telegram_group_members_refused"
        | "telegram_group_sender_refused"
        | "telegram_group_policy_refused" => (
            "verify",
            "NyxBot declined this sender’s group message. In NyxBot, open Channels, choose this chat and check Who can talk.",
        ),
        "telegram_group_bot_budget" => (
            "verify",
            "The group reached its limit for bot-to-bot messages. Have a person address the bot before trying another handoff.",
        ),
        _ => return None,
    };
    let text = match &outcome.route_id {
        Some(route) => format!("{text} Selected route ID {route}."),
        None => text.to_owned(),
    };
    Some(advice(action, text))
}

pub(crate) const GATEWAY_NOT_RECEIVED_REASON: &str = "NyxID recorded a successful gateway callback and found no matching provider event when it checked after two minutes";

/// The single recovery recommendation used by health, audit and notices.
pub fn recommend(facts: &HealthFacts<'_>) -> Recommendation {
    recommend_with_gateway_failure(facts, None)
}

pub(crate) fn recommend_with_gateway_failure(
    facts: &HealthFacts<'_>,
    failure: Option<&str>,
) -> Recommendation {
    // A current failed setup prevents every new message. Old delivery errors
    // cannot explain that failure. Retain genuine bot credential faults first.
    if let Some(code) = failure.filter(|code| {
        !super::assistant_wake_service::text::gateway_error_guidance(code).is_empty()
    }) {
        if let Some(found) = facts
            .failure_reason
            .and_then(|reason| for_code(reason, facts, false))
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
        let guidance = super::assistant_wake_service::text::gateway_error_guidance(code);
        let mut recommendation = advice(
            match code {
                "gateway_not_authenticated" | "gateway_unavailable" => "reconnect",
                _ => "contact_admin",
            },
            format!(
                "{code}: Last connection attempt: {guidance} Retry after correcting the issue; health does not probe the gateway."
            ),
        );
        recommendation.cause = Some(code.to_owned());
        return recommendation;
    }
    if let Some(code) = facts.failure_reason
        && let Some(found) = for_code(code, facts, false)
        && matches!(
            code,
            codes::NYXBOT_ORG_RESTORATION_PENDING
                | codes::NYXBOT_ORG_RESTORATION_WAITING_READINESS
                | codes::NYXBOT_ORG_RESTORATION_RETRYING_GATEWAY
                | codes::NYXBOT_ORG_ACCESS_LOST
                | codes::GATEWAY_INTAKE_REFUSED
        )
    {
        return found;
    }
    if let Some(code) = facts
        .failure_reason
        .filter(|code| code.starts_with("discord_"))
        && let Some(found) = for_code(code, facts, false)
    {
        return found;
    }
    if let Some(advice) = facts.discord_intake.as_ref().and_then(discord::advice) {
        return advice;
    }
    if facts.failure_reason == Some(codes::NATIVE_RELAY_CONFLICT) {
        return for_code(codes::NATIVE_RELAY_CONFLICT, facts, false).expect("mapped code");
    }
    match facts.thread_reason {
        Some("thread_target_rejected") => return advice("reconnect",
            "The bot could not access the original thread. Check the chat and reconnect the channel if the bot has lost access.".into()),
        Some("thread_reply_failed") => return advice("verify",
            "The thread reply may be incomplete. Check the chat before asking again.".into()),
        Some("thread_delayed_reply_failed") => return advice("none",
            "A delayed thread update may be missing. Check the chat for the update.".into()),
        Some("thread_reply_reference_expired") => return advice("none",
            "A delayed thread update could not be sent because its gateway reply reference expired after 29 minutes. No owner action is needed; a new message starts a new reply window.".into()),
        _ => {},
    }
    match facts.outbound_reason {
        Some(reason @ ("gateway_reply_no_response" | "gateway_reply_no_response_pending"
            | "gateway_reply_server_error" | "gateway_reply_server_error_pending"
            | "gateway_reply_unreadable_response" | "gateway_reply_unreadable_response_pending"
            | "gateway_reply_interrupted" | "gateway_reply_interrupted_pending")) => {
            let observed = match reason {
                "gateway_reply_no_response" | "gateway_reply_no_response_pending" => "No response came back from the gateway.",
                "gateway_reply_server_error" | "gateway_reply_server_error_pending" => "The gateway returned a server error.",
                "gateway_reply_unreadable_response" | "gateway_reply_unreadable_response_pending" => "The gateway response could not be read.",
                _ => "NyxID stopped before recording the gateway response.",
            };
            let progress = match reason {
                "gateway_reply_no_response_pending" | "gateway_reply_server_error_pending"
                | "gateway_reply_unreadable_response_pending" | "gateway_reply_interrupted_pending" => "NyxBot is still trying with the same reply.",
                _ => "Delivery could not be confirmed before checking stopped.",
            };
            return advice("gateway", format!("{observed} {progress} Check the chat before asking again."));
        }
        Some("gateway_reply_unconfirmed") => return advice("gateway", "The gateway accepted this reply, but delivery was not confirmed. Check the chat before asking again.".into()),
        Some("gateway_reply_unavailable") => return advice("gateway", "The gateway was temporarily unavailable. This reply has no confirmed delivery. Check the chat before asking again.".into()),
        Some("gateway_reply_rejected") => return advice("gateway", "The gateway refused this reply. Check the chat before asking again.".into()),
        Some("gateway_reply_unknown") => return advice("gateway", "Delivery of this reply could not be confirmed. Check the chat before asking again.".into()),
        _ => {}
    }
    if facts.gateway
        && facts.org_guest_billing.is_none()
        && super::channel_platform::canonical_platform(facts.platform) == "telegram"
        && let Some(recommendation) = facts.inbound_outcome.and_then(telegram_group_advice)
    {
        return recommendation;
    }
    if facts.gateway
        && facts.platform == "whatsapp"
        && facts.org_guest_billing.is_none()
        && let Some(outcome) = facts.inbound_outcome
        && let Some(advice) = super::channel_inbound_diagnostics::recommendation(&outcome.code)
    {
        return advice;
    }
    // Preserve existing gateway advice when no org-guest billing fact was added.
    if facts.gateway && facts.org_guest_billing.is_none() {
        if let Some(missing) = facts.gateway_not_received {
            let message_time = missing
                .message_at
                .map(|at| {
                    format!(
                        " The affected message arrived at {} UTC.",
                        at.format("%Y-%m-%d %H:%M:%S%.3f")
                    )
                })
                .unwrap_or_default();
            return advice(
                "contact_admin",
                format!(
                    "{GATEWAY_NOT_RECEIVED_REASON}.{message_time} Ask your administrator to trace the delivery in gateway Activity using the bot ID and message time."
                ),
            );
        }
        if facts.answer_delivery.is_none() {
            return advice("gateway", "This bot is delivered through the Agent Event Gateway; check the NyxBot channel's status there.".into());
        }
    }
    if facts.failure_reason == Some(codes::BILLING_SUSPENDED) && is_x(facts.platform) {
        return super::channel_inbound_diagnostics::recommendation("x_billing_refused")
            .expect("mapped code");
    }
    if let Some(found) = facts
        .failure_reason
        .and_then(|code| for_code(code, facts, false))
    {
        return found;
    }
    let inbound_code = facts.inbound_outcome.map(|o| o.code.as_str());
    if inbound_code == Some("x_bot_inactive") {
        return super::channel_inbound_diagnostics::recommendation("x_bot_inactive")
            .expect("mapped code");
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
    if let Some(found) = inbound_code.and_then(super::channel_inbound_diagnostics::recommendation) {
        return found;
    }
    if let Some(reason) = facts.org_guest_billing {
        return guest_billing_advice(reason);
    }
    if let Some(answer) = facts.answer_delivery {
        return answer.recommendation();
    }
    if facts.gateway {
        return advice(
            "gateway",
            "This bot is delivered through the Agent Event Gateway; check the NyxBot channel's status there.".into(),
        );
    }
    if is_x(facts.platform) {
        return advice(
            "none",
            super::channel_inbound_diagnostics::CAPABILITY.into(),
        );
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

/// Record the signed callback even though native ownership precludes relay admission.
pub(crate) async fn record_native_relay_conflict(
    db: &mongodb::Database,
    bot: &ChannelBot,
) -> AppResult<()> {
    let code = codes::NATIVE_RELAY_CONFLICT;
    let now = bson::DateTime::now();
    let previous = db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":&bot.id,"user_id":&bot.user_id,"is_active":true,"platform":"whatsapp",
            "platform_bot_id": &bot.platform_bot_id,
            "$expr":{"$eq":[{"$ifNull":["$ownership_version",0_i64]},bot.ownership_version]}},
            bson::doc! {"$set": {
                "diagnostics.last_webhook_at":now, "diagnostics.failure_reason":code,
                "diagnostics.last_webhook_error":{"code":code,"reason":code,"at":now},
            }},
        )
        .projection(bson::doc! {"diagnostics.failure_reason":1})
        .await?;
    if let Some(previous) = previous
        && previous
            .get_document("diagnostics")
            .ok()
            .and_then(|d| d.get_str("failure_reason").ok())
            != Some(code)
    {
        let mut facts = HealthFacts::for_bot(bot, Utc::now(), true);
        facts.failure_reason = Some(code);
        notify_failure(db, bot, &recommend(&facts));
    }
    Ok(())
}

/// Best-effort failure notice to the bot owner, or to every admin of an
/// org-owned bot. Called once per failure transition; never fails the caller.
pub fn notify_failure(db: &mongodb::Database, bot: &ChannelBot, recommendation: &Recommendation) {
    notify_bot(
        db,
        bot,
        recommendation,
        crate::services::notification_service::ChannelBotNoticeKind::Failure,
    );
}

pub(crate) fn notify_guest_defaults(db: &mongodb::Database, bot: &ChannelBot, description: &str) {
    notify_bot(
        db,
        bot,
        &advice("review_guest_access", description.into()),
        crate::services::notification_service::ChannelBotNoticeKind::GuestDefaults,
    );
}

fn notify_bot(
    db: &mongodb::Database,
    bot: &ChannelBot,
    recommendation: &Recommendation,
    kind: crate::services::notification_service::ChannelBotNoticeKind,
) {
    let context = crate::services::notification_service::ChannelBotFailedNotificationContext {
        bot_id: bot.id.clone(),
        bot_label: bot.label.clone(),
        platform_name: platform_name(&bot.platform),
        recommended_action: recommendation.action.to_string(),
        recommended_action_text: recommendation.text.clone(),
        notice_kind: kind.clone(),
        notice_title: (kind
            == crate::services::notification_service::ChannelBotNoticeKind::GuestDefaults)
            .then(|| "Organization bot guest access".to_owned()),
        notice_body: (kind
            == crate::services::notification_service::ChannelBotNoticeKind::GuestDefaults)
            .then(|| format!("{}: {}", bot.label, recommendation.text)),
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
    crate::services::shutdown::process().spawn(async move {
        let mut recipients =
            match crate::services::org_service::list_admin_user_ids(&db, &owner).await {
                Ok(recipients) => recipients,
                Err(error) => {
                    tracing::warn!(%error, "channel bot notice could not resolve org admins");
                    Vec::new()
                }
            };
        if kind == crate::services::notification_service::ChannelBotNoticeKind::GuestDefaults {
            let active = db
                .collection::<bson::Document>("users")
                .find(doc! {"_id":{"$in":&recipients},"is_active":true})
                .projection(doc! {"_id":1})
                .await;
            let Ok(active) = active else {
                return;
            };
            let Ok(active): Result<Vec<bson::Document>, _> = active.try_collect().await else {
                return;
            };
            recipients = active
                .iter()
                .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
                .collect();
        } else if recipients.is_empty() {
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
                tracing::warn!(%error, bot_id = %context.bot_id, "channel bot notice failed");
            }
        }
    });
}

/// One transition notice through the same channel-health notifier used for
/// failures. The caller supplies the linking admin because an org bot's
/// platform owner is a separate org account.
pub fn notify_restored(
    db: &mongodb::Database,
    bot: &ChannelBot,
    linking_admin: &str,
    guest_enabled: bool,
) {
    let guest = if guest_enabled { "on" } else { "off" };
    let context = crate::services::notification_service::ChannelBotFailedNotificationContext {
        bot_id: bot.id.clone(),
        bot_label: bot.label.clone(),
        platform_name: platform_name(&bot.platform),
        recommended_action: "restored".into(),
        recommended_action_text: "Members must verify their chat account again before chatting."
            .into(),
        notice_kind: crate::services::notification_service::ChannelBotNoticeKind::Restored,
        notice_title: Some(format!(
            "{} channel connected again",
            platform_name(&bot.platform)
        )),
        notice_body: Some(format!(
            "{} is connected again. Members must verify their chat account again in NyxID. Guest access is {guest} as before. Open the channel details for the current status.",
            bot.label
        )),
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
    let linking_admin = linking_admin.to_owned();
    let owner = bot.user_id.clone();
    crate::services::shutdown::process().spawn(async move {
        let mut recipients = match crate::services::org_service::list_admin_user_ids(&db, &owner)
            .await
        {
            Ok(recipients) => recipients,
            Err(error) => {
                tracing::warn!(%error, "channel bot restoration notice could not resolve org admins");
                Vec::new()
            }
        };
        recipients.push(linking_admin);
        if recipients.is_empty() {
            recipients.push(owner);
        }
        recipients.sort();
        recipients.dedup();
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
                tracing::warn!(%error, bot_id = %context.bot_id, "channel bot restoration notice failed");
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

/// Bounded read projection shared by NyxBot discovery and diagnostic surfaces.
/// These are observed facts, not a provider probe or a setup-completion promise.
pub async fn setup_evidence(
    db: &mongodb::Database,
    bot: &ChannelBot,
    gateway: bool,
) -> AppResult<serde_json::Value> {
    use serde_json::json;
    let mut facts = HealthFacts::for_bot(bot, Utc::now(), gateway);
    if let Some(intake) = discord::load(db, std::slice::from_ref(bot))
        .await?
        .remove(&bot.id)
    {
        facts.discord_intake = Some(intake);
    }
    let row = if matches!(bot.platform.as_str(), "telegram" | "telegram-new") {
        nyxbot::latest_nyxbot_rows(db, std::slice::from_ref(bot))
            .await?
            .remove(&bot.id)
    } else {
        None
    };
    facts.gateway = nyxbot::current_gateway(bot, row.as_ref(), gateway);
    let action = recommend_with_gateway_failure(
        &facts,
        row.as_ref()
            .and_then(|row| nyxbot::gateway_failure(bot, row)),
    );
    let d = &bot.diagnostics;
    let mut value = json!({
        "contract_version": 1, "observed_at": Utc::now().to_rfc3339(),
        "bot_id": bot.id, "platform": bot.platform, "platform_bot_username": bot.platform_bot_username,
        "credential_source": bot.credential_source, "credential_connection_id": bot.connection_id,
        "provider_slug": if is_x(&bot.platform) { Some("twitter") } else { None },
        "authorization": "not_probed", "mode": facts.mode(), "gateway": facts.gateway,
        "webhook_registered": bot.webhook_registered,
        "subscription": d.subscription_state.as_ref().map(|s| json!({
            "registered": s.registered, "checked_at": s.checked_at.to_rfc3339(), "last_error": s.last_error,
        })),
        "last_authenticated_inbound_at": d.last_inbound_at.map(|t| t.to_rfc3339()),
        "last_authenticated_inbound_source": d.last_inbound_source,
        "last_outbound_attempt_at": d.last_outbound_at.map(|t| t.to_rfc3339()),
        "last_outbound_error": d.last_outbound_error.as_ref().map(|e| json!({"code": e.code, "reason": e.reason, "thread_reason": e.thread_reason, "at": e.at.to_rfc3339()})),
        "last_outbound_notice": d.last_outbound_notice.as_ref().map(|e| json!({"code": e.code, "reason": e.reason, "thread_reason": e.thread_reason, "at": e.at.to_rfc3339()})),
        "failure_reason": d.failure_reason,
        "last_inbound_outcome": d.last_inbound_outcome.as_ref().map(|o| json!({"code":o.code,"at":o.at.to_rfc3339()})),
        "inbound_outcomes": d.inbound_outcomes.iter().map(|(code,c)| (code.clone(),json!({"count":c.count,"at":c.at.to_rfc3339()}))).collect::<serde_json::Map<_,_>>(),
        "receive_limitations": if is_x(&bot.platform) {Some(super::channel_inbound_diagnostics::CAPABILITY)} else {None},
        "recommended_action": action.action, "recommended_action_text": action.text, "recommendation_cause":action.cause,
        "automatic_reply": "not_verified",
        "evidence_note": "Configured credentials, a registered subscription and historical traffic are separate evidence. No Verify, Resubscribe or test send was run. Gateway support and end-to-end automatic replies need their own evidence.",
    });
    if let Some(intake) = &facts.discord_intake {
        value["discord_intake"] = json!(intake);
    }
    if let Some(row) = row {
        value["nyxbot_status"] = json!(row.status);
        value["nyxbot_last_error"] = json!(nyxbot::gateway_failure(bot, &row));
        if nyxbot::gateway_failure(bot, &row).is_some() {
            value["nyxbot_setup_diagnostic"] = json!(
                row.setup_diagnostic
                    .as_ref()
                    .and_then(nyxbot::SetupDiagnostic::from_document)
            );
        }
    }
    Ok(value)
}
