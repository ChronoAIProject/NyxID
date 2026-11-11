//! Admin-owned delegation; members cannot widen public org spending authority.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent, credits::Credits, nyxbot_channel::NyxbotChannel,
        org_channel::GuestPolicy, user_service::UserService,
    },
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const ORG_GUEST_EXPLANATION: &str = "Verified organization members use their own permissions and billing. Guests need guest access and a daily budget paid by the organization. Only organization admins can change guest access.";
pub const PERSONAL_GUEST_EXPLANATION: &str = "Guests use your billing. A specialist's per-service guest access controls which services they can use. NyxBot does not use your services for guests.";

pub(crate) fn default_daily_credits() -> Credits {
    Credits::from_whole(1)
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestSettings {
    pub enabled: bool,
    #[serde(default)]
    pub service_ids: Vec<String>,
    #[serde(default = "default_daily_credits")]
    pub daily_credits: Credits,
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
    update(
        db,
        actor,
        row,
        GuestSettings {
            enabled: input.enabled.unwrap_or(policy.enabled),
            service_ids: input
                .service_ids
                .unwrap_or_else(|| policy.service_ids.clone()),
            daily_credits: input
                .daily_credits
                .or(policy.daily_credits)
                .unwrap_or_else(default_daily_credits),
            expected_revision: Some(input.expected_revision),
        },
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
    use futures::TryStreamExt;
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&row.agent_id,"user_id":&b.org_id,"destroyed_at":null})
        .await?;
    let ids = agent
        .as_ref()
        .map(|a| a.grants.service_ids.clone())
        .unwrap_or_default();
    let services:Vec<UserService>=db.collection("user_services").find(doc!{"_id":{"$in":ids},"user_id":&b.org_id,"is_active":true,"node_id":null,"service_type":"http"}).limit(100).await?.try_collect().await?;
    let available: Vec<Value> = services
        .iter()
        .map(|s| json!({"id":s.id,"label":s.slug}))
        .collect();
    let (spent, reserved) = super::budget::today(
        db,
        b.guests.continuity_channel_id.as_deref().unwrap_or(&row.id),
    )
    .await?;
    let reason = match super::budget::require_wallet(db, &b.org_id).await {
        Ok(()) => None,
        Err(AppError::BillingNotConfigured(_)) => Some("org_wallet_not_provisioned"),
        Err(AppError::WalletSuspended) => Some("org_wallet_suspended"),
        Err(e) => return Err(e),
    };
    Ok(
        json!({"channel_id":row.id,"org_id":b.org_id,"can_manage":admin,"member_verified":member_verified,"guest_enabled":b.guests.enabled,
        "delegated_service_ids":b.guests.service_ids,"daily_credits":b.guests.daily_credits,
        "guest_revision":b.guests.revision,"channel_status":row.status,
        "default_daily_credits":default_daily_credits().to_string(),"spent_today_credits":spent,"reserved_today_credits":reserved,
        "description":ORG_GUEST_EXPLANATION,
        "guest_payer":"organization","available_services":available,"readiness_reason":reason}),
    )
}

pub(crate) async fn update(
    db: &Database,
    actor: &str,
    row: &NyxbotChannel,
    input: GuestSettings,
) -> AppResult<()> {
    let b = row.org_specialist.as_ref().ok_or_else(super::denied)?;
    super::require_admin(db, actor, &b.org_id).await?;
    super::enabled(db, &row.user_id).await?;
    if row.status != "active" {
        return Err(AppError::Conflict(
            "Reconnect this bot before changing guest access".into(),
        ));
    }
    if input
        .expected_revision
        .as_ref()
        .is_some_and(|revision| revision != &b.guests.revision)
    {
        return Err(settings_changed());
    }
    if input.daily_credits <= Credits::ZERO || input.service_ids.len() > 100 {
        return Err(AppError::ValidationError(
            "A positive daily guest budget and at most 100 delegated services are required".into(),
        ));
    }
    if input.enabled {
        super::budget::require_wallet(db, &b.org_id).await?;
    }
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(
            doc! {"_id":&row.agent_id,"user_id":&b.org_id,"kind":"specialist","destroyed_at":null},
        )
        .await?
        .ok_or_else(super::denied)?;
    let mut ids = input.service_ids.clone();
    ids.sort();
    ids.dedup();
    let mut authority = BTreeMap::new();
    for id in &ids {
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
    let policy = GuestPolicy {
        revision: uuid::Uuid::new_v4().to_string(),
        continuity_channel_id: b.guests.continuity_channel_id.clone(),
        service_authority: authority,
        enabled: input.enabled,
        service_ids: ids,
        daily_credits: Some(input.daily_credits),
    };
    let context = (db.clone(), actor.to_owned(), row.clone(), policy, agent);
    crate::services::transaction_service::run_db_only(db,&context,|tx,(db,actor,row,policy,agent)|Box::pin(async move{
        let b=row.org_specialist.as_ref().ok_or_else(super::denied)?;
        super::fence_management(db,tx,row,actor).await?;
        let live:AssistantAgent=bson::from_document(super::live_doc(db,tx,"assistant_agents",doc!{"_id":&agent.id,"user_id":&b.org_id,"destroyed_at":null},true).await?).map_err(|_|super::denied())?;
        if policy.service_authority.iter().any(|(id,d)| !live.grants.service_ids.contains(id) || service_digest(&live,id)!=*d) {return Err(AppError::Conflict("Agent permissions changed; review the guest delegation again".into()));}
        for id in &policy.service_ids {
            let service:UserService=bson::from_document(super::live_doc(db,tx,"user_services",doc!{"_id":id,"user_id":&b.org_id,"is_active":true},true).await?).map_err(|_|super::denied())?;
            if service.node_id.is_some() || service.service_type!="http" {return Err(super::denied());}
        }
        let mut filter=doc!{"_id":&row.id,"org_specialist.revision":&b.revision,"status":"active"};
        if b.guests.revision.is_empty() {
            filter.insert("$or",bson::bson!([{"org_specialist.guests.revision":""},{"org_specialist.guests.revision":null}]));
        } else {
            filter.insert("org_specialist.guests.revision",&b.guests.revision);
        }
        let mut fields=doc!{"org_specialist.guests":bson::to_bson(policy).map_err(|_|super::denied())?,"updated_at":bson::DateTime::now()};
        // Explicitly enabling guests opens private guest admission too. Editing
        // a budget or delegation must not broaden a saved private-chat limit.
        if policy.enabled && !b.guests.enabled {fields.insert("private_chats","everyone");}
        let changed=db.collection::<Document>("nyxbot_channels").update_one(filter,
            doc!{"$set":fields}).session(&mut *tx).await?;
        if changed.matched_count!=1 {return Err(settings_changed());}
        // Every exposure revision retires its old guest lanes immediately.
        // Members keep their own keys and turns; settlement retains its holds.
        super::lifecycle::retire_in_session(db,tx,&row.id,Some(&b.org_id)).await?;
        Ok(())
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
    let Some(saved) = previous.org_specialist.as_ref() else {
        return Ok(());
    };
    // The newest incompatible predecessor is a boundary. Never search past it
    // for an older, more permissive configuration.
    if !matches!(previous.status.as_str(), "failed" | "disconnected")
        || saved.version != 1
        || saved.org_id != binding.org_id
        || previous.bot_owner_id != row.bot_owner_id
        || previous.agent_id != row.agent_id
        || super::super::channel_platform::canonical_platform(&previous.platform)
            != super::super::channel_platform::canonical_platform(&row.platform)
    {
        return Ok(());
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
    if !policy
        .daily_credits
        .is_some_and(|credits| credits > Credits::ZERO)
    {
        policy.enabled = false;
    }
    if policy.enabled {
        match super::budget::require_wallet(db, &binding.org_id).await {
            Ok(()) => {}
            Err(AppError::BillingNotConfigured(_) | AppError::WalletSuspended) => {
                policy.enabled = false
            }
            Err(error) => return Err(error),
        }
    }
    db.collection::<Document>("nyxbot_channels").update_one(
        doc! {"_id":&row.id,"status":"pending","org_specialist.revision":&binding.revision},
        doc! {"$set":{"org_specialist.guests":bson::to_bson(&policy).map_err(|_|super::denied())?,
            "private_chats":&previous.private_chats}}).session(&mut *tx).await?;
    crate::services::nyxbot_gateway_release_service::audit(
        db,
        tx,
        &row.user_id,
        "org_channel_guest_policy_restored",
        json!({"channel_id":row.id,"predecessor_id":previous.id,
            "delegated_services":policy.service_ids.len(),"enabled":policy.enabled}),
    )
    .await
}
