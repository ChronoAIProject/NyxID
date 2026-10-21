use super::*;
use crate::models::lark_base_write_photo::{COLLECTION_NAME as PHOTOS, LarkBaseWritePhoto};
use crate::services::api_key_mutation_service as tx;
use mongodb::{
    ClientSession,
    bson::{self, doc},
};

pub(super) enum Claim {
    Active(Box<LarkBaseWrite>),
    Replay(String),
}

fn state_name(state: WriteState) -> &'static str {
    match state {
        WriteState::Uploading => "uploading",
        WriteState::Dispatching => "dispatching",
        WriteState::Succeeded => "succeeded",
        WriteState::FailedDefinite => "failed_definite",
        WriteState::FailedPreDispatch => "failed_pre_dispatch",
        WriteState::Uncertain => "uncertain",
    }
}

fn conflict(row: &LarkBaseWrite) -> AppError {
    match row.state {
        WriteState::Succeeded => AppError::Conflict(format!(
            "Photo already belongs to record {}; use bitable_record_update to change its fields",
            row.record_id.as_deref().unwrap_or("unknown")
        )),
        WriteState::Uploading | WriteState::Dispatching if row.deadline_at > Utc::now() => {
            AppError::Conflict("A Base write for this photo is in progress; try again shortly".into())
        }
        WriteState::Uncertain | WriteState::Dispatching => AppError::Conflict(
            "Base write outcome unknown; check the Base; NyxID will not create it again".into()
        ),
        WriteState::Uploading => AppError::Conflict(
            "A previous Base upload for this photo expired or its authority changed; try again shortly".into()
        ),
        WriteState::FailedPreDispatch | WriteState::FailedDefinite => AppError::Internal(
            "Inactive Base write reached the reservation conflict path".into()
        ),
    }
}

pub(super) async fn claim(
    db: &Database,
    proposed: &LarkBaseWrite,
    photos: &[String],
) -> AppResult<Claim> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let proposed = proposed.clone();
    let photos = photos.to_vec();
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<Claim> = async {
            crate::services::erasure_admission::fence_in_session(&db, &proposed.user_id, session).await?;
            let calls = db.collection::<LarkBaseWrite>(COLLECTION);
            let existing = calls.find_one(doc! {"key":&proposed.key}).session(&mut *session).await?;
            let mut next = proposed.clone();
            if let Some(row) = &existing {
                let same_authority = row.principal == proposed.principal && row.contract_digests == proposed.contract_digests;
                if row.state == WriteState::Succeeded {
                    if row.logical_request_sha256 != proposed.logical_request_sha256 { return Err(conflict(row)); }
                    return Ok(Claim::Replay(row.record_id.clone().ok_or_else(|| conflict(row))?));
                }
                if row.state == WriteState::Dispatching && row.deadline_at <= Utc::now() {
                    terminal_in_session(&db, session, row, WriteState::Uncertain, "deadline", None).await?;
                    // Commit the uncertainty before returning its refusal.
                    return Ok(Claim::Active(Box::new(LarkBaseWrite { state: WriteState::Uncertain, ..row.clone() })));
                }
                if !matches!(row.state, WriteState::FailedPreDispatch | WriteState::FailedDefinite)
                    && !(row.state == WriteState::Uploading && row.deadline_at <= Utc::now() && same_authority) {
                    return Err(conflict(row));
                }
                next.id = row.id.clone();
                next.created_at = row.created_at;
                if same_authority { next.files = row.files.clone(); }
            }
            // Check holders before inserting/replacing the incoming write. A
            // stale dispatching holder must commit its uncertainty while the
            // incoming call is refused, without activating a partial attempt.
            for key in &photos {
                let reservations = db.collection::<LarkBaseWritePhoto>(PHOTOS);
                if let Some(photo) = reservations.find_one(doc! {"photo_key":key}).session(&mut *session).await?
                    && photo.write_id != next.id {
                    let holder = calls.find_one(doc! {"_id":&photo.write_id}).session(&mut *session).await?
                        .ok_or_else(|| AppError::Conflict("Photo is reserved by another Base write".into()))?;
                    if holder.state == WriteState::Dispatching && holder.deadline_at <= Utc::now() {
                        if !terminal_in_session(&db, session, &holder, WriteState::Uncertain, "deadline", None).await? {
                            return Err(AppError::Conflict("Base write attempt changed; try again shortly".into()));
                        }
                        return Ok(Claim::Active(Box::new(LarkBaseWrite { state: WriteState::Uncertain, ..holder })));
                    }
                    if matches!(holder.state, WriteState::FailedPreDispatch | WriteState::FailedDefinite) {
                        reservations.delete_one(doc! {"_id":&photo.id,"write_id":&photo.write_id,"attempt":&photo.attempt})
                            .session(&mut *session).await?;
                    } else {
                        return Err(conflict(&holder));
                    }
                }
            }
            if let Some(row) = existing {
                let changed = calls.replace_one(doc! {"_id":&row.id,"attempt":&row.attempt,"state":state_name(row.state)}, &next)
                    .session(&mut *session).await?;
                if changed.matched_count != 1 { return Err(AppError::Conflict("Base write attempt changed; try again shortly".into())); }
            } else {
                calls.insert_one(&next).session(&mut *session).await?;
            }
            for key in &photos {
                let reservations = db.collection::<LarkBaseWritePhoto>(PHOTOS);
                if let Some(photo) = reservations.find_one(doc! {"photo_key":key}).session(&mut *session).await? {
                    if photo.write_id != next.id {
                        return Err(AppError::Conflict("Base write photo reservation changed; try again shortly".into()));
                    }
                    reservations.update_one(doc! {"_id":&photo.id,"write_id":&next.id,"attempt":&photo.attempt},
                        doc! {"$set":{"attempt":&next.attempt,"expires_at":bson::Bson::Null}}).session(&mut *session).await?;
                } else {
                    reservations.insert_one(LarkBaseWritePhoto {
                        id: Uuid::new_v4().to_string(), photo_key:key.clone(), user_id:next.user_id.clone(),
                        write_id:next.id.clone(), attempt:next.attempt.clone(), created_at:Utc::now(), expires_at:None,
                    }).session(&mut *session).await?;
                }
            }
            Ok(Claim::Active(Box::new(next)))
        }.await;
        tx::transaction_result(result)
    }).await.map_err(tx::map_transaction_error)
}

pub(crate) async fn effect_gate(
    db: &Database,
    row: &LarkBaseWrite,
    phase: WriteState,
) -> AppResult<()> {
    if db
        .collection::<LarkBaseWrite>(COLLECTION)
        .find_one(doc! {
            "_id":&row.id,"attempt":&row.attempt,"state":state_name(phase),
            "deadline_at":{"$gt":bson::DateTime::now()}
        })
        .await?
        .is_none()
    {
        return Err(AppError::Conflict(
            "Base write attempt is no longer dispatchable".into(),
        ));
    }
    Ok(())
}

pub(super) async fn save_file(
    db: &Database,
    row: &LarkBaseWrite,
    file: &UploadedFile,
) -> AppResult<()> {
    let result = db.collection::<LarkBaseWrite>(COLLECTION).update_one(doc! {
        "_id":&row.id,"attempt":&row.attempt,"state":"uploading","deadline_at":{"$gt":bson::DateTime::now()}
    },doc! {"$push":{"files":bson::to_bson(file).map_err(|_| AppError::Internal("Cannot encode Base upload metadata".into()))?},"$set":{"updated_at":bson::DateTime::now()}}).await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Base write upload attempt changed".into(),
        ));
    }
    Ok(())
}

pub(super) async fn barrier(
    db: &Database,
    row: &LarkBaseWrite,
    photos: &[String],
    body_digest: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let row = row.clone();
    let photos = photos.to_vec();
    let body_digest = body_digest.to_owned();
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            // Writing every reservation couples the parent barrier to any
            // concurrent release/reacquire transaction (Astra r5 #1).
            for key in &photos {
                if db.collection::<LarkBaseWritePhoto>(PHOTOS).update_one(doc! {
                    "photo_key":key,"write_id":&row.id,"attempt":&row.attempt
                },doc! {"$set":{"attempt":&row.attempt}}).session(&mut *session).await?.matched_count != 1 {
                    return Err(AppError::Conflict("Base write photo reservation changed".into()));
                }
            }
            if db.collection::<LarkBaseWrite>(COLLECTION).update_one(doc! {
                "_id":&row.id,"attempt":&row.attempt,"state":"uploading","deadline_at":{"$gt":bson::DateTime::now()}
            },doc! {"$set":{"state":"dispatching","create_body_sha256":&body_digest,"updated_at":bson::DateTime::now()}})
                .session(&mut *session).await?.matched_count != 1 {
                return Err(AppError::Conflict("Base write dispatch attempt changed".into()));
            }
            Ok(())
        }.await;
        tx::transaction_result(result)
    }).await.map_err(tx::map_transaction_error)
}

async fn terminal_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &LarkBaseWrite,
    state: WriteState,
    code: &str,
    record: Option<&str>,
) -> AppResult<bool> {
    if !matches!(
        (row.state, state),
        (WriteState::Uploading, WriteState::FailedPreDispatch)
            | (
                WriteState::Dispatching,
                WriteState::Succeeded | WriteState::FailedDefinite | WriteState::Uncertain
            )
    ) {
        return Err(AppError::Internal(
            "Invalid Base write settlement transition".into(),
        ));
    }
    let expires = bson::DateTime::from_chrono(Utc::now() + chrono::Duration::days(400));
    let changed = db
        .collection::<LarkBaseWrite>(COLLECTION)
        .update_one(
            doc! {
                "_id":&row.id,"attempt":&row.attempt,"state":state_name(row.state)
            },
            doc! {"$set":{"state":state_name(state),"record_id":record,"last_code":code,
            "updated_at":bson::DateTime::now(),"expires_at":expires}},
        )
        .session(&mut *session)
        .await?;
    if changed.matched_count != 1 {
        return Ok(false);
    }
    let filter = doc! {"write_id":&row.id,"attempt":&row.attempt};
    if matches!(
        state,
        WriteState::FailedPreDispatch | WriteState::FailedDefinite
    ) {
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .delete_many(filter)
            .session(&mut *session)
            .await?;
    } else {
        db.collection::<LarkBaseWritePhoto>(PHOTOS)
            .update_many(filter, doc! {"$set":{"expires_at":expires}})
            .session(&mut *session)
            .await?;
    }
    Ok(true)
}

pub(super) async fn terminal(
    db: &Database,
    row: &LarkBaseWrite,
    state: WriteState,
    code: &str,
    record: Option<&str>,
) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let row = row.clone();
    let code = code.to_owned();
    let record = record.map(str::to_owned);
    session
        .start_transaction()
        .and_run2(async move |session| {
            tx::transaction_result(
                terminal_in_session(&db, session, &row, state, &code, record.as_deref()).await,
            )
        })
        .await
        .map_err(tx::map_transaction_error)
}

/// Flag-independent, bounded maintenance. State and attempt are always fenced.
pub(crate) async fn sweep(db: &Database) -> AppResult<usize> {
    use futures::TryStreamExt;
    let stale: Vec<LarkBaseWrite> = db
        .collection::<LarkBaseWrite>(COLLECTION)
        .find(doc! {
            "state":{"$in":["uploading","dispatching"]},"deadline_at":{"$lte":bson::DateTime::now()}
        })
        .sort(doc! {"deadline_at":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let mut count = 0;
    for row in stale {
        let state = if row.state == WriteState::Uploading {
            WriteState::FailedPreDispatch
        } else {
            WriteState::Uncertain
        };
        count += usize::from(terminal(db, &row, state, "deadline", None).await?);
    }
    let expired: Vec<LarkBaseWrite> = db
        .collection::<LarkBaseWrite>(COLLECTION)
        .find(doc! {
            "state":{"$nin":["uploading","dispatching"]},"expires_at":{"$lte":bson::DateTime::now()}
        })
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for row in expired {
        let mut session = db.client().start_session().await?;
        let db = db.clone();
        session
            .start_transaction()
            .and_run2(async move |session| {
                let result: AppResult<()> = async {
                    let deleted = db
                        .collection::<LarkBaseWrite>(COLLECTION)
                        .delete_one(doc! {
                            "_id":&row.id,"attempt":&row.attempt,"state":state_name(row.state),
                            "expires_at":{"$lte":bson::DateTime::now()}
                        })
                        .session(&mut *session)
                        .await?;
                    if deleted.deleted_count == 1 {
                        db.collection::<LarkBaseWritePhoto>(PHOTOS)
                            .delete_many(doc! {"write_id":&row.id,"attempt":&row.attempt})
                            .session(&mut *session)
                            .await?;
                    }
                    Ok(())
                }
                .await;
                tx::transaction_result(result)
            })
            .await
            .map_err(tx::map_transaction_error)?;
    }
    Ok(count)
}
