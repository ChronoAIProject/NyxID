//! Public text without starting an agent request. Existing enqueue semantics stay intact.
use super::*;
use crate::{
    models::assistant_group::{GroupMessage, MESSAGES_COLLECTION_NAME},
    services::{api_key_mutation_service as transactions, assistant_group_service},
};

/// `Some([])` is explicit people-only delivery. Omitted targeting only defaults
/// to people when only people are mentioned, or when no usable agents and no
/// agent mentions remain. Neither case changes sticky agent recipients.
pub async fn try_post(
    db: &Database,
    actor: &str,
    id: &str,
    text: &str,
    recipients: Option<&[String]>,
    uploads: &[String],
) -> AppResult<Option<GroupMessage>> {
    if recipients.is_some_and(|ids| !ids.is_empty()) {
        return Ok(None);
    }
    require_available(db, actor).await?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run(
        (db.clone(), actor.to_owned(), id.to_owned(), text.to_owned(), recipients.map(<[String]>::to_vec), uploads.to_vec()),
        |session, context| {
        let (db, actor, id, text, recipients, uploads) = &*context;
        let (actor, id, text) = (actor.as_str(), id.as_str(), text.as_str());
        let recipients = recipients.as_deref();
        Box::pin(async move {
        let work: AppResult<Option<GroupMessage>> = async {
            let group = read_group(db, actor, id, session).await?;
            let access = identity::for_group(db, actor, group, session).await?;
            let roster: Vec<_> = access.agents.values().map(|a| a.agent.clone()).collect();
            let people=people::resolve(text, &access.participants, &roster, true)?;
            if recipients.is_none() && assistant_group_service::ambiguous_mention(&people.agent_text, &roster) {
                return Err(invalid("More than one agent has that name; choose an exact recipient"));
            }
            let mentions = assistant_group_service::mentions(&people.agent_text, &roster);
            if recipients.is_none() && (!mentions.is_empty()
                || (!people.has_people && access.agents.values().any(|a| a.authority().is_some()))) {
                return Ok(None);
            }
            if !uploads.is_empty() {
                return Err(invalid("Attachments need an agent recipient. Choose an available agent or remove the attachments to send a message to people only."));
            }
            let text = text.trim();
            if text.is_empty() || text.chars().count() > super::super::assistant_nyxagent::MAX_MESSAGE_CHARS {
                return Err(invalid("A message must be nonempty and within the message limit"));
            }
            fence_person(db, actor, session).await?;
            fence_group(db, &access.group, session).await?;
            let mut message = append(db, &access.group, actor, text, None, session).await?;
            message.mentioned_participant_ids=people.ids;
            message.author_display_name=access.participants.iter().find(|p|p.id==actor).map(|p|p.display_name.clone());
            db.collection::<GroupMessage>(MESSAGES_COLLECTION_NAME).replace_one(doc! {"_id":&message.id},&message).session(&mut *session).await?;
            notifications::enqueue(db, &access.group, &message, session).await?;
            for agent in mentions {
                if access.agent(&agent).and_then(AgentIdentity::authority).is_none() {
                    append(db, &access.group, actor, REFUSAL, Some(&agent), session).await?;
                }
            }
            Ok(Some(message))
        }.await;
        transactions::transaction_result(work)
    })}).await.map_err(transactions::map_transaction_error)
}

async fn append(
    db: &Database,
    group: &AssistantGroup,
    actor: &str,
    text: &str,
    refused: Option<&str>,
    session: &mut ClientSession,
) -> AppResult<GroupMessage> {
    let now = Utc::now();
    let row = db.collection::<AssistantGroup>(GROUPS).find_one_and_update(
        doc! {"_id": &group.id, "collaboration.membership_revision": marker(group)?.membership_revision},
        doc! {"$inc": {"message_count": 1}, "$set": {"updated_at": bson::DateTime::from_chrono(now), "last_message_at": bson::DateTime::from_chrono(now)}}
    ).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
    let message = GroupMessage {
        crm_artifacts: Vec::new(),
        id: uuid::Uuid::new_v4().to_string(),
        group_id: group.id.clone(),
        user_id: group.user_id.clone(),
        seq: row.message_count,
        role: if refused.is_some() { "notice" } else { "user" }.into(),
        text: text.into(),
        author_user_id: refused.is_none().then(|| actor.into()),
        author_display_name: None,
        agent_id: refused.map(str::to_owned),
        agent_name: None,
        request_id: None,
        recipient_agent_ids: vec![],
        mentioned_participant_ids: vec![],
        attachments: vec![],
        activities: vec![],
        org_group: false,
        collaboration: Some(model::MessageBinding {
            version: model::VERSION,
            original_requester_id: Some(actor.into()),
            execution: None,
        }),
        created_at: now,
    };
    db.collection::<GroupMessage>(MESSAGES_COLLECTION_NAME)
        .insert_one(&message)
        .session(session)
        .await?;
    Ok(message)
}
