//! Admin-owned delegation; members cannot widen public org spending authority.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent, credits::Credits, nyxbot_channel::NyxbotChannel,
        user_service::UserService,
    },
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const ORG_GUEST_EXPLANATION: &str = "Verified organization members use their own permissions and billing. Guests are people who have not verified an organization member account on this bot. The organization pays for their messages and tool use. Only organization admins can change guest access.";
pub const PERSONAL_GUEST_EXPLANATION: &str = "Guests use your billing. A specialist's per-service guest access controls which services they can use. NyxBot does not use your services for guests.";

pub(crate) fn default_daily_credits() -> Credits {
    Credits::from_whole(1)
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestSettings {
    pub enabled: bool,
    #[serde(default)]
    pub service_ids: Option<Vec<String>>,
    #[serde(default)]
    pub daily_credits: Option<Credits>,
    #[serde(default)]
    pub expected_revision: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestSettingsPatch {
    pub enabled: Option<bool>,
    pub service_ids: Option<Vec<String>>,
    pub daily_credits: Option<Credits>,
    pub expected_revision: String,
}

pub(crate) async fn patch(
    db: &Database,
    actor: &str,
    row: &NyxbotChannel,
    input: GuestSettingsPatch,
    callback: Option<&str>,
) -> AppResult<()> {
    if input.enabled.is_none() && input.service_ids.is_none() && input.daily_credits.is_none() {
        return Err(AppError::ValidationError(
            "Choose a guest setting to change".into(),
        ));
    }
    let policy = &row
        .org_specialist
        .as_ref()
        .ok_or_else(super::denied)?
        .guests;
    update_choice(
        db,
        actor,
        row,
        GuestSettings {
            enabled: input.enabled.unwrap_or(policy.enabled),
            service_ids: input.service_ids,
            daily_credits: input.daily_credits,
            expected_revision: Some(input.expected_revision),
        },
        input.enabled.is_some(),
        false,
        callback,
    )
    .await
}

pub(crate) fn service_digest(agent: &AssistantAgent, id: &str) -> String {
    crate::crypto::token::hash_token(&json!({"level":agent.guest_access.get(id).copied().unwrap_or_default(),
        "operations":agent.operation_scopes.get(id),"guest_operations":agent.guest_operation_scopes.get(id)}).to_string())
}

pub(crate) async fn load(
    db: &Database,
    actor: &str,
    id: &str,
    admin: bool,
) -> AppResult<NyxbotChannel> {
    let row = db
        .collection::<NyxbotChannel>("nyxbot_channels")
        .find_one(doc! {"_id":id,"org_specialist.version":1})
        .await?
        .ok_or_else(super::denied)?;
    let b = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    let acl = crate::services::org_agent_service::access(db, actor, &b.org_id).await?;
    if if admin {
        !matches!(
            acl,
            crate::services::org_service::OwnerAccess::AsOrgAdmin { .. }
        )
    } else {
        !crate::services::org_agent_service::can_use(&acl)
    } {
        return Err(super::denied());
    }
    Ok(row)
}

pub(crate) async fn view(db: &Database, actor: &str, row: &NyxbotChannel) -> AppResult<Value> {
    let b = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    let access = crate::services::org_agent_service::access(db, actor, &b.org_id).await?;
    let member_verified = crate::services::org_agent_service::can_use(&access)
        && db
            .collection::<Document>(crate::models::org_channel::LINKS)
            .find_one(doc! { "channel_id": &row.id, "org_id": &b.org_id,
            "binding_revision": &b.revision, "user_id": actor, "active": true })
            .projection(doc! { "_id": 1 })
            .await?
            .is_some();
    let admin = matches!(
        access,
        crate::services::org_service::OwnerAccess::AsOrgAdmin { .. }
    );
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&row.agent_id,"user_id":&b.org_id,"destroyed_at":null})
        .await?;
    let services = match agent.as_ref() {
        Some(agent) => guest_services(db, agent).await?,
        None => Vec::new(),
    };
    let ready = super::guest_grants_ready(db).await?;
    let follows = ready && b.guests.follows_agent_grants;
    let available: Vec<Value> = services
        .iter()
        .map(|s| json!({"id":s.id,"label":s.label}))
        .collect();
    let mut effective = Vec::new();
    let mut binding_limits_guest_services = false;
    for service in services.iter().filter(|service| service.access != "none") {
        if agent
            .as_ref()
            .is_some_and(|a| super::guest_service_selected(a, &b.guests, ready, &service.id))
        {
            effective.push(service);
        } else if b.guests.enabled && ready && !follows {
            binding_limits_guest_services = true;
        }
    }
    let reason = crate::services::channel_bot_health::org_guest_billing(db, &b.org_id).await?;
    let health = reason
        .filter(|_| b.guests.enabled)
        .map(crate::services::channel_bot_health::guest_billing_advice);
    let mut view = json!({"channel_id":row.id,"org_id":b.org_id,"agent_id":row.agent_id,
        "grants_ready":ready,"guest_policy_mode":if follows {"grants"} else {"legacy"},
        "binding_limits_guest_services":binding_limits_guest_services,
        "legacy_fields_ignored":ready,"effective_guest_services":effective,"can_manage":admin,"member_verified":member_verified,"guest_enabled":b.guests.enabled,
        "delegated_service_ids":b.guests.service_ids,
        "guest_revision":b.guests.revision,"channel_status":row.status,
        "private_chats":row.private_chats.as_deref().unwrap_or("owner"),"health":health,
        "description":ORG_GUEST_EXPLANATION,
        "guest_payer":"organization","available_services":available,"readiness_reason":reason});
    let (spent, reserved) = super::budget::today(
        db,
        b.guests.continuity_channel_id.as_deref().unwrap_or(&row.id),
    )
    .await?;
    view["daily_credits"] = json!(b.guests.daily_credits);
    view["default_daily_credits"] = json!(default_daily_credits().to_string());
    view["spent_today_credits"] = json!(spent);
    view["reserved_today_credits"] = json!(reserved);
    Ok(view)
}

pub(crate) async fn update(
    db: &Database,
    actor: &str,
    row: &NyxbotChannel,
    input: GuestSettings,
    callback: Option<&str>,
) -> AppResult<()> {
    update_choice(db, actor, row, input, true, false, callback).await
}

pub(crate) async fn enable_customer_dms(
    db: &Database,
    actor: &str,
    row: &NyxbotChannel,
    input: GuestSettings,
    callback: Option<&str>,
) -> AppResult<()> {
    update_choice(db, actor, row, input, true, true, callback).await
}

async fn update_choice(
    db: &Database,
    actor: &str,
    row: &NyxbotChannel,
    input: GuestSettings,
    explicit_enabled: bool,
    open_private_chats: bool,
    callback: Option<&str>,
) -> AppResult<()> {
    let b = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    super::require_admin(db, actor, &b.org_id).await?;
    super::enabled(db, &row.user_id).await?;
    if row.status != "active" {
        return Err(AppError::Conflict(
            "Reconnect this bot before changing guest access".into(),
        ));
    }
    let migration_off = explicit_enabled && !input.enabled;
    if input
        .expected_revision
        .as_ref()
        .is_some_and(|revision| revision != &b.guests.revision)
        && !(migration_off
            && super::defaults::enabled_revision(
                db,
                row,
                input
                    .expected_revision
                    .as_deref()
                    .unwrap_or(&b.guests.revision),
                None,
            )
            .await?
            .as_deref()
                == Some(b.guests.revision.as_str()))
    {
        return Err(settings_changed());
    }
    let ready = super::guest_grants_ready(db).await?;
    let daily_credits = input
        .daily_credits
        .or(b.guests.daily_credits)
        .unwrap_or_else(default_daily_credits);
    if (!ready && input.enabled && daily_credits <= Credits::ZERO)
        || input
            .service_ids
            .as_ref()
            .is_some_and(|ids| ids.len() > 100)
    {
        return Err(AppError::ValidationError(
            "A positive daily guest budget and at most 100 delegated services are required".into(),
        ));
    }
    if input.enabled && !b.guests.enabled {
        super::budget::require_wallet(db, &b.org_id).await?;
    }
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(
            doc! {"_id":&row.agent_id,"user_id":&b.org_id,"kind":"specialist","destroyed_at":null},
        )
        .await?
        .ok_or_else(super::denied)?;
    let mut ids = if ready {
        b.guests.service_ids.clone()
    } else {
        input
            .service_ids
            .clone()
            .unwrap_or_else(|| b.guests.service_ids.clone())
    };
    ids.sort();
    ids.dedup();
    let author_services = !ready && (input.enabled || input.service_ids.is_some());
    let mut authority = BTreeMap::new();
    for id in ids.iter().filter(|_| author_services) {
        let service = db
            .collection::<UserService>("user_services")
            .find_one(doc! {"_id":id,"user_id":&b.org_id,"is_active":true})
            .await?
            .ok_or_else(super::denied)?;
        if !agent.grants.service_ids.contains(id)
            || service.node_id.is_some()
            || service.service_type != "http"
        {
            return Err(AppError::ValidationError("Delegate only active HTTP services granted to this organization specialist; machines and node credentials are unavailable to guests".into()));
        }
        authority.insert(id.clone(), service_digest(&agent, id));
    }
    let mut policy = b.guests.clone();
    policy.enabled = input.enabled;
    if author_services {
        policy.service_authority = authority;
        policy.service_ids = ids;
    }
    if !ready && (input.enabled || input.daily_credits.is_some()) {
        policy.daily_credits = Some(daily_credits);
    }
    // Deprecated-only writes record the save without changing execution authority.
    let unchanged = ready
        && policy == b.guests
        && !open_private_chats
        && !(explicit_enabled && (!policy.enabled || policy.revision.is_empty()));
    if !unchanged {
        policy.revision = uuid::Uuid::new_v4().to_string();
    }
    #[cfg(test)]
    if policy.enabled {
        crate::services::durable_wake_tests::after("org_guest_before_meta_write").await;
    }
    // Reductions remain available when delivery is unhealthy. Any new
    // authority, including a changed service digest, keeps the live route gate.
    let widening = policy.enabled
        && (!b.guests.enabled
            || open_private_chats
            || (!ready
                && (policy.daily_credits.unwrap_or_else(default_daily_credits)
                    > b.guests.daily_credits.unwrap_or_else(default_daily_credits)
                    || policy.service_ids.iter().any(|id| {
                        !b.guests.service_ids.contains(id)
                            || policy.service_authority.get(id)
                                != b.guests.service_authority.get(id)
                    }))));
    let context = (
        db.clone(),
        actor.to_owned(),
        row.clone(),
        policy,
        agent,
        ready,
        unchanged,
        migration_off,
        author_services,
        open_private_chats,
        widening,
        callback.map(str::to_owned),
    );
    crate::services::transaction_service::run_db_only(db,&context,|tx,(db,actor,row,policy,agent,ready,unchanged,migration_off,author_services,open_private_chats,widening,callback)|Box::pin(async move{
        if *widening {
            crate::services::nyxbot_channel_query_service::fence_meta_default(db,row,callback.as_deref(),tx).await?;
        }
        let b=row.org_specialist.as_ref().ok_or_else(super::denied)?;
        super::fence_management(db,tx,row,actor).await?;
        if *ready && !super::guest_grants_ready(db).await? { return Err(AppError::SecurityReadinessUnavailable); }
        let live:AssistantAgent=bson::from_document(super::live_doc(db,tx,"assistant_agents",doc!{"_id":&agent.id,"user_id":&b.org_id,"destroyed_at":null},true).await?).map_err(|_|super::denied())?;
        if *author_services && policy.service_authority.iter().any(|(id,d)| !live.grants.service_ids.contains(id) || service_digest(&live,id)!=*d) {return Err(AppError::Conflict("Agent permissions changed; review the guest delegation again".into()));}
        for id in policy.service_ids.iter().filter(|_| *author_services) {
            let service:UserService=bson::from_document(super::live_doc(db,tx,"user_services",doc!{"_id":id,"user_id":&b.org_id,"is_active":true},true).await?).map_err(|_|super::denied())?;
            if service.node_id.is_some() || service.service_type!="http" {return Err(super::denied());}
        }
        let mut filter=doc!{"_id":&row.id,"org_specialist.revision":&b.revision,"status":"active"};
        if b.guests.revision.is_empty() {
            filter.insert("$or",bson::bson!([{"org_specialist.guests.revision":""},{"org_specialist.guests.revision":null}]));
        } else {
            filter.insert("org_specialist.guests.revision",&b.guests.revision);
        }
        if *migration_off
            && let Some(revision) = super::defaults::enabled_revision(db, row, &b.guests.revision, Some(&mut *tx)).await? {
            filter.remove("org_specialist.guests.revision");
            filter.insert("$or",bson::bson!([{"org_specialist.guests.revision":&b.guests.revision},{"org_specialist.guests.revision":revision}]));
        }
        if *unchanged {
            if db.collection::<Document>("nyxbot_channels").find_one(filter).session(&mut *tx).await?.is_none() { return Err(settings_changed()); }
            return crate::services::nyxbot_gateway_release_service::audit(db,tx,actor,super::defaults::SAVE_EVENT,json!({"channel_id":row.id})).await;
        }
        let mut fields=doc!{"org_specialist.guests":bson::to_bson(policy).map_err(|_|super::denied())?,"updated_at":bson::DateTime::now()};
        // Explicitly enabling guests opens private guest admission too. Editing
        // a budget or delegation must not broaden a saved private-chat limit.
        if *open_private_chats || (policy.enabled && !b.guests.enabled) {fields.insert("private_chats","everyone");}
        let changed=db.collection::<Document>("nyxbot_channels").update_one(filter,
            doc!{"$set":fields}).session(&mut *tx).await?;
        if changed.matched_count!=1 {return Err(settings_changed());}
        // Every exposure revision retires its old guest lanes immediately.
        // Members keep their own keys and turns; settlement retains its holds.
        super::lifecycle::retire_in_session(db,tx,&row.id,Some(&b.org_id)).await?;
        crate::services::nyxbot_gateway_release_service::audit(db,tx,actor,super::defaults::SAVE_EVENT,json!({"channel_id":row.id})).await
    })).await
}

fn settings_changed() -> AppError {
    AppError::Conflict("Guest settings changed; reload them before saving".into())
}

/// Carry configuration only. Member verification, execution lanes and history
/// remain bound to their original binding, and every new binding gets fresh
/// policy identity. The caller has inserted and fenced the new pending row.
pub(crate) async fn restore_in_session(
    db: &Database,
    tx: &mut ClientSession,
    row: &NyxbotChannel,
) -> AppResult<()> {
    let binding = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    let Some(previous) = db
        .collection::<NyxbotChannel>("nyxbot_channels")
        .find_one(doc! {"channel_bot_id":&row.channel_bot_id,"_id":{"$ne":&row.id}})
        .sort(doc! {"created_at":-1,"_id":-1})
        .session(&mut *tx)
        .await?
    else {
        return Ok(());
    };
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"org_specialist.guests.follows_agent_grants":false}},
        )
        .session(&mut *tx)
        .await?;
    let Some(saved) = previous.org_specialist.as_ref() else {
        return Ok(());
    };
    // The newest incompatible predecessor is a boundary. Never search past it
    // for an older, more permissive configuration.
    if !matches!(previous.status.as_str(), "failed" | "disconnected")
        || saved.version != 1
        || saved.org_id != binding.org_id
        || previous.bot_owner_id != row.bot_owner_id
        || super::super::channel_platform::canonical_platform(&previous.platform)
            != super::super::channel_platform::canonical_platform(&row.platform)
    {
        return Ok(());
    }
    if previous.agent_id != row.agent_id {
        let mut policy = binding.guests.clone();
        policy.enabled = saved.guests.enabled;
        policy.continuity_channel_id = Some(
            saved
                .guests
                .continuity_channel_id
                .clone()
                .unwrap_or_else(|| previous.id.clone()),
        );
        // Retargeting keeps the saved exposure choice but uses the new target's grants.
        db.collection::<Document>("nyxbot_channels").update_one(
            doc! {"_id":&row.id,"status":"pending","org_specialist.revision":&binding.revision},
            doc! {"$set":{"org_specialist.guests":bson::to_bson(&policy).map_err(|_|super::denied())?,"private_chats":&previous.private_chats}}
        ).session(&mut *tx).await?;
        return audit_restoration(db, tx, row, &previous, &policy).await;
    }
    let agent: AssistantAgent = bson::from_document(super::live_doc(db, tx, "assistant_agents",
        doc! {"_id":&row.agent_id,"user_id":&binding.org_id,"kind":"specialist","destroyed_at":null}, true).await?)
        .map_err(|_|super::denied())?;
    let mut policy = saved.guests.clone();
    policy.revision = uuid::Uuid::new_v4().to_string();
    policy.continuity_channel_id = Some(
        saved
            .guests
            .continuity_channel_id
            .clone()
            .unwrap_or_else(|| previous.id.clone()),
    );
    policy.service_ids.clear();
    policy.service_authority.clear();
    for id in saved.guests.service_ids.iter().take(100) {
        let digest = service_digest(&agent, id);
        if !agent.grants.service_ids.contains(id)
            || saved.guests.service_authority.get(id) != Some(&digest)
            || policy.service_ids.contains(id)
        {
            continue;
        }
        let service = db.collection::<UserService>("user_services")
            .find_one_and_update(doc! {"_id":id,"user_id":&binding.org_id,"is_active":true,"node_id":null,"service_type":"http"},
                doc! {"$inc":{"org_channel_fence":1_i64}})
            .session(&mut *tx).await?;
        if service.is_some() {
            policy.service_ids.push(id.clone());
            policy.service_authority.insert(id.clone(), digest);
        }
    }
    db.collection::<Document>("nyxbot_channels").update_one(
        doc! {"_id":&row.id,"status":"pending","org_specialist.revision":&binding.revision},
        doc! {"$set":{"org_specialist.guests":bson::to_bson(&policy).map_err(|_|super::denied())?,
            "private_chats":&previous.private_chats}}).session(&mut *tx).await?;
    audit_restoration(db, tx, row, &previous, &policy).await
}

async fn audit_restoration(
    db: &Database,
    tx: &mut ClientSession,
    row: &NyxbotChannel,
    previous: &NyxbotChannel,
    policy: &super::GuestPolicy,
) -> AppResult<()> {
    crate::services::nyxbot_gateway_release_service::audit(
        db,
        tx,
        &row.user_id,
        super::defaults::RESTORE_EVENT,
        json!({"channel_id":row.id,"predecessor_id":previous.id,"guest_revision":policy.revision,
            "delegated_services":policy.service_ids.len(),"enabled":policy.enabled}),
    )
    .await
}

#[derive(serde::Serialize)]
pub(crate) struct GuestServiceSummary {
    pub id: String,
    pub label: String,
    pub access: String,
    pub limited_operations: bool,
}

pub(crate) async fn guest_services(
    db: &Database,
    agent: &AssistantAgent,
) -> AppResult<Vec<GuestServiceSummary>> {
    use futures::TryStreamExt;
    let services: Vec<UserService> = db
        .collection("user_services")
        .find(doc! {
        "_id":{"$in":&agent.grants.service_ids},"user_id":&agent.user_id,
        "is_active":true,"node_id":null,"service_type":"http"})
        .sort(doc! {"_id":1})
        .await?
        .try_collect()
        .await?;
    let ids: Vec<_> = services.iter().map(|s| &s.endpoint_id).collect();
    let endpoints: Vec<crate::models::user_endpoint::UserEndpoint> = db
        .collection("user_endpoints")
        .find(doc! {"_id":{"$in":ids},"user_id":&agent.user_id})
        .await?
        .try_collect()
        .await?;
    Ok(services
        .iter()
        .map(|service| {
            use crate::services::agent_operation_scope_service::{applicable, guest_level};
            let scopes: Vec<_> = applicable(
                &agent.guest_operation_scopes,
                &service.id,
                service.catalog_service_id.as_deref(),
            )
            .chain(applicable(
                &agent.operation_scopes,
                &service.id,
                service.catalog_service_id.as_deref(),
            ))
            .collect();
            GuestServiceSummary {
                id: service.id.clone(),
                label: endpoints
                    .iter()
                    .find(|e| e.id == service.endpoint_id)
                    .map(|e| e.label.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Unavailable service".into()),
                access: if scopes.iter().any(|s| s.operations.is_empty()) {
                    "none".into()
                } else {
                    serde_json::to_value(guest_level(
                        agent,
                        &service.id,
                        service.catalog_service_id.as_deref(),
                    ))
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "use".into())
                },
                limited_operations: !scopes.is_empty(),
            }
        })
        .collect())
}
