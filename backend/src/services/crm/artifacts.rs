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

/// One atomic update deduplicates identities, preserves creation and bounds the tail.
/// There is no client retry loop under contention.
pub async fn record(
    db: &Database,
    chat: &ChatAuthority,
    mut artifact: TurnCrmArtifact,
) -> AppResult<()> {
    let sanitized = title(artifact.title());
    *artifact.title_mut() = sanitized;
    let turn = chat
        .turn_id
        .as_deref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let entry = bson::to_bson(&artifact)
        .map_err(|_| AppError::Internal("CRM artifact serialization failed".into()))?;
    let kind = bson::to_bson(&artifact.kind())
        .map_err(|_| AppError::Internal("CRM artifact kind serialization failed".into()))?;
    let role = bson::to_bson(&artifact.role())
        .map_err(|_| AppError::Internal("CRM artifact role serialization failed".into()))?;
    let same = doc! {"$and":[{"$eq":["$$artifact.kind",kind]},{"$eq":["$$artifact.id",{"$literal":artifact.id()}]}]};
    let replacement = doc! {"$mergeObjects":[{"$literal":&entry},{"role":{"$cond":[{"$eq":["$$artifact.role","created"]},"created",role]}}]};
    let updated = db.collection::<bson::Document>(COLLECTION_NAME).update_one(
        doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"active_turn.turn_id":turn},
        vec![doc! {"$set":{"active_turn.crm_artifacts":{"$let":{
            "vars":{"entries":{"$ifNull":["$active_turn.crm_artifacts",[]]}},
            "in":{"$cond":[
                {"$anyElementTrue":[{"$map":{"input":"$$entries","as":"artifact","in":same.clone()}}]},
                {"$map":{"input":"$$entries","as":"artifact","in":{"$cond":[same,replacement,"$$artifact"]}}},
                {"$slice":[{"$concatArrays":["$$entries",{"$literal":[entry]}]},-(MAX_TURN_CRM_ARTIFACTS as i32)]}
            ]}
        }}}}],
    ).await?;
    if updated.matched_count == 1 {
        Ok(())
    } else {
        Err(AppError::AssistantTurnRequired)
    }
}

/// A committed effect remains successful even if its optional chat reference cannot be saved.
pub async fn committed_result(
    db: &Database,
    chat: &ChatAuthority,
    artifact: AppResult<TurnCrmArtifact>,
    result: serde_json::Value,
) -> serde_json::Value {
    let outcome = match artifact {
        Ok(artifact) => record(db, chat, artifact).await,
        Err(error) => Err(error),
    };
    if outcome.is_err() {
        tracing::warn!(conversation_id = %chat.conversation_id, turn_id = ?chat.turn_id, "CRM artifact reference could not be recorded after a successful operation");
    }
    result
}

/// Empty messages retain the existing delivery path without a feature-flag read.
pub async fn enabled_for(
    db: &Database,
    owner: &str,
    artifacts: &[TurnCrmArtifact],
) -> AppResult<bool> {
    if artifacts.is_empty() {
        return Ok(false);
    }
    super::enabled(db, owner).await
}

pub async fn channel_text(
    db: &Database,
    owner: &str,
    artifacts: &[TurnCrmArtifact],
    text: &str,
    frontend: &str,
) -> String {
    if enabled_for(db, owner, artifacts).await.unwrap_or(false) {
        append_links(text, artifacts, frontend)
    } else {
        text.to_owned()
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
    use crate::services::assistant_links::AssistantPage;
    let mut text = text.to_owned();
    for artifact in artifacts.iter().take(MAX_TURN_CRM_ARTIFACTS) {
        let page = match artifact {
            TurnCrmArtifact::View { owner_id, id, .. } => AssistantPage::CrmView {
                owner: owner_id,
                id,
            },
            TurnCrmArtifact::Record {
                owner_id,
                object_key,
                id,
                ..
            } => AssistantPage::CrmRecord {
                owner: owner_id,
                object: object_key,
                id,
            },
            TurnCrmArtifact::Object {
                owner_id,
                object_key,
                ..
            } => AssistantPage::CrmObject {
                owner: owner_id,
                object: object_key,
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
