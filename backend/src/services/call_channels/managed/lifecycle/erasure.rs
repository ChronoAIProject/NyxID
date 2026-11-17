use super::*;

/// Deletion preparation closes managed admission even while unsettled effects
/// still block acceptance. It does not delete custody or financial evidence.
pub async fn prepare_erasure(db: &Database, owner: &str) -> AppResult<()> {
    if db
        .collection::<Document>(ManagedLineClaim::COLLECTION_NAME)
        .find_one(doc! {"user_id":owner,"state":{"$in":["claimed","suspended","releasing"]}})
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(());
    }
    transaction_service::run_db_only(db, &(db, owner), |tx, ctx| {
        Box::pin(async move {
            let &(db, owner) = ctx;
            let now = Utc::now();
            // Same owner row as new claims/renewals, without requiring an active
            // owner: disabled/departing people must retain cleanup authority.
            db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":owner},
                    doc! {"$inc":{"channel_adapter_admission_fence":1_i64}},
                )
                .session(&mut *tx)
                .await?;
            if db
                .collection::<ManagedLineErasure>(ManagedLineErasure::COLLECTION_NAME)
                .find_one(doc! {"user_id":owner})
                .session(&mut *tx)
                .await?
                .is_none()
            {
                db.collection::<ManagedLineErasure>(ManagedLineErasure::COLLECTION_NAME)
                    .insert_one(ManagedLineErasure {
                        id: Uuid::new_v4().to_string(),
                        user_id: owner.into(),
                        created_at: now,
                    })
                    .session(&mut *tx)
                    .await?;
            }
            let mut cursor = db
                .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
                .find(doc! {"user_id":owner,"state":{"$in":["claimed","suspended"]}})
                .limit(100)
                .session(&mut *tx)
                .await?;
            let mut rows = vec![];
            while let Some(c) = cursor.next(&mut *tx).await.transpose()? {
                rows.push(c);
            }
            for c in rows {
                release_in_session(db, tx, &c, now).await?;
            }
            Ok(())
        })
    })
    .await?;
    let rows: Vec<ManagedLineClaim> = db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find(doc! {"user_id":owner,"state":"releasing"})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for c in rows {
        complete_release(db, &c.id, Utc::now()).await?;
    }
    Ok(())
}
pub(crate) async fn erasure_guard(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    if db
        .collection::<Document>(ManagedLineClaim::COLLECTION_NAME)
        .find_one(doc! {"user_id":owner,"state":{"$in":["claimed","suspended","releasing"]}})
        .session(tx)
        .await?
        .is_some()
    {
        return Err(conflict(
            "Managed phone line cleanup is still pending; retry deletion after all effects are accounted",
        ));
    }
    Ok(())
}
/// Keep claim identity/generation and paid receipts; remove tenant identity,
/// specialist, consent and binding fields after accepted deletion. Page size is
/// independent of retained historical rentals and provider routing records.
pub async fn erase_personal_fields(db: &Database, owner: &str) -> AppResult<bool> {
    let rows: Vec<ManagedLineClaim> = db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find(doc! {"user_id":owner})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        return Ok(true);
    }
    transaction_service::run_db_only(db,&(db,owner,&rows),|tx,ctx|Box::pin(async move{
        let &(db,owner,rows)=ctx;
        for c in rows {
            if !effects_accounted(db,tx,&c.id).await? || matches!(c.state,LineState::Claimed|LineState::Suspended|LineState::Releasing){return Err(unavailable());}
            db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"user_id":owner},doc!{"$set":{"user_id":"","agent_id":"","accepted_terms":{"managed_line":"","elevenlabs_eua":bson::Bson::Null},"voice_binding_id":bson::Bson::Null,"sms_binding_id":bson::Bson::Null,"caller_confirmations":false,"auto_renew":false},"$inc":{"fence":1_i64}}).session(&mut *tx).await?;
        }Ok(())
    })).await?;
    Ok(false)
}
