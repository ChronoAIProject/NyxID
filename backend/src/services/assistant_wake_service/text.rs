//! Shared fixed wording for live and recovered setup/delivery notices.
pub fn platform_name(platform: &str) -> &str {
    match platform {
        "telegram" | "telegram-new" => "Telegram",
        "discord" => "Discord",
        "slack" => "Slack",
        "lark" => "Lark",
        "feishu" => "Feishu",
        "whatsapp" => "WhatsApp",
        other => other,
    }
}

pub fn failure_reason(code: &str, transport: &str) -> String {
    use crate::services::channel_bot_health::discord;
    let intake = match code {
        discord::NATIVE_UNCONFIRMED => Some(
            "no authenticated native event is confirmed for this hand-over; selection does not prove a connected session",
        ),
        discord::RELAY_STOPPED => Some("NyxID's Discord relay session is stopped"),
        discord::RELAY_BLOCKED => Some(
            "NyxID's Discord relay intake is blocked; check channel health for the setup, release or readiness reason",
        ),
        discord::RELAY_UNCONFIRMED => Some("NyxID's Discord relay connection is not confirmed"),
        _ => None,
    };
    if let Some(reason) = intake {
        return reason.into();
    }
    let receiver = if transport == "gateway" {
        "the Agent Event Gateway"
    } else {
        "NyxID's relay"
    };
    if code == "media_interrupted" {
        return "a photo or file was accepted but interrupted before processing".into();
    }
    let media_reason = match code {
        "media_too_large" => Some("a photo or file exceeded the size limit"),
        "media_source_unavailable" => {
            Some("a photo or file source was unavailable or no longer authorized")
        }
        "media_unsupported_or_invalid" => {
            Some("a photo or file had an unsupported or invalid format")
        }
        "media_rate_limited" => {
            Some("a photo or file could not be processed because the upload rate limit was reached")
        }
        "media_fetch_or_store_failed" => Some("a photo or file could not be downloaded or stored"),
        "media_timeout" => Some("photo or file processing timed out"),
        "media_stopped" => Some("the turn stopped before a photo or file was processed"),
        "media_expired" => Some("a photo or file expired in the queue before processing"),
        "media_failed" => Some("a photo or file could not be processed"),
        _ => None,
    };
    if let Some(reason) = media_reason {
        return reason.into();
    }
    if code == "not_received" {
        return crate::services::channel_bot_health::GATEWAY_NOT_RECEIVED_REASON.into();
    }
    if code == "routed_elsewhere" {
        return "another route on this bot, from an earlier setup, sent it to a different agent"
            .into();
    }
    if code == "undelivered" {
        return format!("NyxID could not deliver it to {receiver}");
    }
    match code.strip_prefix("refused_") {
        Some(status) => format!("{receiver} refused it (HTTP {status})"),
        None => "it could not be delivered".into(),
    }
}

pub fn gateway_error_guidance(code: &str) -> &'static str {
    match code {
        "gateway_configuration_required" => {
            "Ask an administrator to check the active cmaeg service and its URL."
        }
        "gateway_not_authenticated" => {
            "The gateway could not authenticate NyxBot; reconnect and ask an administrator if this persists."
        }
        "gateway_permission_required" => {
            "Ask an administrator to verify the actual gateway confidential client's cmaeg introspection association."
        }
        "gateway_unavailable" => "The gateway is temporarily unavailable; retry shortly.",
        _ => "",
    }
}
