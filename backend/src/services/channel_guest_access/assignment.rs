use bson::{Document, doc};
use mongodb::{ClientSession, Database};

use super::{GuestTurn, obsolete};
use crate::{
    errors::AppResult,
    models::{
        assistant_agent::COLLECTION_NAME as AGENTS,
        channel_guest_access::HostedAssignment,
        channel_thread_follow::ThreadTurnBinding,
        nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, NyxbotChannel, NyxbotThread,
            THREADS_COLLECTION_NAME as THREADS,
        },
    },
};

pub(crate) fn capture(
    channel: &NyxbotChannel,
    chat: &NyxbotThread,
    followed: Option<&ThreadTurnBinding>,
    agent: &str,
) -> AppResult<HostedAssignment> {
    let (settings_chat_id, settings_generation, channel_generation) =
        if let Some(binding) = followed {
            (
                chat.follow.settings_chat_id.clone().ok_or_else(obsolete)?,
                binding.generation,
                binding.channel_generation,
            )
        } else {
            (
                chat.id.clone(),
                chat.follow.binding_generation,
                super::super::channel_thread_follow_service::channel_generation(channel, chat),
            )
        };
    Ok(HostedAssignment {
        agent_id: agent.to_owned(),
        settings_chat_id,
        settings_generation,
        channel_generation,
    })
}

async fn route(
    db: &Database,
    collection: &str,
    filter: Document,
    session: &mut Option<&mut ClientSession>,
) -> AppResult<Document> {
    let rows = db.collection::<Document>(collection);
    let row = if let Some(session) = session.as_deref_mut() {
        rows.find_one_and_update(
            filter,
            doc! {"$inc": {"channel_guest_assignment_fence": 1_i64}},
        )
        .session(session)
        .await?
    } else {
        rows.find_one(filter).await?
    };
    row.ok_or_else(obsolete)
}

async fn validate(
    db: &Database,
    turn: &GuestTurn,
    session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let Some(assignment) = &turn.hosted_assignment else {
        return Ok(());
    };
    let mut session = session;
    let channel: NyxbotChannel = bson::from_document(
        route(
            db,
            CHANNELS,
            doc! {"_id": &turn.binding_id, "channel_bot_id": &turn.bot_id, "status": "active"},
            &mut session,
        )
        .await?,
    )
    .map_err(|_| obsolete())?;
    let settings: NyxbotThread = bson::from_document(
        route(
            db,
            THREADS,
            doc! {"_id": &assignment.settings_chat_id, "channel_id": &channel.id, "user_id": &channel.user_id},
            &mut session,
        )
        .await?,
    )
    .map_err(|_| obsolete())?;
    if settings.follow.binding_generation != assignment.settings_generation
        || super::super::channel_thread_follow_service::channel_generation(&channel, &settings)
            != assignment.channel_generation
    {
        return Err(obsolete());
    }
    if let Some(agent) = settings.agent_id.as_ref().or(channel.agent_id.as_ref()) {
        if agent != &assignment.agent_id {
            return Err(obsolete());
        }
    } else {
        route(
            db,
            AGENTS,
            doc! {"_id": &assignment.agent_id, "user_id": &channel.user_id, "kind": "nyxbot", "destroyed_at": null},
            &mut session,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn fence(
    db: &Database,
    session: &mut ClientSession,
    turn: &GuestTurn,
) -> AppResult<()> {
    validate(db, turn, Some(session)).await
}

pub(super) async fn revalidate(db: &Database, turn: &GuestTurn) -> AppResult<()> {
    validate(db, turn, None).await
}
