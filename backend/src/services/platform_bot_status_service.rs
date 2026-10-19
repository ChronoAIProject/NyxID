//! Shared bot setup status and eligible target policy. HTTP handlers only project DTOs.
use super::{
    platform_bot_config_service as configs,
    platform_bot_runtime_control_service as runtime_controls,
    platform_manager_contract as manager_contract,
};
use crate::{
    AppState, errors::AppResult, models::platform_bot_runtime_controls::PlatformBotRuntimeControls,
};
pub(crate) struct ProfileDisplay {
    pub profile: crate::models::platform_bot_config::PlatformBotConfig,
    pub display_name: String,
}

/// Labels are display-only and must not depend on live credentials or owner access.
pub(crate) async fn profile_displays(
    db: &mongodb::Database,
    profiles: Vec<crate::models::platform_bot_config::PlatformBotConfig>,
) -> AppResult<Vec<ProfileDisplay>> {
    use mongodb::bson::{Document, doc};
    let ids: Vec<_> = profiles.iter().map(|p| p.channel_bot_id.as_str()).collect();
    let mut cursor = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids}})
        .projection(doc! {"label":1})
        .await?;
    let mut labels = std::collections::HashMap::new();
    while cursor.advance().await? {
        let row = cursor.deserialize_current()?;
        if let (Ok(id), Ok(label)) = (row.get_str("_id"), row.get_str("label")) {
            let label: String = label
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            if !label.trim().is_empty() {
                labels.insert(id.to_string(), label);
            }
        }
    }
    Ok(profiles
        .into_iter()
        .map(|profile| {
            let display_name = labels
                .get(&profile.channel_bot_id)
                .cloned()
                .unwrap_or_else(|| "Disconnected bot".into());
            ProfileDisplay {
                profile,
                display_name,
            }
        })
        .collect())
}

pub(crate) async fn profile_display(
    db: &mongodb::Database,
    profile: crate::models::platform_bot_config::PlatformBotConfig,
) -> AppResult<ProfileDisplay> {
    profile_displays(db, vec![profile])
        .await?
        .pop()
        .ok_or_else(|| crate::errors::AppError::Internal("Missing bot display".into()))
}

pub(crate) enum FaqReadiness {
    Ready,
    Unavailable(manager_contract::UnavailableReason),
}
pub(crate) struct ConnectionFieldView {
    pub name: &'static str,
    pub label: &'static str,
    pub secret: bool,
}
pub(crate) struct ConnectionView {
    pub status: String,
    pub webhook_url: String,
    pub credential_source: String,
    pub fields: Vec<ConnectionFieldView>,
    pub instructions: Vec<String>,
}
pub(crate) struct RuntimeView {
    pub contract_version: u32,
    pub profile_generation: i64,
    pub supported: bool,
    pub runtime: Option<PlatformBotRuntimeControls>,
    pub public_readiness: FaqReadiness,
    pub state: &'static str,
    pub reason: Option<&'static str>,
    pub next_action: &'static str,
    pub platform: String,
    pub display_name: String,
    pub connection_url: String,
    pub faq_preview: String,
    pub connection: Option<ConnectionView>,
}
pub(crate) fn initial(
    row: Option<crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls>,
) -> RuntimeView {
    use manager_contract::UnavailableReason;
    let reason = match &row {
        None => UnavailableReason::RuntimeUninitialized,
        Some(r) if r.retired => UnavailableReason::ManagerUnavailable,
        Some(r) if r.emergency_stopped => UnavailableReason::EmergencyStopped,
        _ => UnavailableReason::PublicNotReady,
    };
    RuntimeView {
        contract_version: manager_contract::CONTRACT_VERSION,
        profile_generation: 0,
        supported: true,
        state: match &row {
            Some(r) if r.retired => "retired",
            Some(r) if r.emergency_stopped => "paused",
            Some(r) if r.personal_enabled || (r.public_enabled && r.public_policy.is_none()) => {
                "active"
            }
            _ => "setup_incomplete",
        },
        next_action: "configure",
        reason: None,
        platform: "telegram".into(),
        display_name: "Shared bot".into(),
        connection_url: String::new(),
        faq_preview: crate::services::platform_faq_service::render(&[]).unwrap_or_default(),
        connection: None,
        runtime: row,
        public_readiness: FaqReadiness::Unavailable(reason),
    }
}

pub(crate) async fn view(state: &AppState, id: &str) -> AppResult<RuntimeView> {
    use mongodb::bson::doc;
    let profile = configs::get(&state.db, id).await?;
    let runtime = runtime_controls::get(&state.db, id).await?;
    let mut view = initial(runtime);
    view.profile_generation = profile.generation;
    let bot = state
        .db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&profile.channel_bot_id})
        .await?;
    view.platform = profile
        .channel_transport
        .as_ref()
        .map(|t| t.platform.clone())
        .unwrap_or_else(|| "telegram".into());
    view.faq_preview =
        super::platform_channel_service::format_controls(&profile.manager_bot_id, view.faq_preview);
    view.connection_url = format!("/channel-bots/{}", profile.channel_bot_id);
    view.display_name = bot
        .as_ref()
        .map(|b| b.label.clone())
        .unwrap_or_else(|| "Disconnected bot".into());
    if profile.channel_transport.is_some()
        && let Some(bot) = &bot
    {
        let adapter = crate::services::channel_adapters::resolve_adapter(
            &bot.platform,
            &state.token_exchange_cache,
        )?;
        let descriptor = adapter.registration();
        view.connection = Some(ConnectionView {
            status: bot.status.clone(),
            credential_source: bot.credential_source.clone(),
            webhook_url: crate::services::channel_bot_service::webhook_url(
                &state.config.base_url,
                bot,
            ),
            fields: if bot.credential_source == "user" {
                descriptor
                    .fields
                    .iter()
                    .filter(|f| f.storage != "platform_bot_id")
                    .map(|f| ConnectionFieldView {
                        name: f.name,
                        label: f.label,
                        secret: f.secret,
                    })
                    .collect()
            } else {
                vec![]
            },
            instructions: descriptor
                .setup_instructions
                .iter()
                .map(|text| text.to_string())
                .collect(),
        });
    }
    let mut session = state.db.client().start_session().await?;
    let connected = if profile.channel_transport.is_some() {
        crate::services::platform_channel_service::fence_transport(
            &state.db,
            &mut session,
            &profile,
            None,
            false,
        )
        .await?
    } else {
        bot.as_ref().is_some_and(|b|b.is_active && b.platform_bot_id == profile.manager_bot_id)
            && state.db.collection::<mongodb::bson::Document>(crate::models::platform_credential::COLLECTION_NAME).find_one(doc! {"provider":"telegram-new","fields.manager_bot_id":&profile.manager_bot_id,"fields.observation_id":&profile.observation_id,"fields.webhook_ready":"true"}).await?.is_some()
    };
    if profile.deleted_at.is_some() {
        view.state = "retired";
    } else if !connected {
        view.state = "connection_repair_required";
        view.reason = Some("connection_unavailable");
    } else if view
        .runtime
        .as_ref()
        .is_some_and(|r| r.public_enabled && r.public_policy.is_some())
    {
        view.state = "setup_incomplete";
        view.reason = Some("legacy_public_policy_unavailable");
    }
    if connected
        && view.platform == "discord"
        && view.state != "retired"
        && !state
            .db
            .collection::<mongodb::bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":&profile.channel_bot_id,"shared_discord_commands_ready":true})
            .await?
            .is_some()
    {
        view.state = "setup_incomplete";
        view.reason = Some("discord_commands_required");
    }
    if connected
        && view
            .runtime
            .as_ref()
            .is_some_and(|r| r.public_enabled && r.public_policy.is_none())
        && view.state == "active"
    {
        view.public_readiness = FaqReadiness::Ready;
    } else if view.runtime.as_ref().is_some_and(|r| !r.public_enabled) && view.state == "active" {
        view.public_readiness =
            FaqReadiness::Unavailable(manager_contract::UnavailableReason::PublicDisabled);
    }
    view.connection_url = format!("/admin/platform-bots#profile-{}", id);
    view.next_action = match view.state {
        "active" => "test",
        "paused" => "resume",
        "connection_repair_required" => "repair_connection",
        "retired" => "none",
        _ => "enable",
    };
    Ok(view)
}

pub(crate) async fn eligible_targets(
    db: &mongodb::Database,
    actor: &str,
) -> AppResult<Vec<crate::models::channel_bot::ChannelBot>> {
    let mut eligible = vec![];
    for bot in super::channel_bot_service::list_all_bots(db, actor).await? {
        if !bot.is_active
            || bot.status != "active"
            || !super::platform_channel_service::supports(&bot.platform)
        {
            continue;
        }
        if !super::org_service::resolve_owner_access(db, actor, &bot.user_id)
            .await?
            .can_write()
        {
            continue;
        }
        if !super::platform_bot_authority::permits_personal_routes(db, &bot.id).await? {
            continue;
        }
        eligible.push(bot);
    }
    Ok(eligible)
}
