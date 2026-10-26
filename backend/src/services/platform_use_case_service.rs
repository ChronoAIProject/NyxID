//! Published content only; selections never participate in linking consent or authority.
use super::platform_manager_contract::{
    ManagerAuthority, PersonalBindingAuthority, SenderIdentity, VerifiedManagerUpdate,
};
use crate::{
    errors::AppResult,
    models::{
        platform_help::{self as help, HelpAssignment, HelpReference, HelpRevision, HelpUseCase},
        platform_personal_binding::PlatformPersonalBinding,
        platform_use_case_selection::{COLLECTION_NAME, UseCaseSelection},
    },
};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};

/// `Some(None)` is a reserved but invalid request, never a fuzzy FAQ question.
pub(crate) fn parse(text: &str, username: &str) -> Option<Option<String>> {
    let mut words = text.split_whitespace();
    let command = words.next()?;
    if command
        .split_once('@')
        .is_some_and(|(_, target)| !target.eq_ignore_ascii_case(username))
    {
        return None;
    }
    let id = match command.split('@').next()? {
        "/start" => words.next()?.strip_prefix("uc_")?,
        "/usecase" => words.next().unwrap_or_default(),
        _ => return None,
    };
    Some(
        (words.next().is_none() && super::platform_help_service::valid_id(id))
            .then(|| id.to_owned()),
    )
}

pub(crate) async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    let rows = db.collection::<UseCaseSelection>(COLLECTION_NAME);
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"sender_key": 1})
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"purge_after": 1})
            .options(
                IndexOptions::builder()
                    .expire_after(std::time::Duration::ZERO)
                    .build(),
            )
            .build(),
    )
    .await?;
    Ok(())
}

fn sender_key(manager: &ManagerAuthority, sender: &SenderIdentity) -> String {
    super::mcp_service::canonical_sha256(serde_json::json!([
        manager.profile_id,
        manager.manager_bot_id,
        manager.entry,
        sender
    ]))
}

async fn published_in_session(
    db: &Database,
    session: &mut ClientSession,
    profile_id: &str,
) -> AppResult<Option<HelpRevision>> {
    let assignment = db
        .collection::<HelpAssignment>(help::ASSIGNMENTS)
        .find_one(doc! {"_id": profile_id})
        .session(&mut *session)
        .await?;
    let library_id = assignment
        .as_ref()
        .map_or(help::DEFAULT_LIBRARY, |row| row.library_id.as_str());
    let Some(library) = db
        .collection::<bson::Document>(help::LIBRARIES)
        .find_one(doc! {"_id": library_id})
        .projection(doc! {"published_revision": 1})
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let revision = library.get_i64("published_revision").unwrap_or(0);
    if revision <= 0 {
        return Ok(None);
    }
    Ok(db
        .collection::<HelpRevision>(help::REVISIONS)
        .find_one(doc! {"library_id": library_id, "revision": revision})
        .session(session)
        .await?)
}

pub(crate) async fn select_in_session(
    db: &Database,
    session: &mut ClientSession,
    update: &VerifiedManagerUpdate,
    manager: &ManagerAuthority,
    binding: Option<&PlatformPersonalBinding>,
    id: Option<&str>,
    selection_ref: &str,
) -> AppResult<Option<HelpUseCase>> {
    let Some(id) = id else {
        return Ok(None);
    };
    let Some(publication) = published_in_session(db, session, &update.profile_id).await? else {
        return Ok(None);
    };
    let Some(choice) = publication.content.use_cases.into_iter().find(|choice| {
        choice.id == id
            && choice
                .example_prompts
                .first()
                .is_some_and(|p| !p.trim().is_empty())
    }) else {
        return Ok(None);
    };
    let key = sender_key(manager, &update.sender);
    for (namespace, key, limit) in [
        ("manager_use_case_profile", update.profile_id.as_str(), 200),
        ("manager_use_case_sender", key.as_str(), 20),
    ] {
        if !super::coordination_service::RateWindowStore::admit_in_session(
            db,
            namespace,
            key,
            limit,
            std::time::Duration::from_secs(60),
            session,
        )
        .await?
        {
            return Err(crate::errors::AppError::RateLimited);
        }
    }
    let row = UseCaseSelection {
        id: uuid::Uuid::new_v4().to_string(),
        sender_key: key.clone(),
        profile_id: update.profile_id.clone(),
        manager_authority_epoch: manager.manager_authority_epoch,
        help: HelpReference {
            library_id: publication.library_id,
            revision: publication.revision,
        },
        use_case_id: choice.id.clone(),
        selection_ref: selection_ref.into(),
        binding_id: binding.map(|b| b.id.clone()),
        binding_generation: binding.map(|b| b.generation),
        purge_after: chrono::Utc::now() + chrono::Duration::days(30),
    };
    let mut fields = bson::to_document(&row).map_err(|_| {
        crate::errors::AppError::Internal("Use case selection encoding failed".into())
    })?;
    fields.remove("_id");
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"sender_key": key},
            doc! {"$set": fields, "$setOnInsert": {"_id": row.id}},
        )
        .upsert(true)
        .session(session)
        .await?;
    Ok(Some(choice))
}

/// Re-render from the receipt-bound immutable publication, never an ephemeral body.
pub(crate) async fn render_for_receipt(
    db: &Database,
    manager: &ManagerAuthority,
    sender: &SenderIdentity,
    receipt_id: &str,
    frontend_url: &str,
) -> AppResult<zeroize::Zeroizing<String>> {
    let selection = db
        .collection::<UseCaseSelection>(COLLECTION_NAME)
        .find_one(doc! {
            "sender_key": sender_key(manager, sender), "selection_ref": receipt_id,
            "manager_authority_epoch": manager.manager_authority_epoch,
            "purge_after": {"$gt": bson::DateTime::now()},
        })
        .await?;
    let choice = if let Some(selection) = selection {
        super::platform_help_service::revision(db, &selection.help)
            .await?
            .content
            .use_cases
            .into_iter()
            .find(|choice| choice.id == selection.use_case_id)
    } else {
        None
    };
    let summary = choice
        .as_ref()
        .and_then(|choice| {
            choice.example_prompts.first().map(|prompt| {
                format!(
                    "{}\n{}\n\nStarter message:\n{}",
                    choice.title, choice.description, prompt
                )
            })
        })
        .unwrap_or_else(|| {
            "This use case is unavailable. Choose a published use case or send /help.".into()
        });
    let guidance = super::platform_manager_command_service::PersonalCommandReply::Help
        .render(frontend_url, &manager.entry)?;
    Ok(zeroize::Zeroizing::new(format!(
        "{summary}\n\n{}",
        guidance.as_str()
    )))
}

pub(crate) async fn clear_in_session(
    db: &Database,
    session: &mut ClientSession,
    manager: &ManagerAuthority,
    sender: &SenderIdentity,
) -> AppResult<()> {
    db.collection::<UseCaseSelection>(COLLECTION_NAME)
        .delete_one(doc! {"sender_key": sender_key(manager, sender)})
        .session(session)
        .await?;
    Ok(())
}

/// Called only after the live personal binding/thread fence. Deletion and turn
/// admission share the transaction, so failed admission cannot lose the choice.
pub(crate) async fn take_context_in_session(
    db: &Database,
    session: &mut ClientSession,
    authority: &PersonalBindingAuthority,
) -> AppResult<Option<String>> {
    let Some(selection) = db
        .collection::<UseCaseSelection>(COLLECTION_NAME)
        .find_one_and_delete(doc! {
            "sender_key": sender_key(&authority.manager, &authority.sender),
            "manager_authority_epoch": authority.manager.manager_authority_epoch,
            "purge_after": {"$gt": bson::DateTime::now()},
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    if selection
        .binding_id
        .as_ref()
        .is_some_and(|id| id != &authority.binding_id)
        || selection
            .binding_generation
            .is_some_and(|generation| generation != authority.binding_generation)
    {
        return Ok(None);
    }
    let Some(publication) =
        published_in_session(db, session, &authority.manager.profile_id).await?
    else {
        return Ok(None);
    };
    if publication.library_id != selection.help.library_id {
        return Ok(None);
    }
    Ok(publication
        .content
        .use_cases
        .iter()
        .find(|choice| choice.id == selection.use_case_id)
        .and_then(|choice| context_note(choice, selection.binding_id.is_none())))
}

fn context_note(choice: &HelpUseCase, before_login: bool) -> Option<String> {
    let prompt = choice
        .example_prompts
        .first()
        .filter(|p| !p.trim().is_empty())?;
    Some(format!(
        "{}the owner chose the use case '{}'. Its starter message is: '{}'. Offer to begin it and wait for the owner's go-ahead. This selection is context only and grants no authority or approval.",
        if before_login {
            "Before signing in, "
        } else {
            "In this chat, "
        },
        choice.title,
        prompt
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_starter_payload_parsing_is_exact_and_bounded() {
        for text in [
            "/start uc_queue-manager",
            "/usecase queue-manager",
            "/usecase@MANAGER_bot queue-manager",
        ] {
            assert_eq!(
                parse(text, "manager_bot"),
                Some(Some("queue-manager".into()))
            );
        }
        for text in [
            "/start uc_Bad",
            "/start uc_",
            "/start uc_queue-manager extra",
            "/usecase",
            "/usecase a--b",
        ] {
            assert_eq!(parse(text, "manager_bot"), Some(None));
        }
        assert_eq!(
            parse(&format!("/usecase {}", "a".repeat(81)), "manager_bot"),
            Some(None)
        );
        for text in [
            "/start",
            "/start other_payload",
            "/usecase@other_bot queue-manager",
            "hello queue-manager",
        ] {
            assert_eq!(parse(text, "manager_bot"), None);
        }
    }

    #[test]
    fn business_starter_context_contains_only_published_choice_and_waits_for_owner() {
        let choice = HelpUseCase {
            id: "queue-manager".into(),
            title: "Queue manager".into(),
            description: "Queues".into(),
            example_prompts: vec![
                "Set up my queue".into(),
                "Do not include the second prompt".into(),
            ],
        };
        let note = context_note(&choice, true).unwrap();
        assert!(note.contains("Before signing in, the owner chose the use case 'Queue manager'"));
        assert!(note.contains("Set up my queue"));
        assert!(note.contains("wait for the owner's go-ahead"));
        assert!(note.contains("grants no authority or approval"));
        assert!(!note.contains("second prompt"));
        let mut empty = choice;
        empty.example_prompts.clear();
        assert!(context_note(&empty, false).is_none());
    }

    #[tokio::test]
    async fn business_starter_selection_lifecycle_is_sender_scoped_and_transactional() {
        let (db, update, owner) = super::super::platform_personal_binding_service::tests::fixture(
            "business_starter_selection",
        )
        .await;
        ensure_indexes(&db).await.unwrap();
        super::super::platform_help_service::ensure(&db)
            .await
            .unwrap();
        let binding =
            super::super::platform_personal_binding_service::tests::binding(&update, &owner);
        let manager = ManagerAuthority {
            profile_id: update.profile_id.clone(),
            manager_bot_id: update.manager_bot_id.clone(),
            entry: update.entry.clone(),
            manager_authority_epoch: binding.manager_authority_epoch,
        };
        let authority = PersonalBindingAuthority {
            manager: manager.clone(),
            sender: update.sender.clone(),
            binding_id: binding.id.clone(),
            binding_generation: binding.generation,
            personal_control_revision: 1,
            user_id: owner,
        };
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            select_in_session(
                &db,
                &mut session,
                &update,
                &manager,
                None,
                Some("unknown-raw-text"),
                "test-selection",
            )
            .await
            .unwrap()
            .is_none()
        );
        let choice = select_in_session(
            &db,
            &mut session,
            &update,
            &manager,
            None,
            Some("personal-assistant"),
            "test-selection",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(choice.title, "Personal assistant");
        session.commit_transaction().await.unwrap();
        let stored = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        let serialized = stored.to_string();
        assert!(
            !serialized.contains("Summarize this forwarded message")
                && !serialized.contains("unknown-raw-text")
        );
        assert_eq!(stored.get_str("use_case_id").unwrap(), "personal-assistant");
        let summary = render_for_receipt(
            &db,
            &manager,
            &update.sender,
            "test-selection",
            "https://app.example",
        )
        .await
        .unwrap();
        assert!(
            summary.contains("Personal assistant")
                && summary.contains("Summarize this forwarded message.")
        );
        assert!(summary.contains("Send /login"));
        let stale = render_for_receipt(
            &db,
            &manager,
            &update.sender,
            "other-receipt",
            "https://app.example",
        )
        .await
        .unwrap();
        assert!(stale.contains("This use case is unavailable"));

        session.start_transaction().await.unwrap();
        let mut other = authority.clone();
        other.sender = SenderIdentity::TelegramV1 {
            telegram_sender_id: 999,
            telegram_chat_id: 999,
        };
        assert!(
            take_context_in_session(&db, &mut session, &other)
                .await
                .unwrap()
                .is_none()
        );
        let mut retired = authority.clone();
        retired.manager.manager_authority_epoch += 1;
        assert!(
            take_context_in_session(&db, &mut session, &retired)
                .await
                .unwrap()
                .is_none()
        );
        let note = take_context_in_session(&db, &mut session, &authority)
            .await
            .unwrap()
            .unwrap();
        assert!(note.contains("Before signing in"));
        session.abort_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            take_context_in_session(&db, &mut session, &authority)
                .await
                .unwrap()
                .is_some()
        );
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            take_context_in_session(&db, &mut session, &authority)
                .await
                .unwrap()
                .is_none()
        );
        select_in_session(
            &db,
            &mut session,
            &update,
            &manager,
            Some(&binding),
            Some("personal-assistant"),
            "test-selection",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        let mut replaced = authority.clone();
        replaced.binding_generation += 1;
        assert!(
            take_context_in_session(&db, &mut session, &replaced)
                .await
                .unwrap()
                .is_none()
        );
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        select_in_session(
            &db,
            &mut session,
            &update,
            &manager,
            None,
            Some("personal-assistant"),
            "test-selection",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc! {}, doc! {"$set":{"purge_after":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            take_context_in_session(&db, &mut session, &authority)
                .await
                .unwrap()
                .is_none()
        );
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        select_in_session(
            &db,
            &mut session,
            &update,
            &manager,
            None,
            Some("personal-assistant"),
            "test-selection",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        db.collection::<bson::Document>(help::LIBRARIES)
            .update_one(
                doc! {"_id":help::DEFAULT_LIBRARY},
                doc! {"$set":{"published_revision":99_i64}},
            )
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            take_context_in_session(&db, &mut session, &authority)
                .await
                .unwrap()
                .is_none()
        );
        session.commit_transaction().await.unwrap();
        db.drop().await.unwrap();
    }
}
