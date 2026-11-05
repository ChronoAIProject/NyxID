use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::COLLECTION_NAME,
        assistant_crm_artifact::{MAX_TURN_CRM_ARTIFACTS, TurnCrmArtifact},
    },
    services::assistant_acknowledgement_service::ChatAuthority,
};
use mongodb::{
    Database,
    bson::{self, doc},
};

pub fn title(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(200)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Atomic insert with a bounded tail; duplicate identities update in place.
/// Each competing writer rechecks identity after the insert/update race.
pub async fn record(
    db: &Database,
    chat: &ChatAuthority,
    mut artifact: TurnCrmArtifact,
) -> AppResult<()> {
    artifact.title = title(&artifact.title);
    let turn = chat
        .turn_id
        .as_deref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let entry = bson::to_bson(&artifact)
        .map_err(|_| AppError::Internal("CRM artifact serialization failed".into()))?;
    let kind = bson::to_bson(&artifact.kind)
        .map_err(|_| AppError::Internal("CRM artifact kind serialization failed".into()))?;
    let base =
        doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"active_turn.turn_id":turn};
    let collection = db.collection::<bson::Document>(COLLECTION_NAME);
    loop {
        let mut existing = base.clone();
        existing.insert(
            "active_turn.crm_artifacts",
            doc! {"$elemMatch":{"kind":&kind,"id":&artifact.id}},
        );
        if collection
            .update_one(
                existing,
                doc! {"$set":{"active_turn.crm_artifacts.$":&entry}},
            )
            .await?
            .matched_count
            == 1
        {
            return Ok(());
        }
        let mut absent = base.clone();
        absent.insert(
            "active_turn.crm_artifacts",
            doc! {"$not":{"$elemMatch":{"kind":&kind,"id":&artifact.id}}},
        );
        if collection.update_one(absent, doc! {"$push":{"active_turn.crm_artifacts":{"$each":[&entry],"$slice":-(MAX_TURN_CRM_ARTIFACTS as i32)}}}).await?.matched_count == 1 { return Ok(()); }
        if collection.find_one(base.clone()).await?.is_none() {
            return Err(AppError::AssistantTurnRequired);
        }
    }
}

pub async fn settled(
    db: &Database,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<Vec<TurnCrmArtifact>> {
    let message = db
        .collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        )
        .find_one(
            doc! {"user_id":owner,"conversation_id":conversation,"turn_id":turn,"role":"assistant"},
        )
        .await?;
    Ok(message.map(|m| m.crm_artifacts).unwrap_or_default())
}

pub async fn publish_group(
    db: &Database,
    conversation: &crate::models::assistant_conversation::AssistantConversation,
    message: &crate::models::assistant_group::GroupMessage,
) -> AppResult<()> {
    if conversation.guest_turn || conversation.collaboration.is_some() {
        return Ok(());
    }
    let Some(turn) = conversation.active_turn.as_ref() else {
        return Ok(());
    };
    let artifacts = settled(db, &conversation.user_id, &conversation.id, &turn.turn_id).await?;
    if !artifacts.is_empty() {
        db.collection::<bson::Document>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME)
            .update_one(doc! {"_id":&message.id,"group_id":&message.group_id,"role":"agent"}, doc! {"$set":{"crm_artifacts":bson::to_bson(&artifacts).map_err(|_| AppError::Internal("CRM artifact serialization failed".into()))?}}).await?;
    }
    Ok(())
}

/// Channel payloads contain ordinary links, never a CRM snapshot.
pub fn append_links(text: &str, artifacts: &[TurnCrmArtifact], frontend: &str) -> String {
    use crate::{
        models::assistant_crm_artifact::CrmArtifactKind, services::assistant_links::AssistantPage,
    };
    let mut text = text.to_owned();
    for artifact in artifacts.iter().take(MAX_TURN_CRM_ARTIFACTS) {
        let page = match artifact.kind {
            CrmArtifactKind::View => AssistantPage::CrmView {
                owner: &artifact.owner_id,
                id: &artifact.id,
            },
            CrmArtifactKind::Record => AssistantPage::CrmRecord {
                owner: &artifact.owner_id,
                object: artifact.object_key.as_deref().unwrap_or_default(),
                id: &artifact.id,
            },
            CrmArtifactKind::Object => AssistantPage::CrmObject {
                owner: &artifact.owner_id,
                object: artifact.object_key.as_deref().unwrap_or_default(),
            },
        };
        let link = page.url(frontend);
        if !text.contains(&link) {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&link);
        }
    }
    text
}
