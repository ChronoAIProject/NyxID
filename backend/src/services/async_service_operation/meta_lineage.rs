//! Grant provenance on the original watch, then its existing queued event.
use super::*;
use crate::models::assistant_conversation::ChannelTurnContext;

pub(super) async fn capture(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<Vec<ChannelTurnContext>> {
    let mut contexts: Vec<ChannelTurnContext> = Vec::new();
    for source in row.active_turn.iter().flat_map(|turn| {
        turn.channel_context
            .iter()
            .chain(turn.events.iter().flat_map(|event| &event.channel_contexts))
    }) {
        if source.meta_grant.is_none()
            || contexts
                .iter()
                .any(|existing| existing.meta_grant == source.meta_grant)
        {
            continue;
        }
        if contexts.len() >= 100 {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
        let mut context = source.clone();
        context.display_name = None;
        context.conversation_label = None;
        context.text_completeness = None;
        context.org_origin = None;
        context.original_message = false;
        contexts.push(context);
    }
    fence(db, tx, &contexts, true).await?;
    Ok(contexts)
}

pub(crate) async fn fence(
    db: &Database,
    tx: &mut ClientSession,
    contexts: &[ChannelTurnContext],
    fresh: bool,
) -> AppResult<()> {
    if contexts.len() > 100 {
        return Err(super::super::channel_meta_service::grants::incomplete());
    }
    for context in contexts {
        let proof = context
            .meta_grant
            .as_ref()
            .ok_or_else(super::super::channel_meta_service::grants::incomplete)?;
        if context.channel_bot_id != proof.bot_id {
            return Err(super::super::channel_meta_service::grants::incomplete());
        }
        super::super::channel_meta_service::scopes::fence_reference(db, tx, proof, fresh).await?;
    }
    Ok(())
}
