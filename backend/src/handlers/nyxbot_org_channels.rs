//! Org-specific identity admission; execution still uses the shared turn path.
use super::*;
use crate::services::org_channel_service as org;

#[cfg(test)]
pub(crate) async fn member_refusal(
    state: &AppState,
    row: &NyxbotChannel,
    sender_id: &str,
    eligible: bool,
) -> &'static str {
    member_notice(state, row, sender_id, eligible)
        .await
        .text(None)
}

async fn member_notice(
    state: &AppState,
    row: &NyxbotChannel,
    sender_id: &str,
    eligible: bool,
) -> Notice {
    if row.org_restored_at.is_none() || !eligible {
        return Notice::VerifyMember;
    }
    let previously_linked = {
        let mut prior = vec![doc! {
            "channel_id": &row.id,
            "sender_hash": org::sender_identity(row, sender_id),
        }];
        if let Some(predecessor_id) = row.continuity_from_channel_id.as_deref()
            && let Some(predecessor) = state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .find_one(doc! {"_id": predecessor_id})
                .await
                .ok()
                .flatten()
        {
            prior.push(doc! {
                "channel_id": predecessor.id.clone(),
                "sender_hash": org::sender_identity(&predecessor, sender_id),
            });
        }
        state
            .db
            .collection::<mongodb::bson::Document>(crate::models::org_channel::LINKS)
            .find_one(doc! {
                "org_id": row.org_specialist.as_ref().map(|b| b.org_id.as_str()),
                "$or": prior,
                // A restored prompt is useful only until fresh verification;
                // an active link is proof that this sender already verified.
                "active": false,
            })
            .await
            .ok()
            .flatten()
            .is_some()
    };
    if previously_linked {
        Notice::RestoredMember
    } else {
        Notice::VerifyMember
    }
}

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
    let grant = if crate::services::channel_adapters::meta::is_meta(&row.platform) {
        Some(
            crate::services::channel_meta_service::scopes::event_reference(&state.db, event)
                .await?
                .ok_or_else(crate::services::channel_meta_service::grants::incomplete)?,
        )
    } else {
        None
    };
    if private
        && sender.owner_eligible
        && sender.original_message
        && sender.kind == ThreadSenderKind::Human
        && org::links::receive_for_grant(
            &state.db,
            row,
            sender.id,
            text,
            sender.display_name,
            grant.as_ref(),
        )
        .await?
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
                Inbound::Reply(
                    member_notice(state, row, sender.id, sender.owner_eligible)
                        .await
                        .text(chat.notice_language)
                        .into(),
                )
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
        chats::Admission::Refuse => {
            return Ok(Inbound::Reply(
                member_notice(state, row, sender.id, sender.owner_eligible)
                    .await
                    .text(chat.notice_language)
                    .into(),
            ));
        }
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
        ChatTurnInput {
            text,
            guest: identity.guest,
            addressed: private || addressed == Some(true),
            binding: None,
            event_key: event,
        },
    ))
    .await
    {
        Err(
            AppError::Forbidden(_)
            | AppError::BillingNotConfigured(_)
            | AppError::InsufficientCredits
            | AppError::WalletSuspended,
        ) => Ok(Inbound::Reply(InboundReply::fixed(
            NoticeKind::GuestUnavailable,
            Notice::Unavailable.text(chat.notice_language),
        ))),
        other => other,
    }
}
