//! Admin-owned delegation; members cannot widen public org spending authority.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent, credits::Credits, nyxbot_channel::NyxbotChannel,
        org_channel::GuestPolicy, user_service::UserService,
    },
};
use bson::{Document, doc};
use mongodb::Database;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestSettings {
    pub enabled: bool,
    #[serde(default)]
    pub service_ids: Vec<String>,
    pub daily_credits: Credits,
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
    let (spent, reserved) = super::budget::today(db, &row.id).await?;
    let reason = match super::budget::require_wallet(db, &b.org_id).await {
        Ok(()) => None,
        Err(AppError::BillingNotConfigured(_)) => Some("org_wallet_not_provisioned"),
        Err(AppError::WalletSuspended) => Some("org_wallet_suspended"),
        Err(e) => return Err(e),
    };
    Ok(
        json!({"channel_id":row.id,"org_id":b.org_id,"can_manage":admin,"member_verified":member_verified,"guest_enabled":b.guests.enabled,
        "delegated_service_ids":b.guests.service_ids,"daily_credits":b.guests.daily_credits,
        "default_daily_credits":"1","spent_today_credits":spent,"reserved_today_credits":reserved,
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
        let changed=db.collection::<Document>("nyxbot_channels").update_one(doc!{"_id":&row.id,"org_specialist.revision":&b.revision,"status":{"$in":["active","pending"]}},
            doc!{"$set":{"org_specialist.guests":bson::to_bson(policy).map_err(|_|super::denied())?,"updated_at":bson::DateTime::now()}}).session(&mut *tx).await?;
        if changed.matched_count!=1 {return Err(super::denied());}
        // Every exposure revision retires its old guest lanes immediately.
        // Members keep their own keys and turns; settlement retains its holds.
        super::lifecycle::retire_in_session(db,tx,&row.id,Some(&b.org_id)).await?;
        Ok(())
    })).await
}
