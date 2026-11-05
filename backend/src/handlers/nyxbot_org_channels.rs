//! Org-specific identity admission; execution still uses the shared turn path.
use super::*;
use crate::services::org_channel_service as org;

const MEMBER_REFUSAL: &str =
    "Verify your organization membership in NyxID to chat with this assistant.";

pub(super) async fn inbound(
    state: &AppState,
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    sender: &Sender<'_>,
    text: &str,
    addressed: Option<bool>,
    event: &str,
) -> AppResult<Inbound> {
    use crate::models::channel_thread::ThreadSenderKind;
    if sender.kind == ThreadSenderKind::Unknown {
        return Ok(Inbound::Silent("refused_sender"));
    }
    let private = chat.kind.as_deref() == Some("private");
    if private
        && sender.owner_eligible
        && sender.original_message
        && sender.kind == ThreadSenderKind::Human
        && org::links::receive(&state.db, row, sender.id, text, sender.display_name).await?
    {
        return Ok(Inbound::Reply(
            "Return to NyxID and confirm this chat account in your own session to finish linking."
                .into(),
        ));
    }
    let agent = chat
        .agent_id
        .as_deref()
        .or(row.agent_id.as_deref())
        .ok_or_else(org::denied)?;
    let (identity, admin) = match org::sender(
        &state.db,
        row,
        sender.id,
        sender.owner_eligible && sender.kind == ThreadSenderKind::Human,
        agent,
    )
    .await
    {
        Ok(value) => value,
        Err(AppError::Forbidden(_)) => {
            return Ok(if private || addressed == Some(true) {
                Inbound::Reply(MEMBER_REFUSAL.into())
            } else {
                Inbound::Silent("refused_members")
            });
        }
        Err(e) => return Err(e),
    };
    let mut projected = row.clone();
    projected.owner_sender_ids = if identity.guest {
        Vec::new()
    } else {
        vec![sender.id.into()]
    };
    let current = if admin {
        chats::note_owner_presence(state, &projected, chat, sender.id).await?
    } else {
        chat.clone()
    };
    if sender.kind == ThreadSenderKind::Bot && !chats::bot_guest_allowed(&current, addressed) {
        return Ok(Inbound::Silent("refused_bot_policy"));
    }
    match chats::admission(&projected, &current, sender.id, addressed) {
        chats::Admission::Owner | chats::Admission::Guest => {}
        chats::Admission::Refuse => return Ok(Inbound::Reply(MEMBER_REFUSAL.into())),
        _ => {
            return Ok(Inbound::Silent(
                if addressed == Some(false) && current.reply_mode.as_deref() != Some("all") {
                    "not_addressed"
                } else {
                    "refused_members"
                },
            ));
        }
    }
    if !crate::services::channel_bot_turn_service::admit_with_owner_reset(
        &state.db,
        row,
        &current,
        sender.kind,
        !identity.guest,
        event,
    )
    .await?
    {
        return Ok(Inbound::Silent("refused_bot_budget"));
    }
    match Box::pin(start_chat_turn(
        state,
        row,
        &current,
        sender,
        text,
        identity.guest,
        private || addressed == Some(true),
        None,
        event,
    ))
    .await
    {
        Err(
            AppError::Forbidden(_)
            | AppError::BillingNotConfigured(_)
            | AppError::InsufficientCredits
            | AppError::WalletSuspended,
        ) => Ok(Inbound::Reply(org::UNAVAILABLE.into())),
        other => other,
    }
}
