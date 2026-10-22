//! Bounded per-owner scope planning. Preparation never closes account authority
//! and observes only that owner's credential dependency revision.
use super::*;
use crate::crypto::aes::EncryptionKeys;
use futures::TryStreamExt;

pub(super) const PREPARATIONS: &str = "personal_erasure_preparations";
const VERSION: i32 = 7;
const COLLECTIONS: [&str; 2] = [
    crate::models::user_api_key::COLLECTION_NAME,
    crate::models::user_provider_token::COLLECTION_NAME,
];

fn secret_rows(owner: &str) -> Document {
    doc! {"user_id":owner,"personal_erasure_preserved":{"$ne":true},
    "$or":[{"access_token_encrypted":{"$ne":bson::Bson::Null}},
    {"refresh_token_encrypted":{"$ne":bson::Bson::Null}}]}
}
fn accounts(user: &str) -> Document {
    doc! {"$or":[{"owner_user_id":user},{"owner_user_id":bson::Bson::Null,"created_by":user}]}
}
pub(super) async fn ensure_indexes(db: &Database) -> AppResult<()> {
    for collection in COLLECTIONS {
        for keys in [
            doc! {"user_id":1,"_id":1},
            doc! {"user_id":1,"provider_config_id":1,"created_at":1},
        ] {
            db.collection::<Document>(collection)
                .create_index(mongodb::IndexModel::builder().keys(keys).build())
                .await?;
        }
    }
    db.collection::<Document>(PREPARATIONS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id":1,"generation":1,"subject":1,"group":1,"non_legacy":1})
                .build(),
        )
        .await?;
    Ok(())
}
fn plan_id(generation: &str, collection: &str, source_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("erasure-plan:{generation}:{collection}:{source_id}").as_bytes(),
    )
    .to_string()
}
pub(super) async fn source_plan(
    db: &Database,
    user: &str,
    collection: &str,
    source_id: &str,
) -> AppResult<Option<Document>> {
    let Some(preparation) = db
        .collection::<Document>(PREPARATIONS)
        .find_one(doc! {"_id":user,"status":"ready","version":VERSION})
        .await?
    else {
        return Ok(None);
    };
    let generation = preparation
        .get_str("generation")
        .map_err(|_| AppError::Internal("Invalid preparation generation".into()))?;
    Ok(db.collection::<Document>(PREPARATIONS).find_one(doc! {
        "_id":plan_id(generation, collection, source_id),"source_revision":preparation.get("source_revision").cloned()
    }).await?)
}
async fn has_credentials(db: &Database, user: &str, tx: &mut ClientSession) -> AppResult<bool> {
    for collection in COLLECTIONS {
        if db
            .collection::<Document>(collection)
            .find_one(secret_rows(user))
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        let mut cursor = db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
            .aggregate([doc! {"$match":accounts(user)},
                doc! {"$lookup":{"from":collection,"localField":"_id","foreignField":"user_id",
                    "pipeline":[{"$match":{"$or":[{"access_token_encrypted":{"$ne":bson::Bson::Null}},
                        {"refresh_token_encrypted":{"$ne":bson::Bson::Null}}]}},{"$limit":1}],"as":"credentials"}},
                doc! {"$match":{"credentials.0":{"$exists":true}}},doc! {"$limit":1}])
            .session(&mut *tx).await?;
        if cursor.stream(&mut *tx).try_next().await?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(super) async fn require_ready_in_session(
    db: &Database,
    user: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let owner = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id":user})
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::Conflict("Credential owner changed".into()))?;
    let revision = owner
        .get("credential_dependency_revision")
        .cloned()
        .unwrap_or(bson::Bson::Int64(0));
    if db
        .collection::<Document>(PREPARATIONS)
        .find_one(doc! {"_id":user,"status":"ready","source_revision":revision,"version":VERSION})
        .session(&mut *tx)
        .await?
        .is_some()
        || !has_credentials(db, user, tx).await?
    {
        return Ok(());
    }
    Err(AppError::ErasurePreparing)
}
pub(crate) async fn ready_or_enqueue(db: &Database, user: &str) -> AppResult<bool> {
    let db = super::super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<bool> = async {
        // Preparation is not write admission: disabled owners, including those
        // already retired by the platform-personal lifecycle, still need
        // erasure. Touch the same owner row so credential changes and lifecycle
        // transitions still contend; acceptance rechecks the dependency revision
        // and installs the permanent write barrier in its own transaction.
        let owner = db.collection::<Document>(USERS).find_one_and_update(
            doc! {"_id":user,"user_type":{"$ne":"org"}},
            doc! {"$inc":{"credential_preparation_fence":1_i64}}).session(&mut tx).await?
            .ok_or_else(||AppError::Conflict("Credential owner changed".into()))?;
        let revision=owner.get("credential_dependency_revision").cloned().unwrap_or(bson::Bson::Int64(0));
        if let Some(job)=db.collection::<Document>(PREPARATIONS).find_one(doc! {"_id":user,"source_revision":&revision,"version":VERSION}).session(&mut tx).await? {
            match job.get_str("status").ok() {Some("ready")=>return Ok(true),Some("preparing")=>return Ok(false),_=>{}}
        }
        if !has_credentials(&db,user,&mut tx).await? {return Ok(true);}
        db.collection::<Document>(PREPARATIONS).replace_one(doc! {"_id":user},doc! {
            "_id":user,"version":VERSION,"status":"preparing","source_revision":revision,
            "generation":uuid::Uuid::new_v4().to_string(),"phase":"cleanup","subject":user,"account_cursor":bson::Bson::Null,
            "collection_index":0_i32,"cursor":bson::Bson::Null,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).upsert(true).session(&mut tx).await?;
        Ok(false)
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}
fn progress_filter(job: &Document) -> AppResult<Document> {
    Ok(
        doc! {"_id":job.get_str("_id").map_err(|_|AppError::Internal("Invalid preparation identity".into()))?,
        "status":"preparing","generation":job.get("generation").cloned(),"lease_token":job.get("lease_token").cloned(),
        "$expr":{"$gt":["$lease_expires_at","$$NOW"]}},
    )
}
async fn advance(db: &Database, job: &Document, set: Document) -> AppResult<()> {
    if db.collection::<Document>(PREPARATIONS).update_one(progress_filter(job)?,
        doc! {"$set":set,"$unset":{"lease_token":"","lease_expires_at":""},"$currentDate":{"updated_at":true}}).await?.matched_count!=1 {
        return Err(AppError::Conflict("Preparation lease/generation changed".into()));
    }
    Ok(())
}
async fn advance_page(
    db: &Database,
    job: &Document,
    plans: &[Document],
    obsolete: &[Document],
    set: &Document,
) -> AppResult<()> {
    // A page has at most 32 metadata rows. Commit changes with the leased cursor:
    // one majority acknowledgement per page, and no stale generation can
    // recreate old children or delete a successor's planning rows.
    super::super::transaction_service::run_db_only(db, &(db, job, plans, obsolete, set), |tx, context| {
        let &(db, job, plans, obsolete, set) = context;
        Box::pin(async move {
            if db.collection::<Document>(PREPARATIONS).update_one(progress_filter(job)?,
                doc! {"$set":set.clone(),"$unset":{"lease_token":"","lease_expires_at":""},"$currentDate":{"updated_at":true}})
                .session(&mut *tx).await?.matched_count != 1 {
                return Err(AppError::Conflict("Preparation lease/generation changed".into()));
            }
            if !obsolete.is_empty() {
                db.collection::<Document>(PREPARATIONS).delete_many(doc! {
                    "user_id":job.get("_id").cloned(),"kind":"scope_source",
                    "generation":{"$ne":job.get("generation").cloned()},
                    "_id":{"$in":obsolete.iter().filter_map(|row|row.get("_id")).cloned().collect::<Vec<_>>()}
                }).session(&mut *tx).await?;
            }
            for plan in plans {
                db.collection::<Document>(PREPARATIONS)
                    .replace_one(doc! {"_id":plan.get("_id").cloned()}, plan.clone())
                    .upsert(true).session(&mut *tx).await?;
            }
            Ok(())
        })
    }).await
}
pub(super) async fn process_one(db: &Database, keys: &EncryptionKeys) -> AppResult<bool> {
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    let Some(job)=db.collection::<Document>(PREPARATIONS).find_one_and_update(
        doc! {"status":"preparing","version":VERSION,"$expr":{"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}},
        vec![doc! {"$set":{"lease_token":uuid::Uuid::new_v4().to_string(),"lease_expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":120}},"updated_at":"$$NOW"}}])
        .sort(doc! {"updated_at":1,"_id":1}).return_document(mongodb::options::ReturnDocument::After).await? else {return Ok(false);};
    let user = job
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid preparation identity".into()))?;
    if job.get_str("phase") == Ok("cleanup") {
        let old: Vec<Document> = db.collection::<Document>(PREPARATIONS)
            .find(doc! {"user_id":user,"kind":"scope_source","generation":{"$ne":job.get("generation").cloned()}})
            .projection(doc! {"_id":1}).limit(32).await?.try_collect().await?;
        let set = if old.is_empty() {
            doc! {"phase":"scan"}
        } else {
            doc! {}
        };
        advance_page(db, &job, &[], &old, &set).await?;
        return Ok(true);
    }
    let subject = job
        .get_str("subject")
        .map_err(|_| AppError::Internal("Invalid preparation subject".into()))?;
    let index = job.get_i32("collection_index").unwrap_or(0) as usize;
    if let Some(collection) = COLLECTIONS.get(index) {
        let mut filter = secret_rows(subject);
        if let Ok(cursor) = job.get_str("cursor") {
            filter.insert("_id", doc! {"$gt":cursor});
        }
        let rows: Vec<Document> = db
            .collection::<Document>(collection)
            .find(filter)
            .sort(doc! {"_id":1})
            .limit(32)
            .await?
            .try_collect()
            .await?;
        let mut plans = Vec::with_capacity(rows.len());
        for source in &rows {
            let mut marker = super::super::unified_key_service::erasure_credential_group(
                db, keys, collection, source,
            )
            .await?;
            let group = marker
                .get_str("group")
                .map_err(|_| AppError::Internal("Invalid credential group".into()))?
                .to_owned();
            if job.get_str("phase") == Ok("plan") {
                let non_legacy = db.collection::<Document>(PREPARATIONS).find_one(doc! {
                    "user_id":user,"generation":job.get("generation").cloned(),"subject":subject,"group":&group,"non_legacy":true
                }).await?.is_some();
                let grant = marker.get_bool("grant_wide_only") == Ok(true)
                    || (marker.get_bool("revokes_grant") == Ok(true) && non_legacy);
                marker.insert("scope", if grant { "grant" } else { "token" });
                if grant {
                    let representative = db.collection::<Document>(PREPARATIONS).find_one(doc! {
                        "user_id":user,"generation":job.get("generation").cloned(),"subject":subject,"group":&group,"kind":"scope_source"
                    }).sort(doc! {"has_refresh":-1,"source_updated_at":-1,"source_created_at":-1,"source_id":1,"source_collection":1}).await?
                        .ok_or_else(||AppError::Internal("Missing grant representative".into()))?;
                    marker.insert(
                        "representative_collection",
                        representative.get("source_collection").cloned(),
                    );
                    marker.insert(
                        "representative_id",
                        representative.get("source_id").cloned(),
                    );
                }
            }
            let generation = job
                .get_str("generation")
                .map_err(|_| AppError::Internal("Invalid preparation generation".into()))?;
            let id = source
                .get_str("_id")
                .map_err(|_| AppError::Internal("Invalid credential identity".into()))?;
            marker.extend(doc! {"_id":plan_id(generation, collection, id),"kind":"scope_source","user_id":user,
                "generation":generation,"source_revision":job.get("source_revision").cloned(),"subject":subject,
                "source_collection":*collection,"source_id":id,
                "has_refresh":source.get_binary_generic("refresh_token_encrypted").is_ok(),
                "source_updated_at":source.get("updated_at").or_else(||source.get("created_at")).cloned().unwrap_or(bson::Bson::Null),
                "source_created_at":source.get("created_at").cloned().unwrap_or(bson::Bson::Null)});
            plans.push(marker);
        }
        let set = if let Some(last) = rows.last() {
            doc! {"cursor":last.get("_id").cloned()}
        } else {
            doc! {"collection_index":(index+1) as i32,"cursor":bson::Bson::Null}
        };
        advance_page(db, &job, &plans, &[], &set).await?;
        return Ok(true);
    }
    let mut account_filter = accounts(user);
    if let Ok(cursor) = job.get_str("account_cursor") {
        account_filter.insert("_id", doc! {"$gt":cursor});
    }
    if let Some(account) = db
        .collection::<Document>(crate::models::service_account::COLLECTION_NAME)
        .find_one(account_filter)
        .sort(doc! {"_id":1})
        .await?
    {
        let id = account
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid credential subject".into()))?;
        advance(db,&job,doc! {"subject":id,"account_cursor":id,"collection_index":0_i32,"cursor":bson::Bson::Null}).await?;
        return Ok(true);
    }
    if job.get_str("phase") == Ok("scan") {
        advance(db,&job,doc! {"phase":"plan","subject":user,"account_cursor":bson::Bson::Null,"collection_index":0_i32,"cursor":bson::Bson::Null}).await?;
        return Ok(true);
    }
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        let revision = job
            .get("source_revision")
            .cloned()
            .unwrap_or(bson::Bson::Int64(0));
        let owner = db
            .collection::<Document>(USERS)
            .find_one_and_update(
                doc! {"_id":user},
                doc! {"$inc":{"credential_preparation_fence":1_i64}},
            )
            .session(&mut tx)
            .await?;
        let current = owner
            .as_ref()
            .and_then(|row| row.get("credential_dependency_revision"))
            .cloned()
            .unwrap_or(bson::Bson::Int64(0));
        let state = if owner.is_some() && current == revision {
            "ready"
        } else {
            "stale"
        };
        if db
            .collection::<Document>(PREPARATIONS)
            .update_one(
                progress_filter(&job)?,
                doc! {"$set":{"status":state},"$unset":{"lease_token":"","lease_expires_at":""}},
            )
            .session(&mut tx)
            .await?
            .matched_count
            != 1
        {
            return Err(AppError::Conflict(
                "Preparation lease/generation changed".into(),
            ));
        }
        Ok(())
    }
    .await;
    super::super::org_membership_authority::finish(tx, result).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inactive_owners_prepare_without_reopening_authority_and_rewrap_invalidates_snapshot() {
        for deleting in [false, true] {
            let (db, user) = super::super::tests::fixture().await;
            let keys = crate::test_utils::test_encryption_keys();
            let mut legacy = crate::test_utils::test_encryption_keys();
            legacy.set_contextual_record_writes(false);
            let credential = import_material(&db, &legacy, &user, Some("provider"), "oauth2").await;
            db.collection::<Document>(USERS)
                .update_one(
                    doc! {"_id":&user},
                    doc! {"$set":{"is_active":false,"platform_personal_deleting":deleting},
                    "$inc":{"platform_personal_effect_revision":1_i64}},
                )
                .await
                .unwrap();
            let original = db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&user})
                .await
                .unwrap()
                .unwrap();
            finish_preparation(&db, &keys, &user).await;
            super::super::tests::assert_account_preserved(
                db.collection::<Document>(USERS)
                    .find_one(doc! {"_id":&user})
                    .await
                    .unwrap()
                    .unwrap(),
                original,
            );

            // A real storage writer still invalidates an inactive owner's
            // prepared revision, even though the credential value is unchanged.
            let source = db
                .collection::<Document>(COLLECTIONS[0])
                .find_one(doc! {"_id":&credential.id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                crate::services::credential_record_service::migrate_snapshot(
                    &db,
                    &keys,
                    COLLECTIONS[0],
                    &source,
                )
                .await
                .unwrap()
            );
            assert!(matches!(
                Box::pin(super::super::accept(&db, &user, &user, None)).await,
                Err(AppError::ErasurePreparing)
            ));
            assert!(super::super::status(&db, &user).await.unwrap().is_none());
            finish_preparation(&db, &keys, &user).await;
            let accepted = Box::pin(super::super::accept(&db, &user, &user, None))
                .await
                .unwrap();
            assert_eq!(accepted.status, ErasureStatus::Accepted);
            assert!(matches!(
                crate::services::erasure_admission::begin_write(&db, &user).await,
                Err(AppError::Conflict(_))
            ));
            assert!(matches!(
                crate::services::admin_user_service::set_user_active(&db, "admin", &user, true)
                    .await,
                Err(AppError::Conflict(_))
            ));
            assert!(
                crate::services::user_credentials_service::upsert_user_credentials(
                    &db,
                    &keys,
                    &user,
                    "provider",
                    "late-client",
                    Some("late-secret"),
                    None,
                )
                .await
                .is_err()
            );
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn stale_cleanup_cannot_delete_successor_plans_or_recreate_old_plans() {
        let (db, owner) = super::super::tests::fixture().await;
        let old = doc! {"_id":&owner,"status":"preparing","version":VERSION,
        "generation":"old","phase":"cleanup","lease_token":"old-lease",
        "lease_expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(2))};
        let preparations = db.collection::<Document>(PREPARATIONS);
        preparations.insert_one(&old).await.unwrap();
        preparations
            .update_one(
                doc! {"_id":&owner},
                doc! {"$set":{
                "generation":"successor","lease_token":"successor-lease"}},
            )
            .await
            .unwrap();
        let successor = preparations
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .unwrap();
        let current_plan = doc! {"_id":plan_id("successor", COLLECTIONS[0], "source"),
        "user_id":&owner,"kind":"scope_source","generation":"successor"};
        let old_plan = doc! {"_id":plan_id("old", COLLECTIONS[0], "source"),
        "user_id":&owner,"kind":"scope_source","generation":"old"};
        preparations.insert_one(&current_plan).await.unwrap();

        // The stale scanner can observe successor rows as obsolete. Its lease
        // must fence both removal and any page writes in the same transaction.
        assert!(matches!(
            advance_page(
                &db,
                &old,
                std::slice::from_ref(&old_plan),
                std::slice::from_ref(&current_plan),
                &doc! {"phase":"scan"}
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            preparations.find_one(doc! {"_id":&owner}).await.unwrap(),
            Some(successor.clone())
        );
        assert_eq!(
            preparations
                .find_one(doc! {"_id":current_plan.get("_id").cloned()})
                .await
                .unwrap(),
            Some(current_plan.clone())
        );
        assert!(
            preparations
                .find_one(doc! {"_id":old_plan.get("_id").cloned()})
                .await
                .unwrap()
                .is_none()
        );

        preparations.insert_one(&old_plan).await.unwrap();
        advance_page(
            &db,
            &successor,
            &[],
            std::slice::from_ref(&old_plan),
            &doc! {"phase":"scan"},
        )
        .await
        .unwrap();
        assert!(
            preparations
                .find_one(doc! {"_id":old_plan.get("_id").cloned()})
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            preparations
                .find_one(doc! {"_id":current_plan.get("_id").cloned()})
                .await
                .unwrap(),
            Some(current_plan)
        );
        let advanced = preparations
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(advanced.get_str("phase"), Ok("scan"));
        assert!(!advanced.contains_key("lease_token"));
        db.drop().await.unwrap();
    }

    fn binary(bytes: Vec<u8>) -> bson::Bson {
        bson::Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes,
        })
    }

    async fn replace(db: &Database, owner: &str, keys: &EncryptionKeys) -> AppResult<()> {
        let encrypted = keys.encrypt(b"successor-token").await?;
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .write_concern(WriteConcern::majority())
            .await?;
        super::super::super::erasure_admission::credential_change_in_session(db, owner, &mut tx)
            .await?;
        db.collection::<Document>(COLLECTIONS[1])
            .update_one(
                doc! {"_id":"token-0000"},
                doc! {"$set":{"access_token_encrypted":binary(encrypted)}},
            )
            .session(&mut tx)
            .await?;
        super::super::super::transaction_service::commit(&mut tx).await
    }
    async fn finish_preparation(db: &Database, keys: &EncryptionKeys, user: &str) {
        for _ in 0..48 {
            if ready_or_enqueue(db, user).await.unwrap() {
                return;
            }
            assert!(process_one(db, keys).await.unwrap());
        }
        panic!("bounded preparation did not finish");
    }
    #[derive(Debug)]
    struct HeldImport {
        inner: crate::crypto::local_key_provider::LocalKeyProvider,
        first: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl crate::crypto::key_provider::KeyProvider for HeldImport {
        async fn wrap_dek(&self, key: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
            if !self.first.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.wrap_dek(key).await
        }
        async fn unwrap_dek(
            &self,
            key: &crate::crypto::key_provider::WrappedKey,
        ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
            self.inner.unwrap_dek(key).await
        }
        fn current_key_id(&self) -> u8 {
            self.inner.current_key_id()
        }
        fn has_key_id(&self, id: u8) -> bool {
            self.inner.has_key_id(id)
        }
        fn has_previous_key(&self) -> bool {
            false
        }
    }
    async fn import_material(
        db: &Database,
        keys: &EncryptionKeys,
        owner: &str,
        provider: Option<&str>,
        credential_type: &str,
    ) -> crate::models::user_api_key::UserApiKey {
        crate::services::user_api_key_service::create_api_key(
            db,
            keys,
            owner,
            crate::services::user_api_key_service::CreateApiKeyParams {
                label: "Known opaque copy",
                credential_type,
                credential: "same-opaque-upstream-token",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: provider,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn indexed_preparation_progresses_under_continuous_unrelated_credential_writes() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        import_material(&db, &keys, &user, Some("personal-provider"), "oauth2").await;
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        let initial = db
            .collection::<Document>(PREPARATIONS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        let mut ready = false;
        for iteration in 0..32 {
            crate::services::user_api_key_service::create_api_key(
                &db,
                &keys,
                &uuid::Uuid::new_v4().to_string(),
                crate::services::user_api_key_service::CreateApiKeyParams {
                    label: "Unrelated live writer",
                    credential_type: "bearer",
                    credential: &format!("unrelated-token-{iteration}"),
                    access_token: None,
                    refresh_token: None,
                    token_scopes: None,
                    expires_at: None,
                    provider_config_id: None,
                    connection_id: None,
                    oauth_client_id: None,
                    oauth_client_secret: None,
                    status: "active",
                    source: None,
                    source_id: None,
                },
            )
            .await
            .unwrap();
            assert!(process_one(&db, &keys).await.unwrap());
            if ready_or_enqueue(&db, &user).await.unwrap() {
                ready = true;
                break;
            }
        }
        assert!(ready, "unrelated writers starved finite preparation");
        let current = db
            .collection::<Document>(PREPARATIONS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.get("generation"), initial.get("generation"));
        assert_eq!(
            current.get("source_revision"),
            initial.get("source_revision")
        );
        // Other owners' imports do not change this owner's preparation.
        let copied = import_material(
            &db,
            &keys,
            &uuid::Uuid::new_v4().to_string(),
            None,
            "bearer",
        )
        .await;
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        assert!(
            db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
                .find_one(doc! {"_id":copied.id})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn cross_provider_and_unattributed_manual_copies_do_not_block_acceptance() {
        for provider in [Some("different-provider"), None] {
            let (db, user) = super::super::tests::fixture().await;
            let keys = crate::test_utils::test_encryption_keys();
            let org = uuid::Uuid::new_v4().to_string();
            db.collection::<crate::models::user::User>(USERS)
                .insert_one(crate::test_utils::test_user(
                    &org,
                    crate::models::user::UserType::Org,
                ))
                .await
                .unwrap();
            import_material(&db, &keys, &user, Some("personal-provider"), "oauth2").await;
            let copy = import_material(&db, &keys, &org, provider, "oauth2").await;
            assert!(copy.source_id.is_none());
            finish_preparation(&db, &keys, &user).await;
            let job = super::super::accept(&db, &user, &user, None).await.unwrap();
            assert_eq!(job.status, ErasureStatus::Accepted);
            assert!(
                db.collection::<Document>(USERS)
                    .find_one(doc! {"_id":&user,"is_active":false})
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                db.collection::<Document>(COLLECTIONS[0])
                    .find_one(doc! {"_id":&copy.id,"user_id":&org})
                    .await
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                db.collection::<Document>(super::super::provider::EFFECTS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn held_other_owner_import_preserves_prepared_revision_and_acceptance() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let org = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &org,
                crate::models::user::UserType::Org,
            ))
            .await
            .unwrap();
        import_material(&db, &keys, &user, Some("personal-provider"), "oauth2").await;
        let held = std::sync::Arc::new(HeldImport {
            inner: crate::crypto::local_key_provider::LocalKeyProvider::new([0x11; 32], None),
            first: Default::default(),
            entered: Default::default(),
            release: Default::default(),
        });
        let held_keys = EncryptionKeys::with_provider(held.clone());
        let import = import_material(&db, &held_keys, &org, None, "bearer");
        let prepare = async {
            held.entered.notified().await;
            finish_preparation(&db, &keys, &user).await;
            let observed = db
                .collection::<Document>(PREPARATIONS)
                .find_one(doc! {"_id":&user})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(observed.get_str("status"), Ok("ready"));
            held.release.notify_one();
            observed
        };
        let (copy, observed) = tokio::join!(import, prepare);
        assert!(copy.source_id.is_none());
        let unchanged_person = db
            .collection::<Document>(USERS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            unchanged_person.get("credential_dependency_revision"),
            observed.get("source_revision")
        );
        let job = super::super::accept(&db, &user, &user, None).await.unwrap();
        assert_eq!(job.status, ErasureStatus::Accepted);
        let preparation = db
            .collection::<Document>(PREPARATIONS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(preparation.get("generation"), observed.get("generation"));
        assert!(
            db.collection::<Document>(COLLECTIONS[0])
                .find_one(doc! {"_id":&copy.id,"user_id":&org})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn providerless_oauth_writer_stays_usable_then_retains_an_external_erasure_obligation() {
        use crate::services::user_api_key_service::{self, CreateApiKeyParams};
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let key = user_api_key_service::create_api_key(
            &db,
            &keys,
            &user,
            CreateApiKeyParams {
                label: "Custom OAuth",
                credential_type: "oauth2",
                credential: "imported-access",
                access_token: None,
                refresh_token: Some("imported-refresh"),
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        assert!(key.provider_config_id.is_none());
        assert_eq!(
            crate::services::credential_record_service::CredentialRecord::key(&key)
                .text(
                    &keys,
                    "access_token_encrypted",
                    key.access_token_encrypted.as_deref().unwrap()
                )
                .await
                .unwrap()
                .as_str(),
            "imported-access"
        );
        user_api_key_service::update_api_key(
            &db,
            &keys,
            &user,
            &key.id,
            None,
            Some("replaced-custom-access"),
        )
        .await
        .unwrap();
        let key = db
            .collection::<crate::models::user_api_key::UserApiKey>(
                crate::models::user_api_key::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&key.id,"user_id":&user})
            .await
            .unwrap()
            .unwrap();
        finish_preparation(&db, &keys, &user).await;
        let job = super::super::accept(&db, &user, &user, None).await.unwrap();
        let mut actionable = false;
        for _ in 0..6 {
            if matches!(
                super::super::provider::pass(&db, &keys, &job)
                    .await
                    .unwrap(),
                super::super::provider::Pass::Action(_)
            ) {
                actionable = true;
                break;
            }
        }
        assert!(actionable);
        let effect = db
            .collection::<Document>(super::super::provider::EFFECTS)
            .find_one(doc! {"user_id":&user,"source_id":&key.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(effect.get_str("status"), Ok("operator_action"));
        assert_eq!(
            effect.get_str("reason"),
            Ok("oauth_provider_unknown_requires_external_reconciliation")
        );
        assert!(effect.contains_key("capability_encrypted"));
        let source = db
            .collection::<Document>(COLLECTIONS[0])
            .find_one(doc! {"_id":&key.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!source.contains_key("access_token_encrypted"));
        assert!(!source.contains_key("refresh_token_encrypted"));
        // Unknown attribution cannot be attested as an unsupported provider.
        let request = super::super::provider::EffectReconciliationRequest {
            effect_id: effect.get_str("_id").unwrap().to_owned(),
            action: "record_remote_revocation_unavailable".into(),
            evidence_reference: "external-ticket".into(),
            expected_retry_generation: 0,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        };
        assert!(matches!(
            super::super::provider::reconcile_effect(&db, &keys, &user, &user, &request).await,
            Err(AppError::Conflict(_))
        ));
        let confirmed = super::super::provider::EffectReconciliationRequest {
            action: "record_external_revocation_confirmed".into(),
            ..request
        };
        super::super::provider::reconcile_effect(&db, &keys, &user, &user, &confirmed)
            .await
            .unwrap();
        let completed = db
            .collection::<Document>(super::super::provider::EFFECTS)
            .find_one(doc! {"_id":&confirmed.effect_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.get_str("status"), Ok("complete"));
        assert_eq!(completed.get_bool("remote_revocation_confirmed"), Ok(true));
        assert!(!completed.contains_key("capability_encrypted"));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn actual_byo_replacement_rewrap_and_deletion_invalidate_prepared_dependency_revision() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let mut legacy = crate::test_utils::test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        db.collection::<Document>(COLLECTIONS[1]).insert_one(doc! {"_id":"token","user_id":&user,"provider_config_id":"provider",
            "credential_user_id":&user,"status":"active","access_token_encrypted":binary(keys.encrypt(b"existing-token").await.unwrap())}).await.unwrap();
        let credentials = crate::services::user_credentials_service::upsert_user_credentials(
            &db,
            &legacy,
            &user,
            "provider",
            "original-client",
            Some("original-secret"),
            None,
        )
        .await
        .unwrap();
        finish_preparation(&db, &keys, &user).await;
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        crate::services::user_credentials_service::upsert_user_credentials_with_expected_state_version(&db, &legacy, &user, "provider", "replacement-client", Some("replacement-secret"), None, Some(credentials.state_version)).await.unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        finish_preparation(&db, &keys, &user).await;
        // Storage-only rewrap must invalidate the same prepared dependency
        // authority as live BYO replacement, even with unchanged plaintext.
        let source = db
            .collection::<Document>(crate::models::user_provider_credentials::COLLECTION_NAME)
            .find_one(doc! {"_id":&credentials.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            crate::services::credential_record_service::migrate_snapshot(
                &db,
                &keys,
                crate::models::user_provider_credentials::COLLECTION_NAME,
                &source,
            )
            .await
            .unwrap()
        );
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        // The actual token is migrated through its live field adapter too;
        // strict preparation must use that adapter instead of raw decrypt.
        let token = db
            .collection::<Document>(COLLECTIONS[1])
            .find_one(doc! {"_id":"token"})
            .await
            .unwrap()
            .unwrap();
        assert!(
            crate::services::credential_record_service::migrate_snapshot(
                &db,
                &keys,
                crate::models::user_provider_token::COLLECTION_NAME,
                &token,
            )
            .await
            .unwrap()
        );
        let mut strict = crate::test_utils::test_encryption_keys();
        strict.set_allow_legacy_record_reads(false);
        finish_preparation(&db, &strict, &user).await;
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        assert_eq!(
            source_plan(&db, &user, COLLECTIONS[1], "token")
                .await
                .unwrap()
                .unwrap()
                .get_str("scope"),
            Ok("token")
        );
        crate::services::user_credentials_service::delete_user_credentials(&db, &user, "provider")
            .await
            .unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        assert!(
            db.collection::<Document>(USERS)
                .find_one(doc! {"_id":&user,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn preparation_pages_more_than_256_rows_and_refresh_invalidates_exact_source() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let tokens = db.collection::<Document>(COLLECTIONS[1]);
        let mut rows = Vec::new();
        for index in 0..270 {
            rows.push(doc! {"_id":format!("token-{index:04}"),"user_id":&user,"provider_config_id":"provider",
                "status":"active","access_token_encrypted":binary(keys.encrypt(format!("token-{index}").as_bytes()).await.unwrap())});
        }
        tokens.insert_many(rows).await.unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        for _ in 0..40 {
            if ready_or_enqueue(&db, &user).await.unwrap() {
                break;
            }
            assert!(process_one(&db, &keys).await.unwrap());
        }
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        assert_eq!(
            db.collection::<Document>(PREPARATIONS)
                .count_documents(doc! {"user_id":&user,"scope":"token"})
                .await
                .unwrap(),
            270
        );
        let original = source_plan(&db, &user, COLLECTIONS[1], "token-0000")
            .await
            .unwrap()
            .unwrap();
        replace(&db, &user, &keys).await.unwrap();
        assert!(
            !tokens
                .find_one(doc! {"_id":"token-0000"})
                .await
                .unwrap()
                .unwrap()
                .contains_key("erasure_scope_preparation")
        );
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        assert!(
            source_plan(&db, &user, COLLECTIONS[1], "token-0000")
                .await
                .unwrap()
                .is_none()
        );
        finish_preparation(&db, &keys, &user).await;
        let refreshed = source_plan(&db, &user, COLLECTIONS[1], "token-0000")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(original.get("generation"), refreshed.get("generation"));
        assert_eq!(
            db.collection::<Document>(PREPARATIONS)
                .count_documents(
                    doc! {"user_id":&user,"generation":original.get("generation").cloned()}
                )
                .await
                .unwrap(),
            0
        );
        let group = refreshed.get_str("group").unwrap().to_owned();
        replace(&db, &user, &keys).await.unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        finish_preparation(&db, &keys, &user).await;
        assert_eq!(
            source_plan(&db, &user, COLLECTIONS[1], "token-0000")
                .await
                .unwrap()
                .unwrap()
                .get_str("group"),
            Ok(group.as_str())
        );
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&user,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        assert!(super::super::status(&db, &user).await.unwrap().is_none());
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn own_provider_app_group_dispatches_one_grant_or_per_credential_tokens() {
        use axum::{Router, http::StatusCode, routing::post};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        for grant in [true, false] {
            let (db, user) = super::super::tests::fixture().await;
            let keys = crate::test_utils::test_encryption_keys();
            let count = Arc::new(AtomicUsize::new(0));
            let calls = count.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/revoke", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(
                    listener,
                    Router::new().route(
                        "/revoke",
                        post(move |body: String| {
                            if grant {
                                assert!(
                                    body.contains("own-refresh-2"),
                                    "grant must use the newest refresh member: {body}"
                                );
                            }
                            calls.fetch_add(1, Ordering::SeqCst);
                            async { StatusCode::OK }
                        }),
                    ),
                )
                .await
                .unwrap();
            });
            let provider = doc! {"_id":"group-provider","slug":"group-provider","name":"Group","provider_type":"oauth2","is_active":true,
            "created_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            "revocation":{"style":"rfc7009","url":url,"auth":"none","request_encoding":"form","revokes_grant":grant,"effect_scope":if grant {"grant_wide"}else{"token_only"}}};
            db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
                .insert_one(provider)
                .await
                .unwrap();
            for index in 0..3 {
                let mut row = doc! {"_id":format!("token-{index}"),"user_id":&user,"provider_config_id":"group-provider",
                "token_type":"oauth2","status":"active","created_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(3-index)),"updated_at":bson::DateTime::now()};
                if index == 0 {
                    row.insert(
                        "access_token_encrypted",
                        binary(keys.encrypt(b"stale-access").await.unwrap()),
                    );
                } else {
                    row.insert(
                        "refresh_token_encrypted",
                        binary(
                            keys.encrypt(format!("own-refresh-{index}").as_bytes())
                                .await
                                .unwrap(),
                        ),
                    );
                }
                db.collection::<Document>(COLLECTIONS[1])
                    .insert_one(row)
                    .await
                    .unwrap();
            }
            finish_preparation(&db, &keys, &user).await;
            let rows: Vec<Document> = db
                .collection::<Document>(COLLECTIONS[1])
                .find(doc! {"user_id":&user})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            let first = source_plan(&db, &user, COLLECTIONS[1], rows[0].get_str("_id").unwrap())
                .await
                .unwrap()
                .unwrap();
            let group = first.get_str("group").unwrap();
            for row in &rows {
                assert!(!row.contains_key("erasure_scope_preparation"));
                let plan = source_plan(&db, &user, COLLECTIONS[1], row.get_str("_id").unwrap())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(plan.get_str("group"), Ok(group));
                assert_eq!(
                    plan.get_str("scope"),
                    Ok(if grant { "grant" } else { "token" })
                );
            }
            let job = super::super::accept(&db, &user, &user, None).await.unwrap();
            let mut complete = false;
            for _ in 0..12 {
                if matches!(
                    Box::pin(super::super::provider::pass(&db, &keys, &job))
                        .await
                        .unwrap(),
                    super::super::provider::Pass::Complete
                ) {
                    complete = true;
                    break;
                }
            }
            assert!(complete);
            let expected = if grant { 1 } else { 3 };
            assert_eq!(count.load(Ordering::SeqCst), expected);
            assert_eq!(db.collection::<Document>(super::super::provider::EFFECTS).count_documents(doc! {"user_id":&user,"kind":"oauth_token_revocation","status":"complete","capability_encrypted":{"$exists":false}}).await.unwrap(),expected as u64);
            assert_eq!(db.collection::<Document>(COLLECTIONS[1]).count_documents(doc! {"user_id":&user,"status":"revoked","access_token_encrypted":{"$exists":false}}).await.unwrap(),3);
            server.abort();
            db.drop().await.unwrap();
        }
    }
}
