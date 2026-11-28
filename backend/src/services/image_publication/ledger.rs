//! Immutable revisions, exact attempt fences and the one-way create barrier.
use super::*;
use mongodb::ClientSession;

pub(super) async fn load(db: &Database, id: &str) -> AppResult<ImagePublication> {
    db.collection(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or_else(|| refused("draft_unavailable"))
}

pub(super) async fn update(
    db: &Database,
    row: &mut ImagePublication,
    state: &str,
    stage: &str,
    status: Option<u16>,
) -> AppResult<()> {
    if state == "ready" && stage == "ready" {
        row.last_code = None;
        row.uploaded = true;
    }
    let now = Utc::now();
    let filter = doc! {
        "_id": &row.id,
        "state": &row.state,
        "attempt": &row.attempt,
    };
    let values = doc! {
        "state": state,
        "stage": stage,
        "http_status": status.map(i32::from),
        "last_code": &row.last_code,
        "image": &row.image,
        "uploaded": row.uploaded,
        "post": &row.post,
        "verification": &row.verification,
        "posted_at": row.posted_at.map(bson::DateTime::from_chrono),
        "checked_at": row.checked_at.map(bson::DateTime::from_chrono),
        "verified_at": row.verified_at.map(bson::DateTime::from_chrono),
        "updated_at": bson::DateTime::from_chrono(now),
        "lease_until": bson::DateTime::from_chrono(row.lease_until),
    };
    let changed = db
        .collection::<ImagePublication>(COLLECTION_NAME)
        .update_one(filter, doc! {"$set": values})
        .await?;
    if changed.matched_count != 1 {
        return Err(refused("draft_changed"));
    }
    row.state = state.into();
    row.stage = stage.into();
    row.http_status = status;
    row.updated_at = now;
    Ok(())
}

pub(super) async fn effect_gate(
    db: &Database,
    user: &str,
    id: &str,
    state: &str,
    attempt: &str,
) -> AppResult<()> {
    let filter = doc! {"_id":id,"user_id":user,"state":state,"attempt":attempt};
    let exists = db
        .collection::<ImagePublication>(COLLECTION_NAME)
        .find_one(filter)
        .await?;
    if exists.is_none() {
        return Err(refused("draft_cancelled_or_changed"));
    }
    Ok(())
}

pub(super) fn preview_key(chat: &ChatAuthority, connection: &str, request: &str) -> String {
    mcp::canonical_sha256(json!({
        "version": 2,
        "user": chat.user_id,
        "conversation": chat.conversation_id,
        "connection": connection,
        "request_id": request,
    }))
}

fn same_payload(a: &ImagePublication, b: &ImagePublication) -> bool {
    a.company == b.company
        && a.caption_sha256 == b.caption_sha256
        && a.content_sha256 == b.content_sha256
        && a.attachment_id == b.attachment_id
        && a.metadata_identity == b.metadata_identity
        && a.authority == b.authority
        && a.contract == b.contract
}

pub(super) fn replaceable(state: &str) -> bool {
    matches!(
        state,
        "ready" | "upload_failed" | "processing" | "cancelled" | "superseded" | "failed"
    )
}

pub(super) fn can_reuse_image(old: &ImagePublication, next: &ImagePublication) -> bool {
    replaceable(&old.state)
        && old.state != "upload_failed"
        && old.image.is_some()
        && (old.uploaded || old.state == "ready")
        && old.company == next.company
        && old.authority == next.authority
        && old.contract == next.contract
        && old.content_sha256 == next.content_sha256
}

pub(super) async fn retire_cards(
    db: &Database,
    row: &ImagePublication,
    session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let filter = doc! {
        "user_id": &row.user_id,
        "conversation_id": &row.conversation_id,
        "confirmation.image_publication.draft_id": &row.id,
        "status": {"$in": ["pending", "allowed"]},
    };
    let values = doc! {"status": "expired", "expires_at": bson::DateTime::now()};
    let cards = db.collection::<bson::Document>("assistant_acknowledgements");
    let operation = cards.update_many(filter, doc! {"$set": values});
    match session {
        Some(session) => operation.session(session).await?,
        None => operation.await?,
    };
    Ok(())
}

async fn resume(
    db: &Database,
    session: &mut ClientSession,
    mut row: ImagePublication,
    proposed: &ImagePublication,
    transfer: bool,
) -> AppResult<(ImagePublication, bool)> {
    let expired = row.lease_until <= Utc::now();
    let retry = row.state == "upload_failed"
        || expired && matches!(row.state.as_str(), "preparing" | "processing");
    if !retry {
        return Ok((row, false));
    }
    if row.state != "processing" && !transfer {
        return Err(refused("preview_retry_requires_file_approval"));
    }
    let state = if row.state == "processing" {
        "processing"
    } else {
        "preparing"
    };
    if state == "preparing" {
        row.image = None;
        row.uploaded = false;
    }
    let filter = doc! {"_id": &row.id, "state": &row.state, "attempt": &row.attempt};
    let values = doc! {
        "state": state,
        "attempt": &proposed.attempt,
        "lease_until": bson::DateTime::from_chrono(proposed.lease_until),
        "image": &row.image,
        "uploaded": row.uploaded,
    };
    let changed = db
        .collection::<ImagePublication>(COLLECTION_NAME)
        .update_one(filter, doc! {"$set": values})
        .session(session)
        .await?;
    if changed.matched_count != 1 {
        return Err(refused("draft_claim_changed"));
    }
    row.state = state.into();
    row.attempt.clone_from(&proposed.attempt);
    row.lease_until = proposed.lease_until;
    Ok((row, true))
}

async fn claim_in_session(
    db: &Database,
    session: &mut ClientSession,
    proposed: &ImagePublication,
    prior: Option<&ImagePublication>,
    transfer: bool,
) -> AppResult<(ImagePublication, bool)> {
    crate::services::erasure_admission::fence_in_session(db, &proposed.user_id, session).await?;
    let rows = db.collection::<ImagePublication>(COLLECTION_NAME);
    if let Some(existing) = rows
        .find_one(doc! {"key": &proposed.key})
        .session(&mut *session)
        .await?
    {
        if !same_payload(&existing, proposed) {
            return Err(refused("request_id_payload_changed"));
        }
        return resume(db, session, existing, proposed, transfer).await;
    }

    let mut next = proposed.clone();
    if let Some(old) = prior {
        if !replaceable(&old.state) {
            return Err(refused("prior_draft_already_submitted_or_busy"));
        }
        // This exact predecessor contends with its create barrier. Other
        // deliberate drafts in the conversation remain independent.
        let filter = doc! {"_id": &old.id, "state": &old.state, "attempt": &old.attempt};
        let values = doc! {"state": "superseded", "updated_at": bson::DateTime::now()};
        let changed = rows
            .update_one(filter, doc! {"$set": values})
            .session(&mut *session)
            .await?;
        if changed.matched_count != 1 {
            return Err(refused("prior_draft_changed"));
        }
        retire_cards(db, old, Some(&mut *session)).await?;
        if can_reuse_image(old, &next) {
            next.image.clone_from(&old.image);
            next.uploaded = true;
        }
    }
    if !transfer && next.image.is_none() {
        return Err(refused("preview_retry_state_changed"));
    }
    rows.insert_one(&next).session(session).await?;
    Ok((next, true))
}

pub(super) async fn preview_claim(
    guard: &Guard<'_>,
    adapter: &dyn ImagePublisher,
) -> AppResult<(ImagePublication, bool)> {
    let p = guard.prepared;
    let now = Utc::now();
    let proposed = ImagePublication {
        id: Uuid::new_v4().to_string(),
        key: preview_key(&p.chat, &guard.service.service_id, &p.request_id),
        user_id: p.chat.user_id.clone(),
        conversation_id: p.chat.conversation_id.clone(),
        connection_id: guard.service.service_id.clone(),
        authority: p.authority.clone(),
        contract: p.contract.clone(),
        company: p.company.clone(),
        caption_sha256: hash(p.caption.as_bytes()),
        wire_caption_sha256: hash(adapter.wire_caption(&p.caption).as_bytes()),
        attempt: Uuid::new_v4().to_string(),
        lease_until: now + chrono::Duration::seconds(90),
        attachment_id: p.attachment.clone(),
        content_sha256: p.sha256.clone(),
        metadata_identity: p.identity.clone(),
        state: "preparing".into(),
        image: None,
        uploaded: false,
        post: None,
        stage: "initialization".into(),
        last_code: None,
        http_status: None,
        verification: "not_submitted".into(),
        posted_at: None,
        checked_at: None,
        verified_at: None,
        created_at: now,
        updated_at: now,
    };
    let db = guard.db.clone();
    let prior = p.draft.clone();
    let transfer = p.transfer;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = claim_in_session(&db, session, &proposed, prior.as_ref(), transfer).await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}
