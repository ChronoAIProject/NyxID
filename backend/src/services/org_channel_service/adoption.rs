//! Explicit, reviewed handoff of all linked bots to the specialist's saved Grants.
//! No policy is inferred from an ordinary Grants save or a legacy settings PUT.
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde_json::{Value, json};

use crate::{
    errors::{AppError, AppResult},
    models::{assistant_agent::AssistantAgent, nyxbot_channel::NyxbotChannel},
    services::{audit_service, org_agent_service, transaction_service},
};

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adoption {
    pub review: String,
}

async fn agent(
    db: &Database,
    actor: &str,
    id: &str,
    tx: &mut ClientSession,
) -> AppResult<AssistantAgent> {
    let agent = db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":id,"kind":"specialist","destroyed_at":null})
        .session(&mut *tx)
        .await?
        .ok_or_else(super::denied)?;
    if actor == agent.user_id {
        return Err(super::denied());
    }
    super::require_admin(db, actor, &agent.user_id).await?;
    Ok(agent)
}

async fn channels(
    db: &Database,
    agent: &AssistantAgent,
    tx: &mut ClientSession,
) -> AppResult<Vec<NyxbotChannel>> {
    let mut cursor = db.collection::<NyxbotChannel>("nyxbot_channels")
        .find(doc! {"agent_id":&agent.id,"org_specialist.version":1,"org_specialist.org_id":&agent.user_id})
        .sort(doc!{"_id":1}).limit(1001).session(&mut *tx).await?;
    let rows: Vec<NyxbotChannel> = cursor.stream(&mut *tx).try_collect().await?;
    if rows.len() > 1000 {
        return Err(AppError::Conflict(
            "Too many linked bot records to review together".into(),
        ));
    }
    Ok(rows)
}

fn review(
    actor: &str,
    agent: &AssistantAgent,
    channels: &[NyxbotChannel],
    services: &[super::management::GuestServiceSummary],
) -> AppResult<String> {
    let bindings: Vec<_> = channels
        .iter()
        .map(|row| {
            json!({"id":row.id,"label":row.bot_label,
        "status":row.status,"binding":row.org_specialist})
        })
        .collect();
    let material = json!({"actor":actor,"agent":agent.id,"owner":agent.user_id,
        "grants":agent.grants,"guests":agent.guest_access,"scopes":agent.operation_scopes,
        "guest_scopes":agent.guest_operation_scopes,"bindings":bindings,"services":services});
    audit_service::keyed_fingerprint(
        audit_service::audit_chain_hmac_key(),
        b"org-channel-guest-grants-review-v1",
        material.to_string().as_bytes(),
    )
    .ok_or(AppError::SecurityReadinessUnavailable)
}

pub(crate) async fn preview(db: &Database, actor: &str, id: &str) -> AppResult<Value> {
    let mut tx = db.client().start_session().await?;
    let agent = agent(db, actor, id, &mut tx).await?;
    let channels = channels(db, &agent, &mut tx).await?;
    let services = super::management::guest_services(db, &agent).await?;
    let pending: Vec<_> = channels
        .iter()
        .filter(|row| {
            row.org_specialist
                .as_ref()
                .is_some_and(|b| !b.guests.follows_agent_grants)
        })
        .collect();
    let ready = super::guest_grants_ready(db).await?;
    Ok(
        json!({"ready":ready,"review":review(actor,&agent,&channels,&services)?,
        "pending":!pending.is_empty(),"services":services,
        "bots":channels.iter().map(|row| json!({"id":row.id,"name":row.bot_label,
            "status":row.status})).collect::<Vec<_>>()}),
    )
}

pub(crate) async fn adopt(db: &Database, actor: &str, id: &str, input: Adoption) -> AppResult<()> {
    if input.review.len() != 64 || !input.review.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::ValidationError(
            "Review the saved guest settings before adopting them".into(),
        ));
    }
    let ctx = (db.clone(), actor.to_owned(), id.to_owned(), input);
    transaction_service::run_db_only(db, &ctx, |tx, (db, actor, id, input)| {
        Box::pin(async move {
            if !super::guest_grants_ready(db).await? {
                return Err(AppError::SecurityReadinessUnavailable);
            }
            let agent = agent(db, actor, id, tx).await?;
            org_agent_service::authorize_guest_change(db, actor, &agent, tx).await?;
            // All grant writers and new channel bindings write this row, fencing
            // both policy edits and a concurrent change to the reviewed binding set.
            super::live_doc(
                db,
                tx,
                "assistant_agents",
                doc! {"_id":id,"user_id":&agent.user_id,"destroyed_at":null},
                true,
            )
            .await?;
            let channels = channels(db, &agent, tx).await?;
            if !channels.is_empty()
                && channels.iter().all(|row| {
                    row.org_specialist.as_ref().is_some_and(|b| {
                        b.guests.follows_agent_grants
                            && b.guests.grants_adoption_id.as_deref() == Some(&input.review)
                    })
                })
            {
                return Ok(());
            }
            let services = super::management::guest_services(db, &agent).await?;
            if review(actor, &agent, &channels, &services)? != input.review {
                return Err(AppError::Conflict(
                    "Guest settings or linked bots changed; review them again".into(),
                ));
            }
            for service in &services {
                super::live_doc(
                    db,
                    tx,
                    "user_services",
                    doc! {"_id":&service.id,"user_id":&agent.user_id,
                    "is_active":true,"node_id":null,"service_type":"http"},
                    true,
                )
                .await?;
            }
            for row in &channels {
                let b = row.org_specialist.as_ref().ok_or_else(super::denied)?;
                let changed = db
                    .collection::<Document>("nyxbot_channels")
                    .update_one(
                        doc! {"_id":&row.id,"org_specialist.revision":&b.revision},
                        doc! {"$set":{"org_specialist.guests.follows_agent_grants":true,
                        "org_specialist.guests.grants_adoption_id":&input.review,
                        "org_specialist.guests.revision":uuid::Uuid::new_v4().to_string(),
                        "updated_at":bson::DateTime::now()}},
                    )
                    .session(&mut *tx)
                    .await?;
                if changed.matched_count != 1 {
                    return Err(super::denied());
                }
                super::lifecycle::retire_in_session(db, tx, &row.id, Some(&agent.user_id)).await?;
            }
            crate::services::nyxbot_gateway_release_service::audit(
                db,
                tx,
                actor,
                "org_channel_guest_grants_adopted",
                json!({"agent_id":id,"org_id":agent.user_id,
                "binding_count":channels.len(),"adoption_id":input.review}),
            )
            .await?;
            Ok(())
        })
    })
    .await
}
