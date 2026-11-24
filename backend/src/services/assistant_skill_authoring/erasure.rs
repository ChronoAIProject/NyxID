//! Exact original draft records, paged under their closed grant scope.
use super::*;
use crate::models::assistant_agent_learning::{
    AUTHORED_CLAIMS_COLLECTION_NAME as CLAIMS, AuthoredDraftClaim,
    PUBLICATION_TARGETS_COLLECTION_NAME as TARGETS,
};
use futures::TryStreamExt;
use mongodb::ClientSession;

pub(crate) fn filter(bot: &str, grant: &str) -> bson::Document {
    doc! {"claim.meta_contexts":{"$elemMatch":{"meta_grant.bot_id":bot,"meta_grant.grant_id":grant}}}
}

pub(crate) struct Page {
    pub after: Option<String>,
    pub pending: bool,
}

pub(crate) async fn erase_page(
    db: &Database,
    tx: &mut ClientSession,
    scope: &bson::Document,
    after: Option<&str>,
) -> AppResult<Page> {
    let incomplete = super::super::channel_meta_service::grants::incomplete;
    let bot = scope.get_str("bot_id").map_err(|_| incomplete())?;
    let grant = scope.get_str("grant_id").map_err(|_| incomplete())?;
    let mut query = filter(bot, grant);
    if let Some(after) = after {
        query.insert("_id", doc! {"$gt":after});
    }
    let mut cursor = db
        .collection::<bson::Document>(CLAIMS)
        .find(query)
        .sort(doc! {"_id":1})
        .limit(100)
        .session(&mut *tx)
        .await?;
    let rows: Vec<bson::Document> = cursor.stream(&mut *tx).try_collect().await?;
    let mut pending = false;
    for row in &rows {
        let claim: AuthoredDraftClaim =
            bson::from_document(row.get_document("claim").map_err(|_| incomplete())?.clone())
                .map_err(|_| incomplete())?;
        let owner = row.get_str("owner_id").map_err(|_| incomplete())?;
        let agent = row.get_str("agent_id").map_err(|_| incomplete())?;
        if claim.meta_contexts.is_empty() || claim.meta_contexts.len() > 100 {
            return Err(incomplete());
        }
        for context in &claim.meta_contexts {
            let proof = context.meta_grant.as_ref().ok_or_else(incomplete)?;
            if proof.bot_id != context.channel_bot_id {
                return Err(incomplete());
            }
            super::super::channel_meta_service::scope_erasure::reference_in_range(proof, scope)?;
        }
        if let Some(raw) = db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":&claim.proposal_id})
            .session(&mut *tx)
            .await?
        {
            let proposal: AssistantAgentLearningProposal =
                bson::from_document(raw.clone()).map_err(|_| incomplete())?;
            if proposal.owner_id != owner
                || proposal.agent_id != agent
                || proposal.source != ProposalSource::Authored
            {
                return Err(incomplete());
            }
            let p = proposal.publication.as_ref();
            let completed = p.is_some_and(|p| {
                p.verified_at.is_some() && p.skill_id.as_ref().is_some_and(|id| !id.is_empty())
            });
            if !completed {
                if p.is_some_and(|p| {
                    p.started
                        || p.uncertain_dispatch
                        || p.verified_at.is_some()
                        || p.skill_id.is_some()
                }) {
                    pending = true;
                    continue;
                }
                if let Some(p) = p.filter(|p| p.target_kind.as_deref() == Some("update")) {
                    let target = format!(
                        "{}:{}",
                        p.target_skill_id.as_deref().ok_or_else(incomplete)?,
                        p.version
                    );
                    if let Some(holder)=db.collection::<bson::Document>(TARGETS).find_one(doc! {"_id":&target,"proposal_id":&proposal.id,
                        "owner_id":owner,"agent_id":agent,"operation_id":&p.operation_id,"package_sha256":&p.sha256}).session(&mut *tx).await? {
                        if holder.get_str("state")!=Ok("reserved") {pending=true;continue;}
                        if db.collection::<bson::Document>(TARGETS).delete_one(doc! {"_id":target,"proposal_id":&proposal.id,
                            "operation_id":&p.operation_id,"package_sha256":&p.sha256,"state":"reserved"}).session(&mut *tx).await?.deleted_count!=1 {return Err(incomplete())}
                    }
                }
                let mut exact = doc! {"_id":&proposal.id,"owner_id":owner,"agent_id":agent,"source":"authored",
                "publication":raw.get("publication").cloned().unwrap_or(bson::Bson::Null)};
                exact.insert(
                    "revision",
                    raw.get("revision").cloned().unwrap_or(bson::Bson::Null),
                );
                if db
                    .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                    .delete_one(exact)
                    .session(&mut *tx)
                    .await?
                    .deleted_count
                    != 1
                {
                    return Err(incomplete());
                }
            }
        }
        if db
            .collection::<bson::Document>(CLAIMS)
            .delete_one(
                doc! {"_id":row.get("_id").cloned(),"owner_id":owner,"agent_id":agent,
                "claim":row.get("claim").cloned()},
            )
            .session(&mut *tx)
            .await?
            .deleted_count
            != 1
        {
            return Err(incomplete());
        }
    }
    Ok(Page {
        after: if rows.len() == 100 {
            Some(
                rows.last()
                    .and_then(|row| row.get_str("_id").ok())
                    .ok_or_else(incomplete)?
                    .to_owned(),
            )
        } else {
            None
        },
        pending,
    })
}
