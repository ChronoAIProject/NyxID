//! Reserved personal commands commit with the ingress receipt, never an AI turn.
use super::platform_entry_scope as scope;
use mongodb::Database;
use zeroize::Zeroizing;

use super::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    platform_bot_runtime_control_service as controls, platform_link_intent_service as intents,
    platform_manager_contract::*,
    platform_manager_ingress_service as ingress,
    platform_personal_binding_service::LifecycleProducer,
};
use crate::errors::{AppError, AppResult};
pub use crate::models::platform_manager_ingress::{ChatLoginStatus, ManagerCommandEffect};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersonalCommandKind {
    Link,
    Confirm,
    Unlink,
    Status,
    Help,
    Management(crate::models::platform_manager_management::ManagerManagementView),
}

pub fn personal_command(text: &str, username: &str) -> Option<PersonalCommandKind> {
    let mut words = text.split_whitespace();
    let command = words.next()?;
    if command
        .split_once('@')
        .is_some_and(|(_, target)| !target.eq_ignore_ascii_case(username))
    {
        return None;
    }
    match command.split('@').next()? {
        "/link" | "/login" => Some(PersonalCommandKind::Link),
        "/unlink" | "/logout" => Some(PersonalCommandKind::Unlink),
        "/status" | "/check" => Some(PersonalCommandKind::Status),
        "/help" | "/start" if words.next().is_none() => Some(PersonalCommandKind::Help),
        "/confirm"
            if words
                .next()
                .is_some_and(|code| code.to_ascii_uppercase().starts_with("P-")) =>
        {
            Some(PersonalCommandKind::Confirm)
        }
        _ => None,
    }
}

/// Ephemeral one-time output. The intent bearer is never stored in a send queue.
pub enum PersonalCommandReply {
    Link {
        token: Zeroizing<String>,
    },
    Confirmed,
    Unlinked,
    Status(ChatLoginStatus),
    Help,
    ManagementUnavailable,
    Management {
        connect_token: Option<Zeroizing<String>>,
    },
}

impl ChatLoginStatus {
    pub(crate) fn text(self, entry: &EntryIdentity) -> AppResult<String> {
        let chat = super::platform_chat_presentation::ChatPresentation::for_entry(entry)?;
        if chat != super::platform_chat_presentation::ChatPresentation::Telegram {
            return Ok(match self {
                Self::LoggedOut => format!(
                    "Chat login status: logged out. You can ask public NyxID questions. Send {} to sign in or create an account, then review and confirm linking this private chat. {}",
                    chat.login(),
                    chat.question_instruction()
                ),
                Self::Pending => format!(
                    "Chat login status: confirmation pending. Finish the account review on the NyxID website, then send its confirmation command in this private chat. Signing up or signing in on the website alone does not log this chat in. Send {} to cancel, or {} for a new link.",
                    chat.logout(),
                    chat.login()
                ),
                Self::AwaitingConfirmation => format!(
                    "Chat login status: browser approved; waiting for confirmation here. Return to the approval page and use its confirmation command in this private chat. Send {} for a new review if the command is lost.",
                    chat.login()
                ),
                Self::Expired => format!(
                    "Chat login status: linking request expired. Send {} to start a new account review.",
                    chat.login()
                ),
                Self::LoggedIn => format!(
                    "Chat login status: logged in. This private chat uses your verified NyxID account's NyxBot, existing access, memory and personal credits. Browser sign-out does not log this chat out. Send {} to disconnect this chat. {}",
                    chat.logout(),
                    chat.question_instruction()
                ),
                Self::Unavailable => format!(
                    "Chat login status: personal access unavailable. Your account or this chat's authority is no longer available, or personal chat is paused. Send {} to disconnect; {} can start a fresh review when personal chat is available.",
                    chat.logout(),
                    chat.login()
                ),
            });
        }
        Ok(match self {
            Self::LoggedOut => {
                "Chat login status: logged out. You can ask public NyxID questions. Send /login to sign in or create an account, then review and confirm linking this private chat."
            }
            Self::Pending => {
                "Chat login status: confirmation pending. Finish the account review on the NyxID website, then send its confirmation command in this private chat. Signing up or signing in on the website alone does not log this chat in. Send /logout to cancel, or /login for a new link."
            }
            Self::AwaitingConfirmation => "Chat login status: browser approved; waiting for confirmation here. Return to the approval page and tap Open Telegram to finish, or copy its /confirm command into this private chat. Send /login for a new review if the page is lost.",
            Self::Expired => "Chat login status: linking request expired. Send /login to start a new account review.",
            Self::LoggedIn => {
                "Chat login status: logged in. This private chat uses your verified NyxID account's NyxBot, existing access, memory and personal credits. Closing your browser does not log this chat out. Send /logout to disconnect this chat."
            }
            Self::Unavailable => {
                "Chat login status: personal access unavailable. Your account or this chat's authority is no longer available, or personal chat is paused. Send /logout to disconnect; /login can start a fresh review when personal chat is available."
            }
        }.into())
    }
}

/// No account labels, identifiers, confirmation codes or bearer material leave
/// this projection. The same evaluator runs again at the final send claim.
pub(crate) async fn status_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    update: &VerifiedManagerUpdate,
) -> AppResult<ChatLoginStatus> {
    use super::platform_personal_binding_service::{self as bindings, ResolvedPersonalSender};
    match bindings::resolve_sender_in_session(db, session, update).await? {
        ResolvedPersonalSender::Personal { .. } => return Ok(ChatLoginStatus::LoggedIn),
        ResolvedPersonalSender::Unavailable(_) => return Ok(ChatLoginStatus::Unavailable),
        ResolvedPersonalSender::Unlinked => {}
    }
    let controls = controls::get_in_session(db, session, &update.profile_id)
        .await?
        .ok_or_else(|| AppError::Conflict("Manager transport is unavailable".into()))?;
    let pending = db
        .collection::<mongodb::bson::Document>(crate::models::platform_link_intent::COLLECTION_NAME)
        .find_one(mongodb::bson::doc! {
            "platform_config_id": &update.profile_id,
            "manager_bot_id": &update.manager_bot_id,
            (scope::entry_key(&update.entry)):scope::entry_value(&update.entry),
            "manager_authority_epoch": controls.manager_authority_epoch,
            (scope::actor_key(&update.sender,false)):scope::actor_value(&update.sender),
            (scope::destination_key(&update.sender,false)):scope::destination_value(&update.sender),
        })
        .sort(mongodb::bson::doc! {"created_at":-1,"_id":-1})
        .session(&mut *session)
        .await?;
    let Some(intent) = pending else {
        return Ok(ChatLoginStatus::LoggedOut);
    };
    if intent.get_str("state").ok() == Some("expired")
        || (intent.get_str("state").ok() == Some("pending")
            && intent
                .get_datetime("expires_at")
                .is_ok_and(|at| at.to_chrono() <= chrono::Utc::now()))
    {
        return Ok(ChatLoginStatus::Expired);
    }
    if intent.get_str("state").ok() != Some("pending") {
        return Ok(ChatLoginStatus::LoggedOut);
    }
    if let Ok(id) = intent.get_str("current_challenge_id")
        && let Some(challenge) = db.collection::<mongodb::bson::Document>(crate::models::platform_link_intent::CHALLENGES)
            .find_one(mongodb::bson::doc! {"_id":id,"intent_id":intent.get_str("_id").unwrap_or_default()})
            .session(session).await?
    {
        if challenge.get_datetime("expires_at").is_ok_and(|at| at.to_chrono() <= chrono::Utc::now()) {
            return Ok(ChatLoginStatus::Expired);
        }
        if challenge.get_str("state").ok() == Some("awaiting_chat_confirmation") {
            return Ok(ChatLoginStatus::AwaitingConfirmation);
        }
    }
    Ok(ChatLoginStatus::Pending)
}

/// The send claim reuses the committed effect and serializes against proof
/// cancellation, binding replacement and account disablement. A stale effect
/// never acquires a transport credential.
pub(crate) async fn fence_effect_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    update: &VerifiedManagerUpdate,
    receipt_id: &str,
    effect: &ManagerCommandEffect,
) -> AppResult<bool> {
    use super::platform_personal_binding_service::{self as bindings, ResolvedPersonalSender};
    use mongodb::bson::{Document, doc};
    let row = controls::get_in_session(db, session, &update.profile_id)
        .await?
        .ok_or_else(|| AppError::Conflict("Manager transport is unavailable".into()))?;
    match effect {
        ManagerCommandEffect::Link { intent_id } => {
            if !row.personal_enabled || row.emergency_stopped || row.retired {
                return Ok(false);
            }
            Ok(db.collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
                .update_one(doc! {
                    "_id":intent_id, "initiation_ref":receipt_id,
                    "platform_config_id":&update.profile_id,"manager_bot_id":&update.manager_bot_id,
                    (scope::entry_key(&update.entry)):scope::entry_value(&update.entry),"manager_authority_epoch":row.manager_authority_epoch,
                    (scope::actor_key(&update.sender,false)):scope::actor_value(&update.sender),(scope::destination_key(&update.sender,false)):scope::destination_value(&update.sender),
                    "state":"pending","$expr":{"$gt":["$expires_at","$$NOW"]},
                },doc! {"$inc":{"command_delivery_fence":1_i64}})
                .session(session).await?.matched_count == 1)
        }
        ManagerCommandEffect::Confirmed {
            binding_id,
            binding_generation,
        }
        | ManagerCommandEffect::Management {
            binding_id,
            binding_generation,
            ..
        } => Ok(
            matches!(bindings::resolve_sender_in_session(db,session,update).await?,
                ResolvedPersonalSender::Personal {binding,..}
                    if binding.id == *binding_id && binding.generation == *binding_generation),
        ),
        ManagerCommandEffect::LoggedOut => {
            // Do not let a delayed logout acknowledgement cover a newer login
            // or pending review. The manager fence serializes new proofs too.
            for (collection, state) in [
                (
                    crate::models::platform_personal_binding::COLLECTION_NAME,
                    "active",
                ),
                (
                    crate::models::platform_link_intent::COLLECTION_NAME,
                    "pending",
                ),
            ] {
                let mut filter = doc! {
                    "platform_config_id":&update.profile_id,"manager_bot_id":&update.manager_bot_id,
                    (scope::entry_key(&update.entry)):scope::entry_value(&update.entry),"manager_authority_epoch":row.manager_authority_epoch,
                    (scope::actor_key(&update.sender,false)):scope::actor_value(&update.sender),(scope::destination_key(&update.sender,false)):scope::destination_value(&update.sender),"state":state,
                };
                if state == "pending" {
                    filter.insert("$expr", doc! {"$gt":["$expires_at","$$NOW"]});
                }
                if db
                    .collection::<Document>(collection)
                    .find_one(filter)
                    .session(&mut *session)
                    .await?
                    .is_some()
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        ManagerCommandEffect::Status { state } => {
            Ok(status_in_session(db, session, update).await? == *state)
        }
        ManagerCommandEffect::Help | ManagerCommandEffect::ManagementUnavailable => Ok(true),
    }
}

impl ManagerCommandEffect {
    /// Link bodies contain a one-use bearer and only exist in the current call.
    /// All other command prose is generated from typed committed metadata.
    pub(crate) fn static_reply(&self) -> Option<PersonalCommandReply> {
        Some(match self {
            Self::Link { .. } => return None,
            Self::Confirmed { .. } => PersonalCommandReply::Confirmed,
            Self::LoggedOut => PersonalCommandReply::Unlinked,
            Self::Status { state } => PersonalCommandReply::Status(*state),
            Self::Help => PersonalCommandReply::Help,
            Self::ManagementUnavailable => PersonalCommandReply::ManagementUnavailable,
            Self::Management { .. } => return None,
        })
    }
}

pub struct CommittedCommand {
    pub ingress: IngressClaim,
    pub manager: ManagerAuthority,
    pub sender: SenderIdentity,
    pub reply: PersonalCommandReply,
    pub effect: ManagerCommandEffect,
}

/// None is a replay: no new proof, execution or delivery may be produced.
pub async fn execute(
    db: &Database,
    key: &[u8],
    audit_key: &[u8],
    command: &OriginalManagerCommand,
    kind: PersonalCommandKind,
    lifecycle: &LifecycleProducer,
) -> AppResult<Option<CommittedCommand>> {
    let mut words = command.text().split_whitespace();
    let verb = words.next().and_then(|word| word.split('@').next());
    let valid = match &kind {
        PersonalCommandKind::Link => {
            matches!(verb, Some("/link" | "/login")) && words.next().is_none()
        }
        PersonalCommandKind::Unlink => {
            matches!(verb, Some("/unlink" | "/logout")) && words.next().is_none()
        }
        PersonalCommandKind::Status => {
            matches!(verb, Some("/status" | "/check")) && words.next().is_none()
        }
        PersonalCommandKind::Help => {
            matches!(verb, Some("/help" | "/start")) && words.next().is_none()
        }
        PersonalCommandKind::Confirm => {
            verb == Some("/confirm")
                && words
                    .next()
                    .is_some_and(|code| code.to_ascii_uppercase().starts_with("P-"))
                && words.next().is_none()
        }
        PersonalCommandKind::Management(selection) => {
            super::platform_manager_management::parse(
                command.text(),
                command
                    .text()
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.split_once('@').map(|(_, target)| target))
                    .unwrap_or(""),
            )
            .as_ref()
                == Some(selection)
        }
    };
    if !valid {
        return Err(AppError::ValidationError(
            "Invalid personal linking command".into(),
        ));
    }
    let claim = IngressClaim {
        receipt_id: uuid::Uuid::new_v4().to_string(),
        claim_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut session = db.client().start_session().await?;
    let result=session.start_transaction().and_run((db,key,audit_key,command,kind,lifecycle,claim), |session,(db,key,audit_key,command,kind,lifecycle,claim)| Box::pin(async move {
        let result:AppResult<_>=async {
            let update=command.update();
            if ingress::find_in_session(db,session,update).await?.is_some(){return Ok(None);}
            let row=controls::fence_manager_in_session(db,session,&update.profile_id,&update.manager_bot_id,&update.entry,Some(&update.observation_id)).await?
                .ok_or_else(|| AppError::Conflict("Manager transport is unavailable".into()))?;
            if matches!(kind, PersonalCommandKind::Link | PersonalCommandKind::Confirm) && (row.retired || row.emergency_stopped || !row.personal_enabled) {
                return Err(AppError::Conflict("Personal linking is currently unavailable".into()));
            }
            let (reply,effect,revocation)=match kind {
                PersonalCommandKind::Link=> {
                    if command.text().split_whitespace().next().and_then(|word| word.split('@').next()) == Some("/login") && status_in_session(db,session,update).await? == ChatLoginStatus::LoggedIn {
                        (PersonalCommandReply::Status(ChatLoginStatus::LoggedIn),ManagerCommandEffect::Status {state:ChatLoginStatus::LoggedIn},None)
                    } else {
                        let issued=intents::begin_original_in_session(db,session,key,command,&claim.receipt_id).await?;
                        let token=issued.token.ok_or_else(|| AppError::Conflict("Link was already issued; send /login again".into()))?;
                        (PersonalCommandReply::Link {token},ManagerCommandEffect::Link {intent_id:issued.intent_id},None)
                    }
                },
                PersonalCommandKind::Confirm=> {
                    let completed=intents::confirm_original_in_session(db,session,key,audit_key,command,*lifecycle).await?;
                    (PersonalCommandReply::Confirmed,ManagerCommandEffect::Confirmed {binding_id:completed.binding_id,binding_generation:completed.binding_generation},None)
                },
                PersonalCommandKind::Unlink=> {
                    let revocation=lifecycle.unlink_sender_in_session(db,session,command).await?;
                    (PersonalCommandReply::Unlinked,ManagerCommandEffect::LoggedOut,Some(revocation))
                },
                PersonalCommandKind::Status => {
                    let state=status_in_session(db,session,update).await?;
                    (PersonalCommandReply::Status(state),ManagerCommandEffect::Status {state},None)
                },
                PersonalCommandKind::Help => (PersonalCommandReply::Help,ManagerCommandEffect::Help,None),
                PersonalCommandKind::Management(_) if !matches!(update.entry,EntryIdentity::TelegramV1 {..}) =>
                    (PersonalCommandReply::ManagementUnavailable,ManagerCommandEffect::ManagementUnavailable,None),
                PersonalCommandKind::Management(selection) => {
                    use super::platform_personal_binding_service::{self as bindings,ResolvedPersonalSender};
                    let ResolvedPersonalSender::Personal {binding,..} = bindings::resolve_sender_in_session(db,session,update).await? else {
                        return Err(AppError::Conflict("Sign in to this chat before managing bots".into()));
                    };
                    let mut link_id = None;
                    let mut connect_token = None;
                    if matches!(selection, crate::models::platform_manager_management::ManagerManagementView::Onboarding) {
                        let created = super::channel_connect_link_service::create_personal_telegram_in_session(db,session,&binding.user_id).await?;
                        link_id = Some(created.link.id);
                        connect_token = Some(created.token);
                    }
                    let selection_ref = super::platform_manager_management::select_in_session(db,session,&binding,selection,&claim.receipt_id).await?;
                    (PersonalCommandReply::Management {connect_token},ManagerCommandEffect::Management {
                        binding_id:binding.id,binding_generation:binding.generation,selection:selection.clone(),connect_link_id:link_id,selection_ref,
                    },None)
                },
            };
            let manager=controls::policy(&row).manager;
            let admission=ingress::record_admission_in_session(db,session,update,&manager,claim,&ManagerAudience::Control).await?;
            if !matches!(admission,IngressAdmission::Claimed {..}) {
                return Err(AppError::Conflict("Command was already claimed".into()));
            }
            db.collection::<mongodb::bson::Document>(crate::models::platform_manager_ingress::COLLECTION_NAME)
                .update_one(mongodb::bson::doc!{"_id":&claim.receipt_id,"claim_id":&claim.claim_id,"audience.kind":"control"},
                    mongodb::bson::doc!{"$set":{"phase":"settled","control_effect":mongodb::bson::to_bson(&effect).map_err(|_|AppError::Internal("Control effect encoding failed".into()))?,"settled_at":mongodb::bson::DateTime::now()}}).session(&mut *session).await?;
            Ok(Some((CommittedCommand {ingress:claim.clone(),manager,sender:update.sender.clone(),reply,effect},revocation)))
        }.await;
        transaction_result(result)
    })).await.map_err(map_transaction_error);
    if result.is_err() {
        let mut reread = db.client().start_session().await?;
        if ingress::find_in_session(db, &mut reread, command.update())
            .await?
            .is_some()
        {
            return Ok(None);
        }
    }
    result.map(|committed| {
        committed.map(|(command, revocation)| {
            if let Some(revocation) = revocation {
                tracing::info!(
                    receipt_id = command.ingress.receipt_id,
                    revoked = revocation.revoked,
                    pending_stop_records = revocation.stop_effect_record_ids.len(),
                    "Personal unlink committed; exact stop recovery owns pending effects"
                );
            }
            command
        })
    })
}

impl PersonalCommandReply {
    pub fn render(
        &self,
        frontend_url: &str,
        entry: &EntryIdentity,
    ) -> AppResult<Zeroizing<String>> {
        let chat = super::platform_chat_presentation::ChatPresentation::for_entry(entry)?;
        Ok(Zeroizing::new(match self {
            Self::Link {token} => {
                let mut url=reqwest::Url::parse(frontend_url).map_err(|_|AppError::Internal("Frontend origin is unavailable".into()))?;
                url.set_path("/nyxbot/link");url.set_query(None);url.set_fragment(Some(&format!("intent_token={}",token.as_str())));
                format!("Review your NyxID account at {url}\nThis links your own NyxBot, its existing access and memory, and your personal billing. After approving on the web, return to this private chat with the confirmation command. Never share the link or enter a password here.")
            },
            Self::ManagementUnavailable=>"These chat bot-setup controls are available in Telegram. Open NyxID to manage your owned bots; your personal NyxBot can still use your normal account tools here.".into(),
            Self::Confirmed if chat != super::platform_chat_presentation::ChatPresentation::Telegram => format!("Your personal NyxBot is linked. New questions here use your account's existing NyxBot access and credits. This saved thread will appear in NyxID. Send {} to check login or {} to disconnect. {}",chat.status(),chat.logout(),chat.question_instruction()),
            Self::Confirmed=>"Your personal NyxBot is linked. New messages here use your account's existing NyxBot access and credits. This saved thread will appear in NyxID. Send /status to check login or /logout to disconnect (/unlink also works).".into(),
            Self::Unlinked if chat != super::platform_chat_presentation::ChatPresentation::Telegram => format!("This chat is disconnected and pending login confirmations are cancelled. You can ask public NyxID questions again. Your saved history remains in NyxID; independently started work may continue there. Send {} to reconnect. {}",chat.login(),chat.question_instruction()),
            Self::Unlinked=>"This chat is disconnected and pending login confirmations are cancelled. You can ask public NyxID questions again. Your saved history remains in NyxID; independently started work may continue there. Send /login to reconnect (/link also works).".into(),
            Self::Status(status)=>status.text(entry)?,
            Self::Help if chat != super::platform_chat_presentation::ChatPresentation::Telegram => format!("Ask me about NyxID or how to get started. Send {} to sign in or create an account and review linking your own NyxBot. Complete the website review and return here with its confirmation command; signup alone does not log this chat in. {} checks this chat's live login. {} disconnects this chat and cancels pending confirmations. Browser sign-out does not disconnect this chat. Once linked, questions use your own NyxBot access, memory and personal credits, with history and approval cards available in NyxID. Your linked NyxBot can use your normal account tools to manage owned bots. Deterministic bot creation and setup controls are available in Telegram. {}",chat.login(),chat.status(),chat.logout(),chat.question_instruction()),
            Self::Help=>"Ask me about NyxID or how to get started. Send /login to sign in or create an account and review linking your own NyxBot. Complete the website review and return here with its confirmation command; signup alone does not log this chat in. /status checks this chat's live login. /logout disconnects this chat and cancels pending confirmations. /link and /unlink also work. Once linked, messages use your own NyxBot access, memory and personal credits, with history and approval cards available in NyxID. For Telegram bot creation and setup, send /onboarding; /setups lists onboarding progress. You can also ask your linked NyxBot to list and manage your bots.".into(),
            Self::Management { .. } => return Err(AppError::Internal("Management replies require live rendering".into())),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_commands_do_not_consume_legacy_or_foreign_commands() {
        for (verb, kind) in [
            ("/login", PersonalCommandKind::Link),
            ("/logout", PersonalCommandKind::Unlink),
            ("/status", PersonalCommandKind::Status),
            ("/help", PersonalCommandKind::Help),
            ("/start", PersonalCommandKind::Help),
        ] {
            assert_eq!(personal_command(verb, "manager_bot"), Some(kind));
            assert_eq!(
                personal_command(&format!("{verb}@OTHER_bot"), "manager_bot"),
                None
            );
        }
        assert_eq!(
            personal_command("/start native-creation-payload", "manager_bot"),
            None
        );
        assert_eq!(
            personal_command("/confirm P-1234-5678", "manager_bot"),
            Some(PersonalCommandKind::Confirm)
        );
        assert_eq!(
            personal_command("/confirm ONBOARD-CODE", "manager_bot"),
            None
        );
        assert_eq!(personal_command("/link@OTHER_bot", "manager_bot"), None);
        assert_eq!(
            personal_command("/link@MANAGER_bot", "manager_bot"),
            Some(PersonalCommandKind::Link)
        );
        assert_eq!(personal_command("/setups", "manager_bot"), None);
    }
    #[test]
    fn intent_is_only_in_first_party_fragment() {
        let reply = PersonalCommandReply::Link {
            token: Zeroizing::new("nyx_pli_example".into()),
        };
        let text = reply
            .render(
                "https://example.test/old?query=discard#discard",
                &EntryIdentity::TelegramV1 {
                    telegram_bot_id: 100,
                },
            )
            .unwrap();
        assert!(text.contains("https://example.test/nyxbot/link#intent_token=nyx_pli_example"));
        assert!(!text.contains("?query="));
    }

    #[test]
    fn status_and_help_explain_chat_auth_without_exposing_account_information() {
        for status in [
            ChatLoginStatus::LoggedOut,
            ChatLoginStatus::Pending,
            ChatLoginStatus::LoggedIn,
            ChatLoginStatus::Unavailable,
        ] {
            let text = PersonalCommandReply::Status(status)
                .render(
                    "https://example.test",
                    &EntryIdentity::TelegramV1 {
                        telegram_bot_id: 100,
                    },
                )
                .unwrap();
            assert!(text.starts_with("Chat login status:"));
            assert!(!text.contains("nyx_pli_"));
            assert!(!text.contains("https://"));
        }
        let help = PersonalCommandReply::Help
            .render(
                "https://example.test",
                &EntryIdentity::TelegramV1 {
                    telegram_bot_id: 100,
                },
            )
            .unwrap();
        for command in ["/login", "/status", "/logout", "/onboarding", "/setups"] {
            assert!(help.contains(command));
        }
        assert!(help.contains("signup alone does not log this chat in"));
    }
}
