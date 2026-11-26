//! Intake evidence sampled by the native status sweep and projected by health reads.
use std::collections::{HashMap, HashSet};

use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use serde::Serialize;

use crate::{
    errors::AppResult,
    models::{
        channel_bot::ChannelBot,
        nyxbot_channel::{GatewaySource, NyxbotChannel},
    },
};

pub const RELAY_STOPPED: &str = "discord_relay_stopped";
pub const RELAY_BLOCKED: &str = "discord_relay_blocked";
pub const RELAY_UNCONFIRMED: &str = "discord_relay_unconfirmed";
pub const NATIVE_UNCONFIRMED: &str = "discord_native_intake_unconfirmed";
pub const NATIVE_CONNECTION_DOWN: &str = "discord_native_connection_down";

#[derive(Clone, Debug, Serialize)]
pub struct Intake {
    pub receiving_side: &'static str,
    pub state: String,
    pub code: Option<&'static str>,
    pub block_reason: Option<&'static str>,
    pub handed_over_at: Option<String>,
    pub last_inbound_at: Option<String>,
}

fn connection_down_intake(channel: Option<&NyxbotChannel>) -> Intake {
    let mut intake = native_intake(channel);
    intake.state = "connection_down".into();
    intake.code = Some(NATIVE_CONNECTION_DOWN);
    intake
}

fn native_intake(channel: Option<&NyxbotChannel>) -> Intake {
    let evidence = channel.and_then(|c| {
        c.gateway_intake
            .as_ref()
            .filter(|i| Some(&i.binding_id) == c.binding_id.as_ref())
    });
    let last = evidence.and_then(|i| {
        i.last_inbound_at
            .filter(|at| i.handed_over_at.is_none_or(|handover| *at >= handover))
    });
    Intake {
        receiving_side: "native_gateway",
        state: if last.is_some() {
            "event_observed"
        } else {
            "unconfirmed"
        }
        .into(),
        code: last.is_none().then_some(NATIVE_UNCONFIRMED),
        block_reason: None,
        handed_over_at: evidence.and_then(|i| i.handed_over_at.map(|at| at.to_rfc3339())),
        last_inbound_at: last.map(|at| at.to_rfc3339()),
    }
}

pub(super) fn project(
    bot: &ChannelBot,
    channel: Option<&NyxbotChannel>,
    block: Option<&'static str>,
    live_lease: bool,
) -> Intake {
    if native(channel) {
        let down = channel.is_some_and(|row| {
            crate::services::nyxbot_gateway_source::ready(row)
                && row.gateway_discord_connection.as_ref().is_some_and(|saved| {
                    row.binding_id.as_ref() == Some(&saved.binding_id)
                        && row.gateway_channel_id.as_ref() == Some(&saved.gateway_channel_id)
                        && matches!(&row.gateway_source, Some(GatewaySource::Discord { bot_id }) if bot_id == &saved.bot_id && bot_id == &bot.platform_bot_id)
                        && saved.down
                })
        });
        if down {
            return connection_down_intake(channel);
        }
        return native_intake(channel);
    }
    let state = bot
        .diagnostics
        .discord_gateway
        .as_ref()
        .map_or("unknown", |s| s.state.as_str());
    let code = if block.is_some() {
        Some(RELAY_BLOCKED)
    } else if state == "stopped" {
        Some(RELAY_STOPPED)
    } else if state != "connected" || !live_lease {
        Some(RELAY_UNCONFIRMED)
    } else {
        None
    };
    Intake {
        receiving_side: "nyxid_relay",
        state: state.into(),
        code,
        block_reason: block,
        handed_over_at: None,
        last_inbound_at: bot.diagnostics.last_inbound_at.map(|t| t.to_rfc3339()),
    }
}

fn native(channel: Option<&NyxbotChannel>) -> bool {
    channel.is_some_and(|c| {
        c.transport == "gateway" && matches!(c.gateway_source, Some(GatewaySource::Discord { .. }))
    })
}

fn relay_candidate(bot: &ChannelBot) -> bool {
    // Match the socket discovery filter; preserve existing failure advice for
    // suspended/failed/inactive bots rather than asking them to start a socket.
    bot.is_active && matches!(bot.status.as_str(), "active" | "pending_webhook")
}

/// Match discord_gateway::enabled's owner flag precedence without per-bot I/O.
/// Two batched reads: active memberships, then the relevant compiled-flag rows.
async fn relay_enabled_owners(db: &Database, owners: &HashSet<&str>) -> AppResult<HashSet<String>> {
    use crate::models::{
        feature_flag_override::{COLLECTION_NAME as FLAGS, FeatureFlagOverride},
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership},
    };
    use crate::services::feature_flag_service as flags;
    if owners.is_empty() {
        return Ok(HashSet::new());
    }
    let flag = flags::find_flag(flags::DISCORD_GATEWAY_FLAG_KEY).expect("compiled Discord flag");
    let memberships: Vec<OrgMembership> = db.collection(MEMBERSHIPS)
        .find(doc! {"member_user_id":{"$in":owners.iter().copied().collect::<Vec<_>>()},"revoked_at":bson::Bson::Null})
        .await?.try_collect().await?;
    let mut scopes = vec![bson::Bson::Null];
    scopes.extend(
        memberships
            .iter()
            .map(|m| bson::Bson::String(m.org_user_id.clone())),
    );
    let rows: Vec<FeatureFlagOverride> = db.collection(FLAGS)
        .find(doc! {"flag_key":flag.key,"definition_id":bson::Bson::Null,"org_user_id":{"$in":scopes}})
        .await?.try_collect().await?;
    let (org, platform): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| r.org_user_id.is_some());
    Ok(owners
        .iter()
        .filter(|owner| {
            let roles: Vec<_> = memberships
                .iter()
                .filter(|m| m.member_user_id == **owner)
                .map(|m| (m.org_user_id.clone(), m.role))
                .collect();
            !flags::resolve_personal_definitions(
                [(flag.key, flag.default_enabled, None)],
                &platform,
                &org,
                &roles,
                owner,
            )
            .is_empty()
        })
        .map(|owner| (*owner).to_owned())
        .collect())
}

/// At most seven batched reads: channels/releases/leases (3), relay owner flag
/// resolution (2), and one fleet readiness report (2). No per-bot read loops,
/// provider probes, or intake advice inferred from an unloaded snapshot.
pub async fn load(db: &Database, bots: &[ChannelBot]) -> AppResult<HashMap<String, Intake>> {
    use crate::models::{
        coordination::LEASE_COLLECTION_NAME, nyxbot_channel::COLLECTION_NAME as CHANNELS,
        nyxbot_gateway_release::COLLECTION_NAME as RELEASES,
    };
    let ids: Vec<_> = bots
        .iter()
        .filter(|b| b.platform == "discord")
        .map(|b| b.id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let channels: Vec<NyxbotChannel> = db
        .collection(CHANNELS)
        .find(doc! {"channel_bot_id":{"$in":&ids},"status":{"$in":["active","pending"]}})
        .await?
        .try_collect()
        .await?;
    let relay_owners: HashSet<_> = bots
        .iter()
        .filter(|b| b.platform == "discord")
        .filter(|b| {
            relay_candidate(b) && !native(channels.iter().find(|c| c.channel_bot_id == b.id))
        })
        .map(|b| b.user_id.as_str())
        .collect();
    let enabled = relay_enabled_owners(db, &relay_owners).await?;
    let ready = enabled.is_empty()
        || crate::services::security_readiness_service::require_serving(
            db,
            crate::services::feature_flag_service::DISCORD_GATEWAY_FLAG_KEY,
        )
        .await
        .is_ok();
    let releases:Vec<Document>=db.collection(RELEASES).find(doc!{"channel_bot_id":{"$in":&ids},"$or":[
        {"state":{"$ne":"settled"}},{"resolution":{"$nin":crate::services::nyxbot_gateway_release_service::accepted_resolutions_bson()}}
    ]}).projection(doc!{"channel_bot_id":1,"native_drain_for":1,"native_not_before":1,"state":1,"last_error":1,"gateway_channel_id":1,"dispatch_started_at":1,"last_gateway_answer":1,"review_reason":1,"resolution":1}).await?.try_collect().await?;
    let leases:Vec<Document>=db.collection(LEASE_COLLECTION_NAME).find(doc!{"_id":{"$in":ids.iter().map(|id|format!("discord-gateway:{id}")).collect::<Vec<_>>()},"$expr":{"$gt":["$expires_at","$$NOW"]}}).projection(doc!{"_id":1}).await?.try_collect().await?;
    let mut result = HashMap::new();
    for bot in bots.iter().filter(|b| b.platform == "discord") {
        let channel = channels.iter().find(|c| c.channel_bot_id == bot.id);
        if !native(channel) && !relay_candidate(bot) {
            continue;
        }
        if !native(channel) && !enabled.contains(&bot.user_id) {
            result.insert(
                bot.id.clone(),
                Intake {
                    receiving_side: "interactions_only",
                    state: "not_enabled".into(),
                    code: None,
                    block_reason: None,
                    handed_over_at: None,
                    last_inbound_at: bot.diagnostics.last_inbound_at.map(|at| at.to_rfc3339()),
                },
            );
            continue;
        }
        let block = if !ready {
            Some("readiness_unavailable")
        } else if channel.is_some_and(|c| {
            c.pending_agent_api_key_id.is_some() || c.gateway_setup_release_id.is_some()
        }) {
            Some("setup_pending")
        } else if releases
            .iter()
            .any(|r| r.get_str("channel_bot_id") == Ok(&bot.id))
        {
            releases
                .iter()
                .filter(|r| r.get_str("channel_bot_id") == Ok(&bot.id))
                .map(release_block_reason)
                .min_by_key(|reason| match *reason {
                    "preserved" => 0,
                    "destination_changed" => 1,
                    "owner_unavailable" => 2,
                    "auth_refused" => 3,
                    "identity_unconfirmed" | "changed_version" => 4,
                    "create_outcome_unconfirmed" => 5,
                    "release_unresolved" => 6,
                    _ => 7,
                })
        } else if channel.is_some_and(|c| c.transport != "direct") {
            Some("gateway_owned")
        } else {
            None
        };
        let lease = leases
            .iter()
            .any(|l| l.get_str("_id") == Ok(format!("discord-gateway:{}", bot.id).as_str()));
        result.insert(bot.id.clone(), project(bot, channel, block, lease));
    }
    Ok(result)
}

fn release_block_reason(row: &Document) -> &'static str {
    use crate::services::nyxbot_gateway_release_service::diagnostic_reason;
    if row.get_str("state") == Ok("settled") {
        return "preserved";
    }
    let current = diagnostic_reason(row.get_str("last_error").ok());
    // A fresh not-found observation supersedes historical refusal evidence;
    // it still cannot prove that an unacknowledged create never happened.
    if current == "create_outcome_unconfirmed"
        && row.get_str("gateway_channel_id").is_err()
        && row.get_datetime("dispatch_started_at").is_ok()
        && matches!(row.get_str("last_gateway_answer"), Err(_) | Ok("not_found"))
    {
        return "create_outcome_unconfirmed";
    }
    // Keep the original actionable reason through a transient re-observation.
    let reason = if row.get_str("state") == Ok("review_required")
        && matches!(current, "unavailable" | "unspecified")
    {
        row.get_str("review_reason")
            .ok()
            .map(|r| diagnostic_reason(Some(r)))
            .filter(|r| *r != "unspecified")
            .unwrap_or(current)
    } else {
        current
    };
    if matches!(
        reason,
        "preserved"
            | "destination_changed"
            | "owner_unavailable"
            | "auth_refused"
            | "identity_unconfirmed"
            | "changed_version"
    ) {
        return reason;
    }
    if row.get_datetime("native_not_before").is_ok() {
        "native_draining"
    } else {
        "release_unresolved"
    }
}

pub fn advice(intake: &Intake) -> Option<super::Recommendation> {
    let (action, text) = match intake.code? {
        RELAY_BLOCKED if intake.block_reason == Some("readiness_unavailable") => (
            "contact_admin",
            "Discord relay is enabled, but fleet readiness is unavailable. An administrator must restore channel:discord-gateway readiness across the serving replicas before intake can resume.",
        ),
        RELAY_BLOCKED if intake.block_reason == Some("create_outcome_unconfirmed") => (
            "contact_admin",
            "The gateway reports this connection as missing and cannot yet confirm the record was never created. NyxID keeps checking automatically, up to one hour apart. Ask a platform administrator to review the release using the link to this bot’s NyxID page and verify with the gateway that the original request can no longer complete. Relay intake stays paused until cleanup is confirmed and the 90-second drain finishes, so a second Discord session cannot start.",
        ),
        RELAY_BLOCKED if intake.block_reason == Some("preserved") => (
            "contact_admin",
            "An administrator preserved the gateway record. NyxID will not delete it automatically. A platform administrator must arrange its removal at the gateway; NyxID keeps checking for confirmed absence before the safety drain and relay restart.",
        ),
        RELAY_BLOCKED if intake.block_reason == Some("destination_changed") => (
            "contact_admin",
            "The configured gateway destination no longer matches this release. A platform administrator must restore or reconcile the catalog destination. NyxID keeps checking, but cannot send cleanup to a different destination without matching authority.",
        ),
        RELAY_BLOCKED if intake.block_reason == Some("owner_unavailable") => (
            "contact_admin",
            "The release owner's account is unavailable. A platform administrator must restore the owner's authority or arrange gateway cleanup. NyxID keeps checking; relay intake remains paused until cleanup is confirmed and the safety drain finishes.",
        ),
        RELAY_BLOCKED if intake.block_reason == Some("auth_refused") => (
            "contact_admin",
            "The gateway refused NyxID's cleanup authority. A gateway administrator must restore creator authentication and permission for this owner. NyxID keeps checking; a native activation retry cannot bypass this refusal.",
        ),
        RELAY_BLOCKED
            if matches!(
                intake.block_reason,
                Some("identity_unconfirmed" | "changed_version")
            ) =>
        {
            (
                "contact_admin",
                "The gateway record's identity or version does not match the retained cleanup authority. A platform administrator must inspect the release and coordinate with the gateway administrator. NyxID keeps checking but will not delete a replacement record or start a second Discord session.",
            )
        }
        RELAY_BLOCKED if intake.block_reason == Some("native_draining") => (
            "contact_admin",
            "NyxID is waiting for the 90-second safety drain after confirmed gateway cleanup. Discord relay intake resumes automatically after all retained releases are resolved.",
        ),
        RELAY_BLOCKED if intake.block_reason == Some("release_unresolved") => (
            "contact_admin",
            "NyxID is automatically rechecking the retained gateway release, with retries up to one hour apart. Relay intake resumes only after the gateway confirms cleanup and the 90-second safety drain finishes. Uncertain creation or deletion keeps intake paused; NyxID will not start a second Discord session.",
        ),
        RELAY_BLOCKED => (
            "contact_admin",
            "Discord relay intake is blocked by a retained setup or release fence. NyxID repairs safe failed attempts automatically. An administrator must review unresolved gateway releases; do not start a second Discord session.",
        ),
        RELAY_STOPPED => (
            "contact_admin",
            "Discord relay intake is stopped. No current relay connection is confirmed. Ask an administrator to check channel:discord-gateway, replica readiness and retained gateway releases. Eligible relay sessions restart automatically.",
        ),
        RELAY_UNCONFIRMED => (
            "verify",
            "Discord relay intake is not confirmed connected. Check the reported state, bot token, Message Content Intent and channel permissions. A stale connected report is not live connection proof.",
        ),
        NATIVE_CONNECTION_DOWN => (
            "contact_admin",
            "The Agent Event Gateway reports that its native Discord connection is down or stale. Ask an administrator to check CMA native intake and restore the connection. NyxID retains this channel’s native assignment.",
        ),
        NATIVE_UNCONFIRMED => (
            "contact_admin",
            "Intake is assigned to the native Agent Event Gateway, but no authenticated event is confirmed for this hand-over. Gateway selection does not confirm connection health. Send a message and ask an administrator to check CMA native intake; NyxID has not switched this bot back to relay.",
        ),
        _ => return None,
    };
    Some(super::advice(action, text.into()))
}

pub const INTAKE_CODES: &[&str] = &[
    RELAY_STOPPED,
    RELAY_BLOCKED,
    RELAY_UNCONFIRMED,
    NATIVE_UNCONFIRMED,
    NATIVE_CONNECTION_DOWN,
];

pub fn is_intake_code(code: &str) -> bool {
    INTAKE_CODES.contains(&code)
}
