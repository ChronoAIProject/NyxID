//! Verified Sandbox files, fenced pending storage and owner-message recovery.
use super::{api_key_mutation_service as transactions, assistant_upload_service::byte_budget};
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::{
        assistant_attachment::{AssistantAttachment, COLLECTION_NAME as ATTACHMENTS},
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as CONVERSATIONS, PlatformReplyRouting,
            TurnAttachment, TurnOrigin,
        },
        async_service_operation::{
            AsyncServiceOperation, COLLECTION_NAME as WATCHES, DeliveryBinding,
        },
        generated_artifact::{
            COLLECTION_NAME as ARTIFACTS, DELIVERIES, Delivery, Metadata, PendingArtifact,
            ResultManifest,
        },
    },
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use uuid::Uuid;
pub mod http;
mod mp4;
pub mod sandbox;
pub(crate) mod storage;

/// Recovery audit data is fixed metadata: never message/watch/operation IDs or arguments.
pub fn recovery_audit_data(http: bool) -> serde_json::Value {
    serde_json::json!({
        "tool_name": "nyx__artifact_delivery",
        "outcome": "requested",
        "transport": if http { "http" } else { "mcp" },
    })
}
pub async fn audit_recovery_request(
    db: &Database,
    actor: &super::audit_service::AuditActor,
    http: bool,
) {
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        actor,
        "assistant_mcp_tool_call",
        Some(recovery_audit_data(http)),
    )
    .await;
}

fn invalid() -> AppError {
    AppError::Conflict("Generated artifact association is invalid".into())
}
fn missing() -> AppError {
    AppError::NotFound("Artifact delivery not found".into())
}
pub fn web_delivery(d: &DeliveryBinding) -> bool {
    d.origin == TurnOrigin::User
        && d.channel.is_none()
        && d.reply_channel.is_none()
        && d.group_id.is_none()
        && d.group_request_id.is_none()
        && d.voice_request_id.is_none()
        && d.trigger_run_id.is_none()
        && d.platform_origin.is_none()
        && d.platform_reply == Some(PlatformReplyRouting::WebOnly)
        && d.invocation_id.is_none()
}
pub fn sandbox_contract(
    service: &super::mcp_service::McpToolService,
    submit: &super::mcp_service::McpToolEndpoint,
) -> bool {
    let Some(c) = &submit.async_operation else {
        return false;
    };
    submit.name == "submit_execution_handler"
        && submit.method == "POST"
        && submit.path == "/executions"
        && c.status_operation == "get_execution_handler"
        && c.result_operation == "get_execution_result_handler"
        && c.id_parameter == "operation_id"
        && c.id_field == "/operation_id"
        && c.status_field == "/status"
        && [
            (c.status_operation.as_str(), "/executions/{operation_id}"),
            (
                c.result_operation.as_str(),
                "/executions/{operation_id}/result",
            ),
        ]
        .iter()
        .all(|(name, path)| {
            service.endpoints.iter().any(|e| {
                e.name == *name
                    && e.path == *path
                    && e.method == "GET"
                    && e.target_id == submit.target_id
            })
        })
}
pub fn admitted(w: &AsyncServiceOperation) -> bool {
    w.artifact_mode.as_deref() == Some(sandbox::MODE) && web_delivery(&w.delivery)
}
pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(ARTIFACTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"watch_id":1,"source_result_identity":1,"artifact_index":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    for keys in [
        doc! {"expires_at":1,"state":1},
        doc! {"assistant_attachment_id":1},
        doc! {"user_id":1,"conversation_id":1},
    ] {
        db.collection::<Document>(ARTIFACTS)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"watch_id":1,"event_turn_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_id":1})
                .build(),
        )
        .await?;
    Ok(())
}
fn context_path(row: &PendingArtifact) -> [&str; 11] {
    [
        "data_encrypted",
        "conversation",
        &row.conversation_id,
        "turn",
        &row.turn_id,
        "watch",
        &row.watch_id,
        "operation",
        &row.operation_id,
        "source",
        &row.source_result_identity,
    ]
}
pub fn check_bson(row: &PendingArtifact) -> AppResult<()> {
    if bson::to_vec(row).map_err(|_| invalid())?.len() >= 16 * 1024 * 1024 {
        return Err(AppError::BadRequest("artifact_too_large".into()));
    }
    Ok(())
}
/// Caller owns one reservation from before the HTTP body read until this commit.
pub async fn persist(
    db: &Database,
    keys: &EncryptionKeys,
    w: &AsyncServiceOperation,
    result: &sandbox::ResultFiles,
    status: &sandbox::Status,
    reservation: &byte_budget::ByteReservation,
) -> AppResult<()> {
    if !admitted(w) || reservation.payload_limit != sandbox::RESERVATION {
        return Err(invalid());
    }
    let operation = w.operation_id.as_deref().ok_or_else(invalid)?;
    let expires_at = status.expires_at.unwrap_or(w.expires_at).min(w.expires_at);
    if expires_at <= Utc::now() {
        return Err(AppError::AssistantAttachmentExpired);
    }
    let mut rows = Vec::new();
    for (index, file) in result.files.iter().enumerate() {
        let mut row = PendingArtifact {
            id: Uuid::new_v4().to_string(),
            watch_id: w.id.clone(),
            user_id: w.user_id.clone(),
            conversation_id: w.conversation_id.clone(),
            turn_id: w.turn_id.clone(),
            operation_id: operation.into(),
            source_result_identity: result.identity.clone(),
            artifact_index: index,
            filename: file.filename.clone(),
            source_path: file.source_path.clone(),
            content_type: file.content_type.into(),
            size: file.bytes.len() as i64,
            sha256: file.sha256.clone(),
            preview_kind: file.preview.into(),
            source_status_version: status.version.clone(),
            content_revision: "1".into(),
            state: "pending".into(),
            assistant_attachment_id: Uuid::new_v4().to_string(),
            data_encrypted: None,
            secret_context_versions: Default::default(),
            created_at: Utc::now(),
            expires_at,
        };
        // New collection always uses authenticated v3 context, independently of legacy write migration.
        let path = context_path(&row);
        row.data_encrypted = Some(
            keys.encrypt_record(
                &file.bytes,
                &RecordContext::new(ARTIFACTS, &row.id, &row.user_id, &path)?,
            )
            .await?,
        );
        row.secret_context_versions
            .insert("data_encrypted".into(), 3);
        check_bson(&row)?;
        rows.push(row);
    }
    let manifest = ResultManifest {
        source_result_identity: result.identity.clone(),
        source_status_version: status.version.clone(),
        content_revision: "1".into(),
        count: rows.len(),
        delivery_reason: "delivery_pending".into(),
        expires_at,
    };
    let encrypted = super::record_field_service::encrypt(
        keys,
        WATCHES,
        &w.id,
        &w.user_id,
        "result_encrypted",
        result.summary.as_bytes(),
    )
    .await?;
    let mut versions = w.secret_context_versions.clone();
    super::credential_record_service::mark_version(&mut versions, keys, "result_encrypted");
    let database = db.clone();
    let watch = w.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<()> = Box::pin(async {
            super::erasure_admission::fence_in_session(&database,&watch.user_id,session).await?;
            let current = database.collection::<AsyncServiceOperation>(WATCHES).find_one(doc!{"_id":&watch.id,"user_id":&watch.user_id,"state":&watch.state,"lease_id":&watch.lease_id,"lease_until":{"$gt":bson::DateTime::now()},"operation_id":&watch.operation_id})
                .session(&mut *session).await?.ok_or_else(invalid)?;
            if !admitted(&current) || current.artifact_result.as_ref().is_some_and(|m|m.source_result_identity != manifest.source_result_identity) {return Err(invalid());}
            if manifest.expires_at <= Utc::now() {return Err(AppError::AssistantAttachmentExpired);}
            // Conversation write conflicts with owner deletion and cancellation.
            if database.collection::<Document>(CONVERSATIONS).update_one(doc!{"_id":&watch.conversation_id,"user_id":&watch.user_id},doc!{"$inc":{"async_operation_generation":1}}).session(&mut *session).await?.matched_count != 1 {return Err(missing());}
            for row in &rows {
                database.collection::<Document>(ARTIFACTS).update_one(doc!{"watch_id":&row.watch_id,"source_result_identity":&row.source_result_identity,"artifact_index":row.artifact_index as i64},doc!{"$setOnInsert":bson::to_document(row).map_err(|_|invalid())?}).upsert(true).session(&mut *session).await?;
            }
            database.collection::<Document>(WATCHES).update_one(doc!{"_id":&watch.id,"state":&watch.state,"lease_id":&watch.lease_id},doc!{"$set":{
                "artifact_result":bson::to_bson(&manifest).map_err(|_|invalid())?,"result_encrypted":bson::to_bson(&Some(&encrypted)).map_err(|_|invalid())?,"secret_context_versions":bson::to_bson(&versions).map_err(|_|invalid())?,
                "state":"ready","reason":if result.success {"completed"}else{"operation_failed"},"lease_id":null,"lease_until":null,"poll_at":bson::DateTime::now()}}).session(&mut *session).await?;
            Ok(())
        }).await; transactions::transaction_result(operation)
    }).await.map_err(transactions::map_transaction_error)
}
fn metadata(row: &PendingArtifact, expires: DateTime<Utc>) -> Metadata {
    Metadata {
        filename: Some(row.filename.clone()),
        source: Some("sandbox".into()),
        source_watch_id: Some(row.watch_id.clone()),
        source_turn_id: Some(row.turn_id.clone()),
        source_operation_id: Some(row.operation_id.clone()),
        source_result_identity: Some(row.source_result_identity.clone()),
        source_status_version: row.source_status_version.clone(),
        content_revision: Some(row.content_revision.clone()),
        preview_kind: Some(row.preview_kind.clone()),
        expires_at: Some(expires),
    }
}
fn exact_pending(row: &PendingArtifact, w: &AsyncServiceOperation, m: &ResultManifest) -> bool {
    row.watch_id == w.id
        && row.user_id == w.user_id
        && row.conversation_id == w.conversation_id
        && row.turn_id == w.turn_id
        && Some(row.operation_id.as_str()) == w.operation_id.as_deref()
        && row.source_result_identity == m.source_result_identity
}
/// Use persisted expiry and lineage, with no ciphertext reads or new deadline.
async fn attachment_expiry_in_session(
    db: &Database,
    session: &mut ClientSession,
    w: &AsyncServiceOperation,
    event_turn: &str,
    attachments: &[TurnAttachment],
) -> AppResult<DateTime<Utc>> {
    let manifest = w.artifact_result.as_ref().ok_or_else(invalid)?;
    let mut expiry = manifest.expires_at;
    for attachment in attachments
        .iter()
        .filter(|a| a.artifact.source_watch_id.as_deref() == Some(w.id.as_str()))
    {
        let saved = db
            .collection::<Document>(ATTACHMENTS)
            .find_one(doc! {"_id":&attachment.id,"user_id":&w.user_id,
            "conversation_id":&w.conversation_id,"turn_id":event_turn,
            "origin":"generated_artifact","source_watch_id":&w.id,
            "source_turn_id":&w.turn_id,"source_operation_id":&w.operation_id,
            "source_result_identity":&manifest.source_result_identity,
            "size":attachment.size,"content_type":&attachment.content_type})
            .projection(doc! {"expires_at":1})
            .session(&mut *session)
            .await?
            .ok_or_else(invalid)?;
        expiry = expiry.min(
            saved
                .get_datetime("expires_at")
                .map_err(|_| invalid())?
                .to_chrono(),
        );
    }
    Ok(expiry)
}
/// Runs within completion admission or exact owner-message recovery. No new execution.
async fn materialize(
    db: &Database,
    keys: &EncryptionKeys,
    w: &AsyncServiceOperation,
    session: &mut ClientSession,
    turn_id: &str,
    existing: &[TurnAttachment],
) -> AppResult<(Vec<TurnAttachment>, &'static str)> {
    if !admitted(w) {
        return Err(invalid());
    }
    if !matches!(w.reason.as_deref(), Some("completed" | "operation_failed")) {
        return Ok((Vec::new(), "delivery_failed"));
    }
    let Some(manifest) = &w.artifact_result else {
        return Ok((Vec::new(), "delivery_failed"));
    };
    if manifest.expires_at <= Utc::now() {
        return Ok((Vec::new(), "result_expired"));
    }
    let _reservation = byte_budget::reserve(sandbox::RESERVATION).await?;
    let mut cursor = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find(doc! {"watch_id":&w.id,"source_result_identity":&manifest.source_result_identity})
        .sort(doc! {"artifact_index":1})
        .limit(9)
        .session(&mut *session)
        .await?;
    let rows: Vec<PendingArtifact> = cursor.stream(&mut *session).try_collect().await?;
    if rows.len() != manifest.count
        || rows.len() > 8
        || rows.iter().any(|r| !exact_pending(r, w, manifest))
    {
        return Err(invalid());
    }
    let additional = rows
        .iter()
        .filter(|r| !existing.iter().any(|a| a.id == r.assistant_attachment_id))
        .count();
    if existing.len() + additional > super::assistant_nyxagent::MAX_TURN_ATTACHMENTS {
        return Ok((Vec::new(), "delivery_capacity_exhausted"));
    }
    let policy = super::assistant_upload_retention::artifact_policy_in_session(db, session).await?;
    let mut attachments = Vec::new();
    let mut total = 0usize;
    for row in rows {
        if row.expires_at <= Utc::now() {
            return Ok((Vec::new(), "result_expired"));
        }
        let normal_days =
            policy
                .tool_image_days
                .unwrap_or(if row.content_type.starts_with("image/") {
                    policy.image_days
                } else {
                    policy.document_days
                });
        let expiry = row
            .expires_at
            .min(manifest.expires_at)
            .min(Utc::now() + Duration::days(i64::from(normal_days)));
        let mut meta = TurnAttachment {
            artifact: metadata(&row, expiry),
            id: row.assistant_attachment_id.clone(),
            origin: "generated_artifact".into(),
            content_type: row.content_type.clone(),
            size: row.size,
            label: "Sandbox result".into(),
            chat_delivery: Some(false),
            image_input: None,
            image_inspection: Some(false),
            pages: None,
        };
        if row.state == "materialized" {
            super::assistant_upload_retention::generated_available_in_session(db, session, doc! {"_id":&row.assistant_attachment_id,"user_id":&w.user_id,"conversation_id":&w.conversation_id}, policy).await?;
            let saved = db.collection::<AssistantAttachment>(ATTACHMENTS).find_one(doc!{"_id":&row.assistant_attachment_id,"user_id":&w.user_id,"conversation_id":&w.conversation_id,"source_watch_id":&w.id,"source_result_identity":&manifest.source_result_identity}).session(&mut *session).await?.ok_or_else(missing)?;
            meta.artifact = saved.artifact;
            attachments.push(meta);
            continue;
        }
        if row.state != "pending" || row.secret_context_versions.get("data_encrypted") != Some(&3) {
            return Err(invalid());
        }
        let size = usize::try_from(row.size).map_err(|_| invalid())?;
        total = total
            .checked_add(size)
            .filter(|n| *n <= sandbox::TOTAL_BYTES)
            .ok_or_else(invalid)?;
        let path = context_path(&row);
        let bytes = keys
            .decrypt_record(
                row.data_encrypted.as_deref().ok_or_else(invalid)?,
                &RecordContext::new(ARTIFACTS, &row.id, &row.user_id, &path)?,
            )
            .await?;
        use sha2::{Digest, Sha256};
        if bytes.len() != size || hex::encode(Sha256::digest(&*bytes)) != row.sha256 {
            return Err(invalid());
        }
        let mut attachment = AssistantAttachment {
            artifact: meta.artifact.clone(),
            id: meta.id.clone(),
            origin: meta.origin.clone(),
            user_id: w.user_id.clone(),
            conversation_id: w.conversation_id.clone(),
            turn_id: turn_id.into(),
            content_type: row.content_type.clone(),
            size: row.size,
            data_encrypted: Vec::new(),
            secret_context_versions: Default::default(),
            created_at: Utc::now(),
        };
        attachment.data_encrypted =
            super::assistant_record_service::encrypt_image(keys, &attachment, &bytes).await?;
        attachment
            .secret_context_versions
            .insert("data_encrypted".into(), 3);
        db.collection::<AssistantAttachment>(ATTACHMENTS)
            .insert_one(&attachment)
            .session(&mut *session)
            .await?;
        db.collection::<Document>(ARTIFACTS).update_one(doc!{"_id":&row.id,"state":"pending"},doc!{"$set":{"state":"materialized","data_encrypted":null,"secret_context_versions":{}}}).session(&mut *session).await?;
        attachments.push(meta);
    }
    Ok((attachments, "materialized"))
}
pub async fn bind_in_session(
    db: &Database,
    keys: &EncryptionKeys,
    row: &mut AssistantConversation,
    w: &AsyncServiceOperation,
    session: &mut ClientSession,
) -> AppResult<()> {
    if w.artifact_mode.is_none() {
        return Ok(());
    }
    let turn = row.active_turn.as_ref().ok_or_else(invalid)?;
    let written = db
        .collection::<Document>(WATCHES)
        .find_one(doc! {"_id":&w.id})
        .projection(doc! {"artifact_event_turn_id":1})
        .session(&mut *session)
        .await?
        .ok_or_else(invalid)?;
    if written
        .get_str("artifact_event_turn_id")
        .ok()
        .is_some_and(|id| id != turn.turn_id)
    {
        return Err(invalid());
    }
    if db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME).find_one(doc!{"user_id":&w.user_id,"conversation_id":&w.conversation_id,"turn_id":&w.turn_id,"role":"user","origin":"user"}).projection(doc!{"_id":1}).session(&mut *session).await?.is_none() {return Err(invalid());}
    if !admitted(w)
        || turn.origin != TurnOrigin::Event
        || turn.turn_id == w.turn_id
        || super::async_service_operation::event_id(row) != Some(w.id.as_str())
        || turn
            .events
            .iter()
            .filter(|e| e.kind == super::async_service_operation::EVENT_KIND)
            .count()
            != 1
        || row.user_id != w.user_id
        || row.id != w.conversation_id
        || row.group_id.is_some()
        || turn.stop_requested
    {
        return Err(invalid());
    }
    // A rejected/missing result still delivers its bounded failure event. It
    // has no verified file lineage and must not create a recovery association.
    if w.artifact_result.is_none() {
        return Ok(());
    }
    let (attachments, reason) =
        materialize(db, keys, w, session, &turn.turn_id, &turn.attachments).await?;
    let turn = row.active_turn.as_mut().ok_or_else(invalid)?;
    for attachment in attachments {
        if !turn.attachments.iter().any(|a| a.id == attachment.id) {
            turn.attachments.push(attachment);
        }
    }
    db.collection::<Document>(WATCHES).update_one(doc!{"_id":&w.id,"state":"queued"},doc!{"$set":{"artifact_result.delivery_reason":reason,"artifact_event_turn_id":&turn.turn_id}}).session(&mut *session).await?;
    Ok(())
}
/// Settlement owns the message identity and writes its immutable recovery lineage.
pub async fn associate_in_session(
    db: &Database,
    row: &AssistantConversation,
    message: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(id) = super::async_service_operation::event_id(row) else {
        return Ok(());
    };
    let Some(w) = db.collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id":id,"user_id":&row.user_id,"conversation_id":&row.id,"state":{"$in":["queued","delivered"]}})
        .session(&mut *session).await? else { return Ok(()); };
    if w.artifact_mode.is_none() {
        return Ok(());
    }
    if !admitted(&w) {
        return Err(invalid());
    }
    let Some(manifest) = &w.artifact_result else {
        return Ok(());
    };
    let turn = row.active_turn.as_ref().ok_or_else(invalid)?;
    if turn.origin != TurnOrigin::Event
        || w.artifact_event_turn_id.as_deref() != Some(turn.turn_id.as_str())
        || turn.turn_id == w.turn_id
        || row.group_id.is_some()
        || turn
            .events
            .iter()
            .filter(|e| e.kind == super::async_service_operation::EVENT_KIND)
            .count()
            != 1
    {
        return Err(invalid());
    }
    let operation = w.operation_id.as_deref().ok_or_else(invalid)?;
    let mut cursor = db
        .collection::<PendingArtifact>(ARTIFACTS)
        .find(doc! {"watch_id":&w.id,"source_result_identity":&manifest.source_result_identity})
        .projection(doc! {"data_encrypted":0})
        .sort(doc! {"artifact_index":1})
        .limit(9)
        .session(&mut *session)
        .await?;
    let pending: Vec<PendingArtifact> = cursor.stream(&mut *session).try_collect().await?;
    if pending.len() != manifest.count
        || pending.len() > 8
        || pending.iter().any(|p| !exact_pending(p, &w, manifest))
    {
        return Err(invalid());
    }
    let selected: Vec<&TurnAttachment> = turn
        .attachments
        .iter()
        .filter(|a| a.artifact.source_watch_id.as_deref() == Some(id))
        .collect();
    for attachment in &selected {
        if attachment.origin != "generated_artifact"
            || attachment.artifact.source_turn_id.as_deref() != Some(w.turn_id.as_str())
            || attachment.artifact.source_operation_id.as_deref() != Some(operation)
            || attachment.artifact.source_result_identity.as_deref()
                != Some(manifest.source_result_identity.as_str())
            || !pending.iter().any(|p| {
                p.state == "materialized"
                    && p.assistant_attachment_id == attachment.id
                    && p.content_type == attachment.content_type
                    && p.size == attachment.size
            })
        {
            return Err(invalid());
        }
    }
    let expiry =
        attachment_expiry_in_session(db, session, &w, &turn.turn_id, &turn.attachments).await?;
    let ids: Vec<String> = selected.into_iter().map(|a| a.id.clone()).collect();
    let expected_ids: Vec<String> = pending
        .iter()
        .filter(|p| p.state == "materialized")
        .map(|p| p.assistant_attachment_id.clone())
        .collect();
    if ids != expected_ids
        || (manifest.delivery_reason == "materialized" && ids.len() != manifest.count)
    {
        return Err(invalid());
    }
    let delivery = Delivery {
        message_id: message.into(),
        user_id: row.user_id.clone(),
        conversation_id: row.id.clone(),
        originating_turn_id: w.turn_id,
        event_turn_id: turn.turn_id.clone(),
        watch_id: w.id,
        operation_id: operation.to_owned(),
        source_result_identity: manifest.source_result_identity.clone(),
        expires_at: expiry,
        attachment_ids: ids,
    };
    let collection = db.collection::<Delivery>(DELIVERIES);
    if collection
        .find_one(doc! {"watch_id":&delivery.watch_id,"event_turn_id":&delivery.event_turn_id})
        .session(&mut *session)
        .await?
        .is_some_and(|saved| saved != delivery)
    {
        return Err(invalid());
    }
    collection
        .update_one(
            doc! {"_id":message},
            doc! {"$setOnInsert":bson::to_document(&delivery).map_err(|_|invalid())?},
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    let saved = collection
        .find_one(doc! {"_id":message})
        .session(session)
        .await?
        .ok_or_else(invalid)?;
    if saved != delivery {
        return Err(invalid());
    }
    Ok(())
}

mod recovery;
pub use recovery::{
    recover, recover_for_chat, recovery_confirmation_arguments, recovery_definition,
};
pub async fn discard_in_session(
    db: &Database,
    filter: Document,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(ARTIFACTS)
        .update_many(
            filter,
            doc! {"$set":{"state":"discarded","data_encrypted":null,"secret_context_versions":{}}},
        )
        .session(session)
        .await?;
    Ok(())
}
pub async fn sweep(db: &Database) -> AppResult<()> {
    db.collection::<Document>(ARTIFACTS).update_many(doc!{"expires_at":{"$lte":bson::DateTime::now()},"state":{"$in":["pending","materializing"]}},doc!{"$set":{"state":"expired","data_encrypted":null,"secret_context_versions":{}}}).await?;
    Ok(())
}
#[cfg(test)]
mod tests;

pub struct DeliveryState {
    pub state: String,
    pub count: usize,
    pub recovery_available: bool,
}
pub async fn delivery_states(
    db: &Database,
    user: &str,
    conversation: &str,
    messages: &[String],
) -> AppResult<std::collections::HashMap<String, DeliveryState>> {
    let deliveries: Vec<Delivery> = db
        .collection::<Delivery>(DELIVERIES)
        .find(doc! {"_id":{"$in":messages},"user_id":user,"conversation_id":conversation})
        .await?
        .try_collect()
        .await?;
    let ids: Vec<&str> = deliveries.iter().map(|d| d.watch_id.as_str()).collect();
    let watches: Vec<AsyncServiceOperation> = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find(doc! {"_id":{"$in":ids},"user_id":user,"conversation_id":conversation})
        .await?
        .try_collect()
        .await?;
    let attachment_ids: Vec<String> = deliveries
        .iter()
        .flat_map(|d| d.attachment_ids.iter().cloned())
        .collect();
    let expired = super::assistant_upload_retention::expired_ids(db, user, &attachment_ids).await?;
    let mut states = std::collections::HashMap::new();
    for delivery in deliveries {
        let state = if delivery.expires_at <= Utc::now()
            || delivery
                .attachment_ids
                .iter()
                .any(|id| expired.contains(id))
        {
            "result_expired"
        } else {
            watches
                .iter()
                .find(|w| w.id == delivery.watch_id)
                .and_then(|w| {
                    if matches!(w.state.as_str(), "cancelling" | "cancelled") {
                        Some("cancelled")
                    } else if w.reason.as_deref() == Some("result_expired") {
                        Some("result_expired")
                    } else {
                        w.artifact_result
                            .as_ref()
                            .map(|m| m.delivery_reason.as_str())
                    }
                })
                .unwrap_or("delivery_failed")
        };
        states.insert(
            delivery.message_id,
            DeliveryState {
                state: state.into(),
                count: delivery.attachment_ids.len(),
                recovery_available: matches!(
                    state,
                    "delivery_pending" | "delivery_failed" | "delivery_capacity_exhausted"
                ),
            },
        );
    }
    Ok(states)
}
