//! Reserved personal commands commit with the ingress receipt, never an AI turn.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersonalCommandKind {
    Link,
    Confirm,
    Unlink,
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
        "/link" => Some(PersonalCommandKind::Link),
        "/unlink" => Some(PersonalCommandKind::Unlink),
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
    Link { token: Zeroizing<String> },
    Confirmed,
    Unlinked,
}

pub struct CommittedCommand {
    pub ingress: IngressClaim,
    pub manager: ManagerAuthority,
    pub sender_id: i64,
    pub chat_id: i64,
    pub reply: PersonalCommandReply,
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
    let valid = match kind {
        PersonalCommandKind::Link => verb == Some("/link") && words.next().is_none(),
        PersonalCommandKind::Unlink => verb == Some("/unlink") && words.next().is_none(),
        PersonalCommandKind::Confirm => {
            verb == Some("/confirm")
                && words
                    .next()
                    .is_some_and(|code| code.to_ascii_uppercase().starts_with("P-"))
                && words.next().is_none()
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
            let row=controls::fence_manager_in_session(db,session,&update.profile_id,&update.manager_bot_id,update.telegram_bot_id,Some(&update.observation_id)).await?
                .ok_or_else(|| AppError::Conflict("Manager transport is unavailable".into()))?;
            if *kind!=PersonalCommandKind::Unlink && (row.retired || row.emergency_stopped || !row.personal_enabled) {
                return Err(AppError::Conflict("Personal linking is currently unavailable".into()));
            }
            let (reply,revocation)=match kind {
                PersonalCommandKind::Link=> {
                    let issued=intents::begin_original_in_session(db,session,key,command,&claim.receipt_id).await?;
                    let token=issued.token.ok_or_else(|| AppError::Conflict("Link was already issued; send /link again".into()))?;
                    (PersonalCommandReply::Link {token},None)
                },
                PersonalCommandKind::Confirm=> {
                    intents::confirm_original_in_session(db,session,key,audit_key,command,*lifecycle).await?;
                    (PersonalCommandReply::Confirmed,None)
                },
                PersonalCommandKind::Unlink=> {
                    let revocation=lifecycle.unlink_sender_in_session(db,session,command).await?;
                    (PersonalCommandReply::Unlinked,Some(revocation))
                }
            };
            let manager=controls::policy(&row).manager;
            let admission=ingress::record_admission_in_session(db,session,update,&manager,claim,&ManagerAudience::Control).await?;
            if !matches!(admission,IngressAdmission::Claimed {..}) {
                return Err(AppError::Conflict("Command was already claimed".into()));
            }
            db.collection::<mongodb::bson::Document>(crate::models::platform_manager_ingress::COLLECTION_NAME)
                .update_one(mongodb::bson::doc!{"_id":&claim.receipt_id,"claim_id":&claim.claim_id,"audience.kind":"control"},
                    mongodb::bson::doc!{"$set":{"phase":"settled","settled_at":mongodb::bson::DateTime::now()}}).session(&mut *session).await?;
            Ok(Some((CommittedCommand {ingress:claim.clone(),manager,sender_id:update.sender_id,chat_id:update.chat_id,reply},revocation)))
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
    pub fn render(&self, frontend_url: &str) -> AppResult<Zeroizing<String>> {
        Ok(Zeroizing::new(match self {
            Self::Link {token} => {
                let mut url=reqwest::Url::parse(frontend_url).map_err(|_|AppError::Internal("Frontend origin is unavailable".into()))?;
                url.set_path("/nyxbot/link");url.set_query(None);url.set_fragment(Some(&format!("intent_token={}",token.as_str())));
                format!("Review your NyxID account at {url}\nThis links your own NyxBot, its existing access and memory, and your personal billing. After approving on the web, return to this private chat with the confirmation command. Never share the link or enter a password here.")
            },
            Self::Confirmed=>"Your personal NyxBot is linked. New messages here use your account's existing NyxBot access and credits. This Telegram thread will appear in NyxID. Send /unlink to disconnect.".into(),
            Self::Unlinked=>"This Telegram chat is disconnected. Your saved history remains in NyxID; independently started work may continue there. Send /link to reconnect.".into(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_commands_do_not_consume_legacy_or_foreign_commands() {
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
            .render("https://example.test/old?query=discard#discard")
            .unwrap();
        assert!(text.contains("https://example.test/nyxbot/link#intent_token=nyx_pli_example"));
        assert!(!text.contains("?query="));
    }
}
