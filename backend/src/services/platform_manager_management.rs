//! Deterministic Telegram management through the current personal binding.
//! The legacy onboarding binding and guide toggle do not select an account.
use super::{
    platform_manager_contract::VerifiedManagerUpdate,
    platform_personal_binding_service::{self as bindings, ResolvedPersonalSender},
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
        platform_manager_ingress::ManagerCommandEffect,
        platform_manager_management::{ManagerManagementView, PersonalSetupSelection, SELECTIONS},
        platform_setup::{COLLECTION_NAME as SETUPS, PlatformSetup, SetupState},
    },
};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database, bson::doc};
use zeroize::Zeroizing;

pub(crate) fn parse(text: &str, username: &str) -> Option<ManagerManagementView> {
    let mut words = text.split_whitespace();
    let command = words.next()?;
    if command
        .split_once('@')
        .is_some_and(|(_, target)| !target.eq_ignore_ascii_case(username))
    {
        return None;
    }
    let verb = command.split('@').next()?;
    let view = match verb {
        "/onboarding" => ManagerManagementView::Onboarding,
        "/setups" => ManagerManagementView::Setups,
        "/bots" => ManagerManagementView::Bots,
        "/setup" => ManagerManagementView::Setup {
            setup_id: uuid::Uuid::parse_str(words.next()?).ok()?.to_string(),
        },
        "/bot" => ManagerManagementView::Bot {
            bot_id: uuid::Uuid::parse_str(words.next()?).ok()?.to_string(),
        },
        _ => return None,
    };
    words.next().is_none().then_some(view)
}

fn status(state: SetupState) -> &'static str {
    match state {
        SetupState::Draft => "Draft",
        SetupState::AwaitingIdentity => "Account review required",
        SetupState::Collecting => "Collecting details",
        SetupState::AwaitingApproval => "Waiting for your review",
        SetupState::Provisioning => "Preparing your bot",
        SetupState::AwaitingChildReview => "Review the created Telegram bot",
        SetupState::ReadyForTest => "Ready for customer test",
        SetupState::Active => "Active",
        SetupState::FailedRetryable => "Needs recovery",
        SetupState::Paused => "Paused",
        SetupState::Cancelled => "Cancelled",
        SetupState::Expired => "Expired",
    }
}

pub(crate) async fn select_in_session(
    db: &Database,
    session: &mut ClientSession,
    binding: &crate::models::platform_personal_binding::PlatformPersonalBinding,
    selection: &ManagerManagementView,
    selection_ref: &str,
) -> AppResult<Option<String>> {
    let ManagerManagementView::Setup { setup_id } = selection else {
        return Ok(None);
    };
    let exists = db
        .collection::<PlatformSetup>(SETUPS)
        .find_one(doc! {"_id":setup_id,"owner_user_id":&binding.user_id})
        .session(&mut *session)
        .await?;
    if exists.is_none() {
        return Ok(None);
    }
    let row = super::platform_setup_service::get_owned_in_session(
        db,
        session,
        &binding.user_id,
        setup_id,
    )
    .await?;
    if matches!(row.state, SetupState::Cancelled | SetupState::Expired) {
        return Ok(None);
    }
    db.collection::<bson::Document>(SELECTIONS).update_one(
        doc! {"_id":&binding.id},doc! {"$set":{"binding_generation":binding.generation,
            "user_id":&binding.user_id,"setup_id":&row.id,"setup_revision":row.revision,"selection_ref":selection_ref,
            "purge_after":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::days(30))}})
        .upsert(true).session(session).await?;
    Ok(Some(selection_ref.to_owned()))
}

/// Selection supplies context, never approval or a substitute account identity.
/// Read after the personal thread fence and within the owner-turn transaction.
pub(crate) async fn selected_context_in_session(
    db: &Database,
    session: &mut ClientSession,
    authority: &super::platform_manager_contract::PersonalBindingAuthority,
) -> AppResult<Option<String>> {
    let Some(choice) = db
        .collection::<PersonalSetupSelection>(SELECTIONS)
        .find_one(doc! {
            "_id":&authority.binding_id,"binding_generation":authority.binding_generation,
            "user_id":&authority.user_id,"purge_after":{"$gt":bson::DateTime::now()},
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    if db
        .collection::<PlatformSetup>(SETUPS)
        .find_one(doc! {
            "_id":&choice.setup_id,"owner_user_id":&authority.user_id,
        })
        .session(&mut *session)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let row = super::platform_setup_service::get_owned_in_session(
        db,
        session,
        &authority.user_id,
        &choice.setup_id,
    )
    .await?;
    if matches!(row.state, SetupState::Cancelled | SetupState::Expired) {
        return Ok(None);
    }
    Ok(Some(format!(
        "The owner selected onboarding setup {} in this private chat. Its current revision is {} and status is {}. Use this setup when the owner refers to their selected setup. Review this owned setup at /onboarding/setups/{}. Draft changes and setup approval require that human review page; ordinary account tools cannot edit onboarding drafts. Selection grants no approval. The usual owned-channel-bot tools and confirmation policy remain available for managing the owner's actual bots.",
        row.id,
        row.revision,
        status(row.state),
        row.id
    )))
}

/// Recomputed inside the final send transaction, after fencing the committed
/// binding generation. A changed resource/body suppresses the prepared notice.
pub(crate) async fn render_in_session(
    db: &Database,
    session: &mut ClientSession,
    update: &VerifiedManagerUpdate,
    effect: &ManagerCommandEffect,
    frontend_url: &str,
    connect_token: Option<&str>,
) -> AppResult<Zeroizing<String>> {
    let ManagerCommandEffect::Management {
        binding_id,
        binding_generation,
        selection,
        connect_link_id,
        selection_ref,
    } = effect
    else {
        return Err(AppError::Forbidden("Management authority required".into()));
    };
    let ResolvedPersonalSender::Personal { binding, .. } =
        bindings::resolve_sender_in_session(db, session, update).await?
    else {
        return Err(AppError::Conflict("Personal chat authority changed".into()));
    };
    if binding.id != *binding_id || binding.generation != *binding_generation {
        return Err(AppError::Conflict("Personal chat authority changed".into()));
    }
    let mut origin = reqwest::Url::parse(frontend_url)
        .map_err(|_| AppError::Internal("Frontend origin unavailable".into()))?;
    origin.set_query(None);
    origin.set_fragment(None);
    origin.set_path("/");
    let root = origin.as_str().trim_end_matches('/');
    let owner = &binding.user_id;
    let body = match selection {
        ManagerManagementView::Onboarding => {
            let link_id = connect_link_id
                .as_deref()
                .ok_or_else(|| AppError::Conflict("Creation request unavailable".into()))?;
            let token = connect_token.ok_or_else(|| {
                AppError::Conflict("Creation link unavailable; send /onboarding again".into())
            })?;
            if !super::channel_connect_link_service::token_matches_in_session(
                db, session, link_id, owner, token,
            )
            .await?
            {
                return Err(AppError::Conflict("Creation request changed".into()));
            }
            format!(
                "Create a Telegram bot for the NyxID account linked to this chat: {root}/connect/bot/{token}\nSign in with that same account on the website, then continue in Telegram.\nGuided assistant setup: {root}/onboarding/setups\nSend /setups to see progress, /setup <id> to select and review one, or /bots to list your bots. Account changes keep your usual confirmation policy."
            )
        }
        ManagerManagementView::Setups => {
            let rows = db
                .collection::<PlatformSetup>(SETUPS)
                .find(doc! {"owner_user_id":owner})
                .sort(doc! {"updated_at":-1,"_id":1})
                .limit(10)
                .session(&mut *session)
                .await?
                .stream(&mut *session)
                .try_collect::<Vec<_>>()
                .await?;
            let mut text = "Your onboarding setups:".to_owned();
            if rows.is_empty() {
                text.push_str("\nNo setups yet. Send /onboarding to get started.");
            }
            for row in rows {
                let row = super::platform_setup_service::get_owned_in_session(
                    db, session, owner, &row.id,
                )
                .await?;
                text.push_str(&format!(
                    "\n{} — {}\n{root}/onboarding/setups/{}",
                    row.id,
                    status(row.state),
                    row.id
                ));
            }
            text.push_str(&format!("\nAll your setups: {root}/onboarding/setups"));
            text
        }
        ManagerManagementView::Setup { setup_id } => {
            let row = db
                .collection::<PlatformSetup>(SETUPS)
                .find_one(doc! {"_id":setup_id,"owner_user_id":owner})
                .session(&mut *session)
                .await?;
            match row {
                Some(row) => {
                    let row = super::platform_setup_service::get_owned_in_session(
                        db, session, owner, &row.id,
                    )
                    .await?;
                    if matches!(row.state, SetupState::Cancelled | SetupState::Expired) {
                        format!(
                            "Setup {} — {}. Select a live setup with /setup <id>.\nReview: {root}/onboarding/setups/{}",
                            row.id,
                            status(row.state),
                            row.id
                        )
                    } else if let Some(selection_ref) = selection_ref
                        && db
                            .collection::<PersonalSetupSelection>(SELECTIONS)
                            .find_one(
                                doc! {"_id":binding_id,"binding_generation":binding_generation,
                                "user_id":owner,"setup_id":setup_id,"selection_ref":selection_ref,
                                "purge_after":{"$gt":bson::DateTime::now()}},
                            )
                            .session(&mut *session)
                            .await?
                            .is_some()
                    {
                        format!(
                            "Selected setup {} — {}\nReview and manage: {root}/onboarding/setups/{}",
                            row.id,
                            status(row.state),
                            row.id
                        )
                    } else {
                        format!(
                            "Setup {} — {}. This request is not the current selection.\nReview: {root}/onboarding/setups/{}",
                            row.id,
                            status(row.state),
                            row.id
                        )
                    }
                }
                None => {
                    "Setup unavailable for your linked account. Send /setups to see your setups."
                        .into()
                }
            }
        }
        ManagerManagementView::Bots => {
            let rows = db
                .collection::<ChannelBot>(BOTS)
                .find(doc! {"user_id":owner,"platform_profile_id":bson::Bson::Null})
                .sort(doc! {"created_at":-1,"_id":1})
                .limit(10)
                .session(&mut *session)
                .await?
                .stream(&mut *session)
                .try_collect::<Vec<_>>()
                .await?;
            let mut text = "Your bots:".to_owned();
            if rows.is_empty() {
                text.push_str("\nNo bots yet. Send /onboarding to create one.");
            }
            for row in rows {
                let label = row
                    .label
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(80)
                    .collect::<String>();
                text.push_str(&format!(
                    "\n{} — {} ({})\n{root}/channel-bots/{}",
                    row.id,
                    label,
                    bot_status(&row),
                    row.id
                ));
            }
            text.push_str(&format!("\nAll your bots: {root}/channel-bots"));
            text
        }
        ManagerManagementView::Bot { bot_id } => {
            let row = db
                .collection::<ChannelBot>(BOTS)
                .find_one(
                    doc! {"_id":bot_id,"user_id":owner,"platform_profile_id":bson::Bson::Null},
                )
                .session(&mut *session)
                .await?;
            match row {
                Some(row) => format!(
                    "Bot {} — {}\nManage: {root}/channel-bots/{}\nAsk your NyxBot here to change its routing or settings; your usual account confirmations apply.",
                    row.id,
                    bot_status(&row),
                    row.id
                ),
                None => {
                    "Bot unavailable for your linked account. Send /bots to list your bots.".into()
                }
            }
        }
    };
    if body.len() > 8192 {
        return Err(AppError::Internal(
            "Management notice exceeded its bound".into(),
        ));
    }
    Ok(Zeroizing::new(body))
}

fn bot_status(bot: &ChannelBot) -> &'static str {
    if !bot.is_active {
        return "paused";
    }
    match bot.status.as_str() {
        "active" => "active",
        "pending" => "setup pending",
        "pending_webhook" => "webhook setup pending",
        "failed" => "failed; review connection and verify",
        "invalid" => "invalid credentials; review connection and verify",
        _ => "unavailable; review bot status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_controls_are_exact_and_bound_to_target_bot() {
        for verb in ["/onboarding", "/setups", "/bots"] {
            assert!(parse(verb, "manager_bot").is_some());
            assert!(parse(&format!("{verb}@MANAGER_bot"), "manager_bot").is_some());
            for bad in [
                format!("{verb}@other_bot"),
                format!("{verb} extra"),
                format!("{verb}@manager_bot@other"),
            ] {
                assert!(parse(&bad, "manager_bot").is_none());
            }
        }
        let id = uuid::Uuid::new_v4();
        for verb in ["/bot", "/setup"] {
            assert!(parse(&format!("{verb} {id}"), "manager_bot").is_some());
            for bad in [
                verb.to_string(),
                format!("{verb} arbitrary"),
                format!("{verb} {id} extra"),
                format!("{verb}@other_bot {id}"),
            ] {
                assert!(parse(&bad, "manager_bot").is_none());
            }
        }
    }
}
