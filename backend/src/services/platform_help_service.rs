//! Versioned public content; it never confers execution or account authority.
use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::{
    errors::{AppError, AppResult},
    models::platform_help::*,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, doc},
    options::{IndexOptions, ReturnDocument},
};

fn conflict() -> AppError {
    AppError::Conflict("Help changed. Reload and review the current version.".into())
}
fn missing() -> AppError {
    AppError::NotFound("Help library or revision not found".into())
}
pub fn seed_content() -> AppResult<HelpContent> {
    Ok(HelpContent {
        articles: serde_json::from_str(include_str!("../../content/platform-public-faq.json"))
            .map_err(|_| AppError::Internal("Public help unavailable".into()))?,
        use_cases: vec![
            HelpUseCase {
                id: "personal-assistant".into(),
                title: "Personal assistant".into(),
                description: "Use your own NyxBot from a linked private chat.".into(),
                example_prompts: vec!["Summarize this forwarded message.".into()],
            },
            HelpUseCase {
                id: "restaurant-guide".into(),
                title: "Restaurant guide".into(),
                description: "Share reviewed restaurant facts and request links with customers."
                    .into(),
                example_prompts: vec!["Help me prepare a restaurant information bot.".into()],
            },
        ],
    })
}
pub(crate) fn valid_id(s: &str) -> bool {
    s.len() <= 80
        && s.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

pub fn validate(content: &HelpContent) -> AppResult<()> {
    let invalid = || {
        AppError::ValidationError("Help supports up to 30 articles and 20 use cases, with unique slugs, bounded text and at most 16 KiB total.".into())
    };
    let text = |s: &str, limit: usize| {
        !s.trim().is_empty()
            && s.len() <= limit
            && !s
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    };
    let mut ids = std::collections::HashSet::new();
    if content.articles.is_empty() || content.articles.len() > 30 || content.use_cases.len() > 20 {
        return Err(invalid());
    }
    for a in &content.articles {
        if !valid_id(&a.id)
            || !ids.insert(&a.id)
            || !text(&a.title, 100)
            || !text(&a.body, 6000)
            || a.tags.len() > 8
            || a.tags.iter().any(|t| !text(t, 40))
        {
            return Err(invalid());
        }
    }
    for a in &content.use_cases {
        if !valid_id(&a.id)
            || !ids.insert(&a.id)
            || !text(&a.title, 100)
            || !text(&a.description, 2000)
            || a.example_prompts.len() > 8
            || a.example_prompts.iter().any(|t| !text(t, 500))
        {
            return Err(invalid());
        }
    }
    if serde_json::to_vec(content).map_err(|_| invalid())?.len() > 16 * 1024 {
        return Err(invalid());
    }
    Ok(())
}
pub async fn ensure(db: &Database) -> AppResult<()> {
    db.collection::<HelpRevision>(REVISIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"library_id":1,"revision":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    seed(db).await
}
async fn seed(db: &Database) -> AppResult<()> {
    if db
        .collection::<HelpLibrary>(LIBRARIES)
        .find_one(doc! {"_id":DEFAULT_LIBRARY})
        .await?
        .is_some()
    {
        return Ok(());
    }
    let content = seed_content()?;
    let transaction_db = db.clone();
    let mut session = db.client().start_session().await?;
    let result = session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &transaction_db;
            transaction_result(
                async {
                    if db
                        .collection::<HelpLibrary>(LIBRARIES)
                        .find_one(doc! {"_id":DEFAULT_LIBRARY})
                        .session(&mut *session)
                        .await?
                        .is_none()
                    {
                        insert_library(
                            db,
                            session,
                            DEFAULT_LIBRARY,
                            "Shared BotFather help",
                            "system",
                            &content,
                        )
                        .await?;
                    }
                    Ok(())
                }
                .await,
            )
        })
        .await
        .map_err(map_transaction_error);
    if let Err(AppError::DatabaseError(error)) = &result
        && super::telegram_new_service::is_duplicate_key_error(error)
    {
        // Another replica may have committed the seed after our snapshot.
        // Its transaction must have created both the library and revision.
        let library = get(db, DEFAULT_LIBRARY).await?;
        revision(
            db,
            &HelpReference {
                library_id: library.id,
                revision: library.published_revision,
            },
        )
        .await?;
        return Ok(());
    }
    result
}
async fn insert_library(
    db: &Database,
    session: &mut ClientSession,
    id: &str,
    name: &str,
    actor: &str,
    content: &HelpContent,
) -> AppResult<HelpLibrary> {
    let now = Utc::now();
    let row = HelpLibrary {
        id: id.into(),
        name: name.into(),
        version: 1,
        published_revision: 1,
        draft: content.clone(),
        updated_at: now,
    };
    db.collection::<HelpRevision>(REVISIONS)
        .insert_one(HelpRevision {
            id: uuid::Uuid::new_v4().to_string(),
            library_id: id.into(),
            revision: 1,
            content: content.clone(),
            published_by: actor.into(),
            published_at: now,
        })
        .session(&mut *session)
        .await?;
    db.collection::<HelpLibrary>(LIBRARIES)
        .insert_one(&row)
        .session(session)
        .await?;
    Ok(row)
}
pub async fn list(db: &Database) -> AppResult<Vec<HelpLibrary>> {
    seed(db).await?;
    Ok(db
        .collection::<HelpLibrary>(LIBRARIES)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}
pub async fn get(db: &Database, id: &str) -> AppResult<HelpLibrary> {
    db.collection::<HelpLibrary>(LIBRARIES)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(missing)
}
pub async fn history(db: &Database, id: &str, before: Option<i64>) -> AppResult<Vec<HelpRevision>> {
    let mut f = doc! {"library_id":id};
    if let Some(n) = before {
        f.insert("revision", doc! {"$lt":n});
    }
    Ok(db
        .collection::<HelpRevision>(REVISIONS)
        .find(f)
        .sort(doc! {"revision":-1})
        .limit(50)
        .await?
        .try_collect()
        .await?)
}
pub async fn revision(db: &Database, reference: &HelpReference) -> AppResult<HelpRevision> {
    db.collection::<HelpRevision>(REVISIONS)
        .find_one(doc! {"library_id":&reference.library_id,"revision":reference.revision})
        .await?
        .ok_or_else(missing)
}
pub async fn published(db: &Database, profile: &str) -> AppResult<HelpRevision> {
    let assignment = db
        .collection::<HelpAssignment>(ASSIGNMENTS)
        .find_one(doc! {"_id":profile})
        .await?;
    if assignment.is_none() {
        seed(db).await?;
    }
    let library = get(
        db,
        assignment
            .as_ref()
            .map_or(DEFAULT_LIBRARY, |a| a.library_id.as_str()),
    )
    .await?;
    revision(
        db,
        &HelpReference {
            library_id: library.id,
            revision: library.published_revision,
        },
    )
    .await
}
pub async fn create(db: &Database, name: &str, actor: &str) -> AppResult<HelpLibrary> {
    if name.trim().is_empty() || name.chars().count() > 100 {
        return Err(AppError::ValidationError(
            "Use a library name of 1–100 characters".into(),
        ));
    }
    seed(db).await?;
    let content = seed_content()?;
    let id = uuid::Uuid::new_v4().to_string();
    let db = db.clone();
    let name = name.trim().to_owned();
    let actor = actor.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transaction_result(
                async {
                    super::platform_bot_config_service::fence_admin(&db, &actor, session).await?;
                    db.collection::<bson::Document>(LIBRARIES)
                        .update_one(
                            doc! {"_id":DEFAULT_LIBRARY},
                            doc! {"$inc":{"library_fence":1_i64}},
                        )
                        .session(&mut *session)
                        .await?;
                    if db
                        .collection::<HelpLibrary>(LIBRARIES)
                        .count_documents(doc! {})
                        .session(&mut *session)
                        .await?
                        >= 100
                    {
                        return Err(AppError::Conflict(
                            "At most 100 shared help libraries are supported.".into(),
                        ));
                    }
                    insert_library(&db, session, &id, &name, &actor, &content).await
                }
                .await,
            )
        })
        .await
        .map_err(map_transaction_error)
}
pub async fn save(
    db: &Database,
    id: &str,
    version: i64,
    content: &HelpContent,
    actor: &str,
) -> AppResult<HelpLibrary> {
    validate(content)?;
    let (db, id, content, actor) = (db.clone(), id.to_owned(), content.clone(), actor.to_owned());
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| transaction_result(async {
        super::platform_bot_config_service::fence_admin(&db, &actor, session).await?;
        db.collection::<HelpLibrary>(LIBRARIES).find_one_and_update(
            doc!{"_id":&id,"version":version},doc!{"$set":{"draft":bson::to_bson(&content).map_err(|_|conflict())?,"updated_at":bson::DateTime::now()},"$inc":{"version":1_i64}}
        ).return_document(ReturnDocument::After).session(session).await?.ok_or_else(conflict)
    }.await)).await.map_err(map_transaction_error)
}
pub async fn publish(
    db: &Database,
    id: &str,
    version: i64,
    actor: &str,
    rollback: Option<i64>,
) -> AppResult<HelpLibrary> {
    let db = db.clone();
    let id = id.to_owned();
    let actor = actor.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session|transaction_result(async {
        super::platform_bot_config_service::fence_admin(&db, &actor, session).await?;
        let row=db.collection::<HelpLibrary>(LIBRARIES).find_one(doc!{"_id":&id,"version":version}).session(&mut *session).await?.ok_or_else(conflict)?;
        let content=if let Some(rev)=rollback{db.collection::<HelpRevision>(REVISIONS).find_one(doc!{"library_id":&id,"revision":rev}).session(&mut *session).await?.ok_or_else(missing)?.content}else{row.draft};
        validate(&content)?;
        let revision=row.published_revision.checked_add(1).ok_or_else(conflict)?;
        db.collection::<HelpRevision>(REVISIONS).insert_one(HelpRevision{id:uuid::Uuid::new_v4().to_string(),library_id:id.clone(),revision,content:content.clone(),published_by:actor.clone(),published_at:Utc::now()}).session(&mut *session).await?;
        db.collection::<HelpLibrary>(LIBRARIES).find_one_and_update(doc!{"_id":&id,"version":version},doc!{"$set":{"draft":bson::to_bson(&content).map_err(|_|conflict())?,"published_revision":revision,"updated_at":bson::DateTime::now()},"$inc":{"version":1_i64}}).return_document(ReturnDocument::After).session(session).await?.ok_or_else(conflict)
    }.await)).await.map_err(map_transaction_error)
}
pub async fn assignment(db: &Database, profile: &str) -> AppResult<HelpAssignment> {
    Ok(db
        .collection::<HelpAssignment>(ASSIGNMENTS)
        .find_one(doc! {"_id":profile})
        .await?
        .unwrap_or(HelpAssignment {
            id: profile.into(),
            library_id: DEFAULT_LIBRARY.into(),
            version: 0,
        }))
}
pub async fn assign(
    db: &Database,
    profile: &str,
    generation: i64,
    version: i64,
    library: &str,
    actor: &str,
) -> AppResult<HelpAssignment> {
    get(db, library).await?;
    let actor = actor.to_owned();
    let db = db.clone();
    let profile = profile.to_owned();
    let library = library.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session|transaction_result(async {
        super::platform_bot_config_service::fence_admin(&db, &actor, session).await?;
        let profiles=db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME);
        if profiles.update_one(doc!{"_id":&profile,"generation":generation,"deleted_at":bson::Bson::Null},doc!{"$inc":{"help_fence":1_i64}}).session(&mut *session).await?.matched_count!=1{return Err(conflict());}
        let row=HelpAssignment{id:profile.clone(),library_id:library.clone(),version:version.checked_add(1).ok_or_else(conflict)?};
        if version==0 {
            db.collection::<HelpAssignment>(ASSIGNMENTS).insert_one(&row).session(&mut *session).await.map_err(|e| if matches!(e.kind.as_ref(),mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(w)) if w.code==11000){conflict()}else{e.into()})?;
        }else if db.collection::<HelpAssignment>(ASSIGNMENTS).replace_one(doc!{"_id":&profile,"version":version},&row).session(&mut *session).await?.matched_count!=1{return Err(conflict());}
        Ok(row)
    }.await)).await.map_err(map_transaction_error)
}

pub(crate) async fn for_turn(
    db: &Database,
    session: &mut ClientSession,
    profile: &str,
    turn: &str,
) -> AppResult<HelpRevision> {
    let turns =
        db.collection::<bson::Document>(crate::models::platform_public_turn::COLLECTION_NAME);
    let row = turns
        .find_one(doc! {"_id":turn})
        .session(&mut *session)
        .await?
        .ok_or_else(missing)?;
    if let Some(reference) = row.get("help_reference") {
        let reference =
            bson::from_bson::<HelpReference>(reference.clone()).map_err(|_| missing())?;
        return revision(db, &reference).await;
    }
    let help = published(db, profile).await?;
    turns.update_one(doc!{"_id":turn},doc!{"$set":{"help_reference":{"library_id":&help.library_id,"revision":help.revision}}}).session(session).await?;
    Ok(help)
}
pub(crate) async fn for_guide(
    db: &Database,
    profile: &str,
    receipt: &super::platform_native_receipt_service::ReceiptClaim,
) -> AppResult<HelpRevision> {
    let receipts =
        db.collection::<bson::Document>(super::platform_native_receipt_service::COLLECTION);
    let filter = doc! {
        "_id": &receipt.id, "profile_id": profile,
        "generation": receipt.generation, "claim_id": &receipt.claim_id,
        "phase": "processing", "$expr": {"$gt": ["$retry_at", "$$NOW"]},
    };
    let row = receipts
        .find_one(filter.clone())
        .await?
        .ok_or_else(conflict)?;
    if let Some(value) = row.get("help_reference") {
        let reference = bson::from_bson::<HelpReference>(value.clone()).map_err(|_| conflict())?;
        return revision(db, &reference).await;
    }
    let help = published(db, profile).await?;
    let reference = doc! {"library_id": &help.library_id, "revision": help.revision};
    // Retries retain the first revision even when an administrator publishes
    // while this receipt is being processed.
    let saved = receipts.find_one_and_update(filter, vec![doc! {
        "$set": {"help_reference": {"$ifNull": ["$help_reference", {"$literal": reference}]}}
    }]).return_document(ReturnDocument::After).await?.ok_or_else(conflict)?;
    let reference = bson::from_bson::<HelpReference>(
        saved.get("help_reference").cloned().ok_or_else(conflict)?,
    )
    .map_err(|_| conflict())?;
    revision(db, &reference).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn help_content_limits_and_unique_ids_are_enforced() {
        let mut c = seed_content().unwrap();
        validate(&c).unwrap();
        c.use_cases[0].id = c.articles[0].id.clone();
        assert!(validate(&c).is_err());
        let mut c = seed_content().unwrap();
        c.articles[0].body = "🦉".repeat(6000);
        assert!(validate(&c).is_err());
    }
    #[tokio::test]
    async fn publication_is_atomic_versioned_shared_and_reversible() {
        let db = crate::test_utils::connect_transaction_test_database("help_publication").await;
        let (first_seed, second_seed) = tokio::join!(ensure(&db), ensure(&db));
        first_seed.unwrap();
        second_seed.unwrap();
        assert_eq!(history(&db, DEFAULT_LIBRARY, None).await.unwrap().len(), 1);
        let actor = uuid::Uuid::new_v4().to_string();
        let mut user = crate::test_utils::test_user(&actor, crate::models::user::UserType::Person);
        super::super::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        user.role_ids = vec![
            super::super::role_service::get_platform_role_ids(&db)
                .await
                .unwrap()
                .admin,
        ];
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(user)
            .await
            .unwrap();
        let first = get(&db, DEFAULT_LIBRARY).await.unwrap();
        let old = published(&db, "profile-a").await.unwrap();
        let outsider = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &outsider,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        assert!(create(&db, "Unauthorized", &outsider).await.is_err());
        assert!(
            save(&db, DEFAULT_LIBRARY, first.version, &first.draft, &outsider)
                .await
                .is_err()
        );
        assert!(
            publish(&db, DEFAULT_LIBRARY, first.version, &outsider, None)
                .await
                .is_err()
        );
        let other = create(&db, "Restaurant help", &actor).await.unwrap();
        db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .insert_one(doc! {"_id":"profile-c", "generation":1_i64})
            .await
            .unwrap();
        assert!(
            assign(&db, "profile-c", 1, 0, &other.id, &outsider)
                .await
                .is_err()
        );
        assert!(
            assign(&db, "profile-c", 2, 0, &other.id, &actor)
                .await
                .is_err()
        );
        let assigned = assign(&db, "profile-c", 1, 0, &other.id, &actor)
            .await
            .unwrap();
        assert_eq!(assigned.version, 1);
        assert!(
            assign(&db, "profile-c", 1, 0, DEFAULT_LIBRARY, &actor)
                .await
                .is_err()
        );
        let receipt = super::super::platform_native_receipt_service::ReceiptClaim {
            id: uuid::Uuid::new_v4().to_string(),
            generation: 1,
            claim_id: uuid::Uuid::new_v4().to_string(),
        };
        db.collection::<bson::Document>(super::super::platform_native_receipt_service::COLLECTION)
            .insert_one(
                doc! {"_id":&receipt.id, "profile_id":"profile-a", "generation":1_i64,
                "claim_id":&receipt.claim_id, "phase":"processing",
                "retry_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(5))},
            )
            .await
            .unwrap();
        assert_eq!(
            for_guide(&db, "profile-a", &receipt)
                .await
                .unwrap()
                .revision,
            1
        );
        assert!(for_guide(&db, "profile-b", &receipt).await.is_err());
        let turn = uuid::Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::platform_public_turn::COLLECTION_NAME)
            .insert_one(doc! {"_id":&turn})
            .await
            .unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert_eq!(
            for_turn(&db, &mut session, "profile-a", &turn)
                .await
                .unwrap()
                .revision,
            1
        );
        session.commit_transaction().await.unwrap();
        let mut draft = first.draft.clone();
        draft.articles[0].body = "Reviewed new public fact".into();
        let saved = save(&db, DEFAULT_LIBRARY, first.version, &draft, &actor)
            .await
            .unwrap();
        assert!(
            save(&db, DEFAULT_LIBRARY, first.version, &draft, &actor)
                .await
                .is_err()
        );
        assert_eq!(
            published(&db, "profile-b").await.unwrap().content.articles[0].body,
            old.content.articles[0].body
        );
        let (a, b) = tokio::join!(
            publish(&db, DEFAULT_LIBRARY, saved.version, &actor, None),
            publish(&db, DEFAULT_LIBRARY, saved.version, &actor, None)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let current = get(&db, DEFAULT_LIBRARY).await.unwrap();
        assert_eq!(published(&db, "profile-a").await.unwrap().revision, 2);
        assert_eq!(
            published(&db, "profile-b").await.unwrap().content.articles[0].body,
            "Reviewed new public fact"
        );
        assert_eq!(
            published(&db, "profile-c").await.unwrap().library_id,
            other.id
        );
        assert_eq!(published(&db, "profile-c").await.unwrap().revision, 1);
        assert_eq!(
            for_guide(&db, "profile-a", &receipt)
                .await
                .unwrap()
                .revision,
            1
        );
        session.start_transaction().await.unwrap();
        assert_eq!(
            for_turn(&db, &mut session, "profile-a", &turn)
                .await
                .unwrap()
                .revision,
            1
        );
        session.commit_transaction().await.unwrap();
        db.collection::<bson::Document>(super::super::platform_native_receipt_service::COLLECTION)
            .update_one(doc! {"_id":&receipt.id}, doc! {"$inc":{"generation":1_i64}})
            .await
            .unwrap();
        assert!(for_guide(&db, "profile-a", &receipt).await.is_err());
        let restored = publish(&db, DEFAULT_LIBRARY, current.version, &actor, Some(1))
            .await
            .unwrap();
        assert_eq!(restored.published_revision, 3);
        assert_eq!(
            published(&db, "profile-a").await.unwrap().content.articles[0].body,
            old.content.articles[0].body
        );
        assert_eq!(
            revision(
                &db,
                &HelpReference {
                    library_id: DEFAULT_LIBRARY.into(),
                    revision: 2
                }
            )
            .await
            .unwrap()
            .content
            .articles[0]
                .body,
            "Reviewed new public fact"
        );
        assert_eq!(history(&db, DEFAULT_LIBRARY, None).await.unwrap().len(), 3);
        db.drop().await.unwrap();
    }
}
