//! Serialize draft generations without holding a transaction across Ornn reads.
use super::*;
use crate::{
    models::{
        assistant_agent_learning::{AUTHORED_CLAIMS_COLLECTION_NAME as CLAIMS, AuthoredDraftClaim},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
    services::api_key_mutation_service as mutations,
};
use mongodb::ClientSession;

#[derive(Clone)]
pub(super) struct Reservation {
    claim_id: String,
    claim: AuthoredDraftClaim,
    pub proposal: AssistantAgentLearningProposal,
    pub card: Option<AssistantAcknowledgement>,
}

fn conflict() -> AppError {
    AppError::Conflict("Skill draft changed; retry the draft request".into())
}

fn unstarted() -> bson::Document {
    doc! {
        "source":"authored",
        "publication.started":{"$ne":true}, "publication.uncertain_dispatch":{"$ne":true},
        "publication.skill_id":bson::Bson::Null, "publication.verified_at":bson::Bson::Null,
        "$and":[
            {"$or":[{"status":"pending"},
                {"status":"publication_failed","failure_code":"approval_expired"}]},
            {"$or":[{"publication.lease_expires_at":bson::Bson::Null},
                {"publication.lease_expires_at":{"$lte":bson::DateTime::now()}}]},
        ],
    }
}

async fn fence_agent(
    db: &Database,
    row: &AssistantAgentLearningProposal,
    session: &mut ClientSession,
) -> AppResult<()> {
    if db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.agent_id,"user_id":&row.owner_id,"destroyed_at":bson::Bson::Null},
            doc! {"$inc":{"authority_fence":1_i64}},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::NotFound("Agent not found".into()));
    }
    Ok(())
}

async fn latest_card(
    db: &Database,
    proposal: &AssistantAgentLearningProposal,
    session: &mut ClientSession,
) -> AppResult<Option<AssistantAcknowledgement>> {
    Ok(db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .find_one(doc! {"user_id":&proposal.owner_id,"tool_name":review::TOOL,
        "authored_skill.proposal_id":&proposal.id})
        .sort(doc! {"created_at":-1,"_id":-1})
        .session(session)
        .await?)
}

async fn authority(
    db: &Database,
    owner: &str,
    claim: &AuthoredDraftClaim,
    card: Option<&AssistantAcknowledgement>,
) -> AppResult<Option<ChatAuthority>> {
    let result = if let Some(card) = card {
        card_authority(db, owner, card).await
    } else {
        source_authority(db, owner, &claim.api_key_id, &claim.conversation_id).await
    };
    match result {
        Ok(chat) => Ok(Some(chat)),
        Err(AppError::NotFound(_) | AppError::Forbidden(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn touch_proposal(
    db: &Database,
    row: &AssistantAgentLearningProposal,
    session: &mut ClientSession,
) -> AppResult<()> {
    let mut filter = unstarted();
    filter.insert("_id", &row.id);
    filter.insert("revision", row.revision);
    if db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(filter, doc! {"$inc":{"authoring_fence":1}})
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(conflict());
    }
    Ok(())
}

fn reusable_card(card: &AssistantAcknowledgement) -> bool {
    matches!(card.status.as_str(), "pending" | "expired")
        || (card.status == "allowed" && card.kind == "action")
}

async fn reuse(
    db: &Database,
    claim_id: &str,
    claim: AuthoredDraftClaim,
    proposal: AssistantAgentLearningProposal,
    card: Option<AssistantAcknowledgement>,
    session: &mut ClientSession,
) -> AppResult<Option<Reservation>> {
    if card.as_ref().is_some_and(|card| {
        !reusable_card(card)
            || card.conversation_id != claim.conversation_id
            || card.api_key_id != claim.api_key_id
            || card
                .authored_skill
                .as_ref()
                .is_none_or(|r| r.revision != proposal.revision)
    }) {
        return Ok(None);
    }
    let Some(chat) = authority(db, &proposal.owner_id, &claim, card.as_ref()).await? else {
        return Ok(None);
    };
    // Serialize against key replacement, conversation deletion and publication.
    acks::fence(db, &chat, session).await?;
    touch_proposal(db, &proposal, session).await?;
    Ok(Some(Reservation {
        claim_id: claim_id.into(),
        claim,
        proposal,
        card,
    }))
}

pub(super) async fn reserve(
    state: &AppState,
    chat: &ChatAuthority,
    candidate: &AssistantAgentLearningProposal,
) -> AppResult<Reservation> {
    let db = state.db.clone();
    let chat = chat.clone();
    let candidate = candidate.clone();
    let claim_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!(
            "authored-draft:{}:{}:{}",
            chat.user_id, candidate.agent_id, candidate.fingerprint
        )
        .as_bytes(),
    )
    .to_string();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let db = &db;
        let chat = &chat;
        let candidate = &candidate;
        let result = Box::pin(async {
            // The target fence also serializes first reservation, so a stale
            // worker cannot recreate claim rows after owner/agent deletion.
            fence_agent(db, candidate, session).await?;
            let current = db.collection::<bson::Document>(CLAIMS)
                .find_one_and_update(doc! {"_id":&claim_id}, doc! {"$inc":{"fence":1_i64},
                    "$setOnInsert":{"owner_id":&chat.user_id,"agent_id":&candidate.agent_id}})
                .upsert(true).return_document(mongodb::options::ReturnDocument::After)
                .session(&mut *session).await?.ok_or_else(conflict)?;
            let proposals = db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
            if let Ok(value) = current.get_document("claim") {
                let claim: AuthoredDraftClaim = bson::from_document(value.clone()).map_err(|_| conflict())?;
                let mut filter = unstarted();
                filter.insert("_id", &claim.proposal_id);
                filter.insert("owner_id", &chat.user_id);
                filter.insert("agent_id", &candidate.agent_id);
                if let Some(proposal) = proposals.find_one(filter).session(&mut *session).await? {
                    let card = latest_card(db, &proposal, session).await?;
                    if let Some(reservation) = reuse(db, &claim_id, claim, proposal, card, session).await? {
                        return Ok(reservation);
                    }
                }
            } else {
                // Adopt pre-claim drafts only when their review origin is known.
                let mut filter = unstarted();
                filter.insert("owner_id", &chat.user_id);
                filter.insert("agent_id", &candidate.agent_id);
                filter.insert("fingerprint", &candidate.fingerprint);
                if let Some(proposal) = proposals.find_one(filter)
                    .sort(doc! {"created_at":-1,"_id":-1}).session(&mut *session).await?
                {
                    let card = latest_card(db, &proposal, session).await?.ok_or_else(||
                        AppError::Conflict("Review or discard the interrupted draft before creating another; its original review conversation was not recorded".into()))?;
                    let claim = AuthoredDraftClaim {
                        proposal_id: proposal.id.clone(), conversation_id: card.conversation_id.clone(),
                        api_key_id: card.api_key_id.clone(), turn_id: card.requested_turn_id.clone().unwrap_or_default(),
                        creation_turn_id: card.creation_turn_id.clone(),
                        requested_turn_id: card.requested_turn_id.clone(), voice_request_id: card.voice_request_id.clone(),
                        trigger_run_id: card.trigger_run_id.clone(),
                        routing: card.routing.clone(),
                    };
                    if let Some(reservation) = reuse(db, &claim_id, claim, proposal, Some(card), session).await? {
                        save_claim(db, &reservation.claim_id, &reservation.claim, session).await?;
                        return Ok(reservation);
                    }
                }
            }
            if proposals.count_documents(doc! {"agent_id":&candidate.agent_id,"status":"pending"})
                .session(&mut *session).await? >= 16
            {
                return Err(AppError::Conflict("Review pending skill drafts before creating more".into()));
            }
            let turn_id = chat.turn_id.as_ref().ok_or_else(|| AppError::Forbidden("Draft from an active owner turn".into()))?;
            let (conversation, _, _) = acks::fence(db, chat, session).await?;
            let turn = conversation.active_turn.as_ref().filter(|t| &t.turn_id == turn_id && !t.stop_requested)
                .ok_or_else(|| AppError::Forbidden("Draft from an active owner turn".into()))?;
            let mut initiating = doc! {"conversation_id":&conversation.id,"user_id":&chat.user_id,
                "role":{"$in":["user","orchestrator"]},"steering":bson::Bson::Null,"execution_pending":{"$ne":true}};
            if let Some(seq) = turn.initiating_message_seq {
                initiating.insert("seq", doc! {"$lte":seq});
            }
            let started_by = db.collection::<AssistantMessage>(MESSAGES).find_one(initiating)
                .sort(doc! {"seq":-1}).session(&mut *session).await?;
            let claim = AuthoredDraftClaim {
                proposal_id: candidate.id.clone(), conversation_id: chat.conversation_id.clone(),
                api_key_id: chat.api_key_id.clone(), turn_id: turn_id.clone(),
                creation_turn_id: Some(turn_id.clone()),
                requested_turn_id: if turn.trigger_run_id.is_some() { Some(turn_id.clone()) } else { started_by.map(|m| m.turn_id) },
                voice_request_id: turn.voice_request_id.clone(), trigger_run_id: turn.trigger_run_id.clone(),
                // Recovery re-raises the card with the drafting turn's routing, not a later turn's.
                routing: Some(super::super::assistant_access_policy::routing_for(chat, &conversation)),
            };
            proposals.insert_one(candidate).session(&mut *session).await?;
            save_claim(db, &claim_id, &claim, session).await?;
            Ok(Reservation { claim_id: claim_id.clone(), claim, proposal: candidate.clone(), card: None })
        }).await;
        mutations::transaction_result(result)
    }).await.map_err(mutations::map_transaction_error)
}

async fn save_claim(
    db: &Database,
    id: &str,
    claim: &AuthoredDraftClaim,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<bson::Document>(CLAIMS)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"claim":bson::to_bson(claim).map_err(|_| conflict())?}},
        )
        .session(session)
        .await?;
    Ok(())
}

pub(super) async fn finalize(
    state: &AppState,
    reservation: &Reservation,
    binding: &Value,
) -> AppResult<AssistantAcknowledgement> {
    let db = state.db.clone();
    let reservation = reservation.clone();
    let binding = binding.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let db = &db;
        let reservation = &reservation;
        let binding = &binding;
        let result = Box::pin(async {
            fence_agent(db, &reservation.proposal, session).await?;
            if db.collection::<bson::Document>(CLAIMS)
                .update_one(doc! {"_id":&reservation.claim_id,"claim.proposal_id":&reservation.proposal.id},
                    doc! {"$inc":{"fence":1_i64}}).session(&mut *session).await?.matched_count != 1
            {
                return Err(conflict());
            }
            touch_proposal(db, &reservation.proposal, session).await?;
            let claim = &reservation.claim;
            let chat = source_authority(db, &reservation.proposal.owner_id, &claim.api_key_id, &claim.conversation_id).await?;
            acks::fence(db, &chat, session).await?;
            if let Some(card) = latest_card(db, &reservation.proposal, session).await? {
                if reusable_card(&card)
                    && card.api_key_id == claim.api_key_id && card.conversation_id == claim.conversation_id
                    && card.authored_skill.as_ref().is_some_and(|r| r.revision == reservation.proposal.revision)
                {
                    return Ok(card);
                }
                return Err(conflict());
            }
            let origin = acks::RequestOrigin {
                creation_turn_id: claim.creation_turn_id.clone(),
                requested_turn_id: claim.requested_turn_id.clone(), voice_request_id: claim.voice_request_id.clone(),
                trigger_run_id: claim.trigger_run_id.clone(), routing: claim.routing.clone(),
            };
            acks::request_in_session(db, &chat, acks::Request {
                kind:"action", service:None, tool:Some(review::TOOL), arguments:Some(binding),
                summary:"Review the complete skill draft, publish it privately to Ornn and attach its exact version to this agent.",
                platform:false,
            }, Some(&origin), session).await
        }).await;
        mutations::transaction_result(result)
    }).await.map_err(mutations::map_transaction_error)
}
