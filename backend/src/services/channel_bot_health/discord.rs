//! Read-only intake evidence. Selection is never connection or delivery proof.
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

#[derive(Clone, Debug, Serialize)]
pub struct Intake {
    pub receiving_side: &'static str,
    pub state: String,
    pub code: Option<&'static str>,
    pub block_reason: Option<&'static str>,
    pub handed_over_at: Option<String>,
    pub last_inbound_at: Option<String>,
}

pub(super) fn project(
    bot: &ChannelBot,
    channel: Option<&NyxbotChannel>,
    block: Option<&'static str>,
    live_lease: bool,
) -> Intake {
    if native(channel) {
        let evidence = channel.and_then(|c| {
            c.gateway_intake
                .as_ref()
                .filter(|i| Some(&i.binding_id) == c.binding_id.as_ref())
        });
        let last = evidence.and_then(|i| {
            i.last_inbound_at
                .filter(|at| i.handed_over_at.is_none_or(|handover| *at >= handover))
        });
        return Intake {
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
            last_inbound_at: last.map(|t| t.to_rfc3339()),
        };
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
        || crate::services::security_readiness_service::require(
            db,
            crate::services::feature_flag_service::DISCORD_GATEWAY_FLAG_KEY,
        )
        .await
        .is_ok();
    let releases:Vec<Document>=db.collection(RELEASES).find(doc!{"channel_bot_id":{"$in":&ids},"$or":[
        {"state":{"$ne":"settled"}},{"resolution":{"$nin":["released","missing","cancelled","not_created","confirmed_missing"]}}
    ]}).projection(doc!{"channel_bot_id":1,"native_drain_for":1,"native_not_before":1,"state":1}).await?.try_collect().await?;
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
            Some(
                if releases.iter().any(|r| {
                    r.get_str("channel_bot_id") == Ok(&bot.id)
                        && r.get_datetime("native_not_before").is_ok()
                }) {
                    "native_draining"
                } else {
                    "release_unresolved"
                },
            )
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

pub fn advice(intake: &Intake) -> Option<super::Recommendation> {
    let (action, text) = match intake.code? {
        RELAY_BLOCKED if intake.block_reason == Some("readiness_unavailable") => (
            "contact_admin",
            "Discord relay is enabled, but fleet readiness is unavailable. An administrator must restore channel:discord-gateway readiness across the serving replicas before intake can resume.",
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
        NATIVE_UNCONFIRMED => (
            "contact_admin",
            "Intake is assigned to the native Agent Event Gateway, but no authenticated event is confirmed for this hand-over. Gateway selection does not confirm connection health. Send a message and ask an administrator to check CMA native intake; NyxID has not switched this bot back to relay.",
        ),
        _ => return None,
    };
    Some(super::advice(action, text.into()))
}

pub fn is_intake_code(code: &str) -> bool {
    matches!(
        code,
        RELAY_STOPPED | RELAY_BLOCKED | RELAY_UNCONFIRMED | NATIVE_UNCONFIRMED
    )
}
