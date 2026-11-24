//! Metadata-only connect disclosure. This never grants authority to connect.
use super::*;
use serde::Serialize;

pub(crate) const EXPOSURE: &str = "Guests will be able to talk to this bot in private chats and in groups that allow everyone. By default, a group opens after a verified organization admin talks there. The organization pays for guests' messages and tool use. Guests can use this specialist's granted services at the guest levels shown below. You can turn Allow guests off in one action.";
pub(crate) const EXCLUSIONS: &str =
    "Guests cannot use account tools, approvals, nodes, machines, saved logins or private memory.";

#[derive(Serialize)]
pub(crate) struct Preview {
    pub bot_id: String,
    pub agent_id: String,
    pub org_id: String,
    pub guest_enabled: bool,
    pub private_chats: String,
    pub guest_policy_mode: &'static str,
    pub effective_guest_services: Vec<management::GuestServiceSummary>,
    pub audience_readiness: super::super::assistant_service_execution::readiness::AudienceReadiness,
    pub health: Option<super::super::channel_bot_health::Recommendation>,
    pub description: String,
}

impl Preview {
    pub(crate) fn summary(&self) -> String {
        let services = self
            .effective_guest_services
            .iter()
            .map(|s| {
                format!(
                    "{}: {}{}",
                    s.label,
                    s.access,
                    if s.limited_operations {
                        " (selected operations only)"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>();
        let guest_capabilities = if self.guest_enabled {
            let list = if services.is_empty() {
                "Model-only chat".into()
            } else {
                services.join(", ")
            };
            format!(" Guests can use: {list}.")
        } else {
            String::new()
        };
        format!(
            "{} {}{} {}{}",
            self.description,
            EXCLUSIONS,
            guest_capabilities,
            self.audience_readiness.summary(),
            self.health
                .as_ref()
                .map(|h| format!(" {}", h.text))
                .unwrap_or_default()
        )
    }
}

pub(crate) async fn load(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    base_url: &str,
    actor: &str,
    bot_id: &str,
    agent_id: &str,
    channel_id: Option<&str>,
) -> AppResult<Preview> {
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":agent_id,"kind":"specialist","destroyed_at":null})
        .await?
        .ok_or_else(denied)?;
    require_admin(db, actor, &agent.user_id).await?;
    let features = super::super::feature_flag_service::resolve_personal_features(db, actor).await?;
    if !features
        .iter()
        .any(|f| f == super::super::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY)
    {
        return Err(AppError::NotFound("Assistant route not found.".into()));
    }
    if !features.iter().any(|f| f == FLAG) {
        return Err(AppError::organization_agent_channels_disabled());
    }
    // report is read-only; serving_report/flag helpers can emit diagnostic writes.
    if !super::super::security_readiness_service::report(db, FLAG)
        .await?
        .serving_ready
    {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    let ready = super::super::security_readiness_service::report(db, GUEST_GRANTS_READINESS)
        .await?
        .serving_ready;
    let bot = db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":bot_id,"user_id":&agent.user_id,"is_active":true})
        .await?
        .ok_or_else(denied)?;
    let channels = db.collection::<NyxbotChannel>("nyxbot_channels");
    let (previous, reused) = if let Some(id) = channel_id {
        let mut row = management::load(db, actor, id, true).await?;
        if row.channel_bot_id != bot.id {
            return Err(denied());
        }
        let reused = if row.transport == "direct"
            && super::super::channel_adapters::meta::is_meta(&row.platform)
        {
            super::super::nyxbot_channel_query_service::reusable_meta(
                db,
                &row.user_id,
                &bot.user_id,
                &row,
                base_url,
            )
            .await?
        } else {
            true
        };
        if !reused {
            row.status = "disconnected".into();
        }
        (Some(row), reused)
    } else {
        let active =
            super::super::nyxbot_channel_query_service::active_for_bot(db, actor, bot_id).await?;
        let reusable = match &active {
            Some(row) => {
                super::super::nyxbot_channel_query_service::reusable_meta(
                    db,
                    actor,
                    &bot.user_id,
                    row,
                    base_url,
                )
                .await?
            }
            None => false,
        };
        if reusable {
            (active, true)
        } else {
            let mut previous = channels
                .find_one(doc! {"channel_bot_id":bot_id})
                .sort(doc! {"created_at":-1,"_id":-1})
                .await?;
            // Connect retires its broken/pending row before restoration reads it.
            if let Some(row) = &mut previous
                && active.as_ref().is_some_and(|active| active.id == row.id)
            {
                row.status = "disconnected".into();
            }
            (previous, false)
        }
    };
    let mut policy = initial_guest_policy(ready, &bot.platform);
    let mut private = if policy.enabled { "everyone" } else { "owner" }.to_owned();
    let mut carried = false;
    if let Some(previous) = previous {
        if !reused {
            policy.follows_agent_grants = false;
        }
        if let Some(binding) = &previous.org_specialist {
            if (reused || matches!(previous.status.as_str(), "failed" | "disconnected"))
                && binding.version == 1
                && binding.org_id == agent.user_id
                && previous.bot_owner_id.as_deref() == Some(agent.user_id.as_str())
                && super::super::channel_platform::canonical_platform(&previous.platform)
                    == super::super::channel_platform::canonical_platform(&bot.platform)
            {
                if previous.agent_id.as_deref() == Some(agent_id) {
                    policy = binding.guests.clone();
                } else {
                    policy = new_guest_policy(ready);
                    policy.enabled = binding.guests.enabled;
                }
                private = previous.private_chats.unwrap_or_else(|| "owner".into());
                carried = true;
            }
        } else if reused {
            return Err(AppError::Conflict("Disconnect and reconnect this bot to change between a personal and organization specialist".into()));
        }
    }
    let services = management::guest_services(db, &agent)
        .await?
        .into_iter()
        .filter(|s| s.access != "none" && guest_service_selected(&agent, &policy, ready, &s.id))
        .collect();
    let health = if policy.enabled {
        super::super::channel_bot_health::org_guest_billing(db, &agent.user_id)
            .await?
            .map(super::super::channel_bot_health::guest_billing_advice)
    } else {
        None
    };
    let mut description = if !policy.enabled && !carried {
        "Customer access starts off. Only verified organization members can use private chats until an administrator enables customer conversations.".to_owned()
    } else if !policy.enabled {
        "Saved guest access is off and stays off. Only verified organization members can talk to this bot.".to_owned()
    } else if carried && private != "everyone" {
        "Saved guest access stays on. Private chats stay restricted to verified organization members. Guests can talk in groups that allow everyone; the organization pays for their messages and tool use.".to_owned()
    } else {
        EXPOSURE.to_owned()
    };
    if policy.enabled && !(ready && policy.follows_agent_grants) {
        description.push_str(" Existing guest service limits apply. Guests can use only the services listed below; an empty list means model-only chat.");
    }
    Ok(Preview {
        bot_id: bot.id,
        agent_id: agent.id.clone(),
        org_id: agent.user_id.clone(),
        guest_enabled: policy.enabled,
        private_chats: private,
        guest_policy_mode: if ready && policy.follows_agent_grants {
            "grants"
        } else {
            "legacy"
        },
        effective_guest_services: services,
        audience_readiness: Box::pin(
            super::super::assistant_service_execution::readiness::audience_readiness(
                db,
                keys,
                &agent.user_id,
                Some(&agent.id),
            ),
        )
        .await?,
        health,
        description,
    })
}
