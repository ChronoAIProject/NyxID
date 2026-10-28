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
            COLLECTION_NAME as ARTIFACTS, ContentSeries, DELIVERIES, Delivery, Metadata,
            PendingArtifact, ResultManifest, SERIES,
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

/// Stable instructions are added only for enabled owner readers. Flag-off text
/// remains byte-for-byte baseline; admitted watches and HTTP recovery drain separately.
pub const GUIDANCE: &str = "\nA completed cloud execution does not prove file delivery. Claim attached/downloadable only after NyxID materializes a private attachment. For an owner's request to recover saved Web cloud files, use nyx__artifact_delivery with an empty object: the server binds the saved result. Never choose result IDs, resubmit an execution, or request machine setup for that recovery.\n";

pub async fn enabled(db: &Database, user: &str) -> AppResult<bool> {
    super::feature_flag_service::personal_flag_enabled(
        db,
        user,
        super::feature_flag_service::ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY,
    )
    .await
}

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
/// Trust the resolved producer catalog, never an instance/display name or URL.
/// Endpoint and target checks remain additional execution-binding fences.
pub async fn trusted_sandbox_catalog(
    db: &Database,
    service: &super::mcp_service::McpToolService,
    submit: &super::mcp_service::McpToolEndpoint,
) -> AppResult<Option<String>> {
    use super::mcp_service::McpToolSource;
    use crate::models::downstream_service::{COLLECTION_NAME, DownstreamService};
    if !sandbox_contract(service, submit) {
        return Ok(None);
    }
    let id = match &service.source {
        McpToolSource::Platform {
            downstream_service_id,
        } => Some(downstream_service_id.as_str()),
        McpToolSource::UserManaged {
            catalog_service_id, ..
        } => catalog_service_id.as_deref(),
        McpToolSource::Internal => None,
    };
    let Some(id) = id else {
        return Ok(None);
    };
    let catalog = db
        .collection::<DownstreamService>(COLLECTION_NAME)
        .find_one(doc! {"_id":id,"is_active":true,"tools_manager_created":{"$ne":true}})
        .await?;
    Ok(catalog
        .filter(|catalog| {
            super::catalog_spec_registry::spec_key_for_slug(&catalog.slug) == Some("chrono-sandbox")
        })
        .map(|catalog| catalog.id))
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
    db.collection::<Document>(SERIES).create_index(
        IndexModel::builder()
            .keys(doc! {"user_id":1,"conversation_id":1,"catalog_service_id":1,"source_path_sha256":1})
            .options(IndexOptions::builder().unique(true).build()).build()
    ).await?;
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
    for file in &result.files {
        let mut row = PendingArtifact {
            id: Uuid::new_v4().to_string(),
            watch_id: w.id.clone(),
            user_id: w.user_id.clone(),
            conversation_id: w.conversation_id.clone(),
            turn_id: w.turn_id.clone(),
            operation_id: operation.into(),
            source_result_identity: result.identity.clone(),
            artifact_index: file.index,
            filename: file.filename.clone(),
            source_path: file.source_path.clone(),
            source_path_sha256: Some(file.source_path_sha256.clone()),
            content_type: file.content_type.into(),
            size: file.bytes.len() as i64,
            sha256: file.sha256.clone(),
            preview_kind: file.preview.into(),
            source_status_version: status.version.clone(),
            content_revision: None,
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
        materialized_expires_at: None,
        files: rows
            .iter()
            .map(|row| crate::models::generated_artifact::AcceptedFile {
                index: row.artifact_index,
                pending_id: row.id.clone(),
                attachment_id: row.assistant_attachment_id.clone(),
                filename: row.filename.clone(),
                source_path_sha256: row.source_path_sha256.clone(),
                content_revision: None,
            })
            .collect(),
        rejected: result.rejected.clone(),
        source_result_identity: result.identity.clone(),
        source_status_version: status.version.clone(),
        content_revision: None,
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
    // The retry callback needs only this owned flag, not the decoded result borrow.
    let success = result.success;
    let series_ids: Vec<String> = rows.iter().map(|_| Uuid::new_v4().to_string()).collect();
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
            let mut committed_manifest = current.artifact_result.clone().unwrap_or_else(|| manifest.clone());
            if current.artifact_result.is_some() {
                let existing = pending_rows_in_session(&database, session, &current, &committed_manifest, false).await?;
                if existing.len() != committed_manifest.count { return Err(invalid()) }
            } else {
                // IDs/encrypted payloads are prepared once. Revision allocation
                // and publication share this retried, conversation-fenced transaction.
                let catalog = current.artifact_catalog_service_id.as_deref().ok_or_else(invalid)?;
                for (row, series_id) in rows.iter().zip(&series_ids) {
                    let mut row = row.clone();
                    if success {
                        row.content_revision = Some(allocate_revision(&database, session, &row, catalog, series_id).await?);
                    }
                    let descriptor = committed_manifest.files.iter_mut()
                        .find(|f| f.pending_id == row.id).ok_or_else(invalid)?;
                    descriptor.content_revision = row.content_revision.clone();
                    database.collection::<Document>(ARTIFACTS).insert_one(
                        bson::to_document(&row).map_err(|_|invalid())?).session(&mut *session).await?;
                }
            }
            database.collection::<Document>(WATCHES).update_one(doc!{"_id":&watch.id,"state":&watch.state,"lease_id":&watch.lease_id},doc!{"$set":{
                "artifact_result":bson::to_bson(&committed_manifest).map_err(|_|invalid())?,"result_encrypted":bson::to_bson(&Some(&encrypted)).map_err(|_|invalid())?,"secret_context_versions":bson::to_bson(&versions).map_err(|_|invalid())?,
                "state":"ready","reason":if success {"completed"}else{"operation_failed"},"lease_id":null,"lease_until":null,"poll_at":bson::DateTime::now()}}).session(&mut *session).await?;
            Ok(())
        }).await; transactions::transaction_result(operation)
    }).await.map_err(transactions::map_transaction_error)
}
async fn allocate_revision(
    db: &Database,
    session: &mut ClientSession,
    row: &PendingArtifact,
    catalog: &str,
    series_id: &str,
) -> AppResult<String> {
    let path = row.source_path_sha256.as_deref().ok_or_else(invalid)?;
    let collection = db.collection::<ContentSeries>(SERIES);
    let key = doc! {"user_id":&row.user_id,"conversation_id":&row.conversation_id,
    "catalog_service_id":catalog,"source_path_sha256":path};
    if let Some(head) = collection.find_one(key).session(&mut *session).await? {
        if head.revision < 1 {
            return Err(invalid());
        }
        if head.sha256 == row.sha256 {
            return Ok(head.revision.to_string());
        }
        let revision = head.revision.checked_add(1).ok_or_else(invalid)?;
        if db
            .collection::<Document>(SERIES)
            .update_one(
                doc! {"_id":&head.id,"revision":head.revision,"sha256":&head.sha256},
                doc! {"$set":{"revision":revision,"sha256":&row.sha256}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(invalid());
        }
        return Ok(revision.to_string());
    }
    collection
        .insert_one(ContentSeries {
            id: series_id.into(),
            user_id: row.user_id.clone(),
            conversation_id: row.conversation_id.clone(),
            catalog_service_id: catalog.into(),
            source_path_sha256: path.into(),
            sha256: row.sha256.clone(),
            revision: 1,
        })
        .session(&mut *session)
        .await?;
    Ok("1".into())
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
        content_revision: row.content_revision.clone(),
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
/// Fetch by saved IDs as well as watch: a present moved/tampered row is never
/// mistaken for absence. Legacy complete manifests retain their prior checks.
async fn pending_rows_in_session(
    db: &Database,
    session: &mut ClientSession,
    w: &AsyncServiceOperation,
    manifest: &ResultManifest,
    ciphertext: bool,
) -> AppResult<Vec<PendingArtifact>> {
    let ids: Vec<&str> = manifest
        .files
        .iter()
        .map(|f| f.pending_id.as_str())
        .collect();
    let collection = db.collection::<PendingArtifact>(ARTIFACTS);
    let mut find = collection
        .find(doc! {
            "$or":[{"watch_id":&w.id},{"_id":{"$in":ids}}],
        })
        .sort(doc! {"artifact_index":1})
        .limit(9);
    if !ciphertext {
        find = find.projection(doc! {"data_encrypted":0});
    }
    let mut cursor = find.session(&mut *session).await?;
    let rows: Vec<PendingArtifact> = cursor.stream(&mut *session).try_collect().await?;
    if manifest.count > 8
        || rows.len() > manifest.count
        || rows.iter().any(|r| !exact_pending(r, w, manifest))
    {
        return Err(invalid());
    }
    if !manifest.files.is_empty()
        && (manifest.files.len() != manifest.count
            || manifest.files.iter().enumerate().any(|(i, f)| {
                f.index >= 8
                    || manifest.files[..i].iter().any(|p| {
                        p.index == f.index
                            || p.pending_id == f.pending_id
                            || p.attachment_id == f.attachment_id
                    })
            })
            || rows.iter().any(|r| {
                !manifest.files.iter().any(|f| {
                    f.pending_id == r.id
                        && f.index == r.artifact_index
                        && f.attachment_id == r.assistant_attachment_id
                        && f.filename == r.filename
                        && (f.source_path_sha256.is_none()
                            || (f.source_path_sha256 == r.source_path_sha256
                                && f.content_revision == r.content_revision))
                })
            }))
    {
        return Err(invalid());
    }
    Ok(rows)
}
async fn mark_missing_in_session(
    db: &Database,
    session: &mut ClientSession,
    w: &AsyncServiceOperation,
    files: &[crate::models::generated_artifact::AcceptedFile],
) -> AppResult<()> {
    use crate::models::generated_artifact::{RejectedFile, RejectionReason};
    for file in files {
        let rejected = RejectedFile {
            index: file.index,
            filename: file.filename.clone(),
            reason: RejectionReason::Missing,
        };
        db.collection::<Document>(WATCHES).update_one(doc! {"_id":&w.id},
            doc! {"$addToSet":{"artifact_result.rejected":bson::to_bson(&rejected).map_err(|_|invalid())?}})
            .session(&mut *session).await?;
        db.collection::<Document>(ARTIFACTS).update_one(doc! {"_id":&file.pending_id},
            doc! {"$set":{"state":"missing","data_encrypted":null,"secret_context_versions":{}}})
            .session(&mut *session).await?;
    }
    Ok(())
}
/// Reconcile genuine absence before the engine clones reply attachments. Present
/// rows with contradictory authority remain errors, including rows moved away
/// from the ordinary watch query. Missing saved bytes are never recreated.
pub async fn prepare_settlement_in_session(
    db: &Database,
    row: &mut AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(id) = super::async_service_operation::event_id(row).map(str::to_owned) else {
        return Ok(());
    };
    let Some(w) = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {
            "_id":&id,"user_id":&row.user_id,"conversation_id":&row.id,"state":"queued",
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    if w.artifact_mode.is_none() {
        return Ok(());
    }
    let turn = row.active_turn.as_ref().ok_or_else(invalid)?;
    if !admitted(&w)
        || turn.origin != TurnOrigin::Event
        || w.artifact_event_turn_id.as_deref() != Some(turn.turn_id.as_str())
    {
        return Err(invalid());
    }
    let Some(manifest) = &w.artifact_result else {
        return Ok(());
    };
    let rows = pending_rows_in_session(db, session, &w, manifest, false).await?;
    let mut missing: Vec<_> = manifest
        .files
        .iter()
        .filter(|f| !rows.iter().any(|r| r.id == f.pending_id))
        .cloned()
        .collect();
    let mut remove = Vec::new();
    for attachment in turn
        .attachments
        .iter()
        .filter(|a| a.artifact.source_watch_id.as_deref() == Some(id.as_str()))
    {
        if !manifest.files.is_empty()
            && !manifest
                .files
                .iter()
                .any(|f| f.attachment_id == attachment.id)
        {
            return Err(invalid());
        }
        let pending = rows
            .iter()
            .find(|r| r.assistant_attachment_id == attachment.id);
        if pending.is_none() && manifest.files.is_empty() {
            // Legacy missing rows have no saved IDs. Check the attachment-index
            // lookup too, so a moved present row still fails closed.
            if db
                .collection::<Document>(ARTIFACTS)
                .find_one(doc! {"assistant_attachment_id":&attachment.id})
                .session(&mut *session)
                .await?
                .is_some()
            {
                return Err(invalid());
            }
        }
        if let Some(pending) = pending
            && (attachment.origin != "generated_artifact"
                || pending.state != "materialized"
                || attachment.content_type != pending.content_type
                || attachment.size != pending.size
                || attachment.artifact.content_revision != pending.content_revision
                || attachment.artifact.source_turn_id.as_deref() != Some(w.turn_id.as_str())
                || attachment.artifact.source_operation_id != w.operation_id
                || attachment.artifact.source_result_identity.as_deref()
                    != Some(manifest.source_result_identity.as_str()))
        {
            return Err(invalid());
        }
        let saved = db
            .collection::<Document>(ATTACHMENTS)
            .find_one(doc! {"_id":&attachment.id})
            .projection(doc! {"data_encrypted":0})
            .session(&mut *session)
            .await?;
        if let Some(saved) = &saved
            && ([
                ("user_id", w.user_id.as_str()),
                ("conversation_id", w.conversation_id.as_str()),
                ("turn_id", turn.turn_id.as_str()),
                ("origin", "generated_artifact"),
                ("source_watch_id", w.id.as_str()),
                ("source_turn_id", w.turn_id.as_str()),
                (
                    "source_operation_id",
                    w.operation_id.as_deref().ok_or_else(invalid)?,
                ),
                (
                    "source_result_identity",
                    manifest.source_result_identity.as_str(),
                ),
                ("content_type", attachment.content_type.as_str()),
            ]
            .iter()
            .any(|(key, value)| saved.get_str(key).ok() != Some(*value))
                || saved.get_i64("size").ok() != Some(attachment.size)
                || saved.get_str("content_revision").ok()
                    != attachment.artifact.content_revision.as_deref())
        {
            return Err(invalid());
        }
        let rejected_missing = pending.is_some_and(|p| {
            manifest.rejected.iter().any(|f| {
                f.index == p.artifact_index
                    && matches!(
                        f.reason,
                        crate::models::generated_artifact::RejectionReason::Missing
                    )
            })
        });
        if pending.is_none() || saved.is_none() || rejected_missing {
            remove.push(attachment.id.clone());
            if manifest.files.is_empty()
                && let Some(pending) = pending
            {
                missing.push(crate::models::generated_artifact::AcceptedFile {
                    index: pending.artifact_index,
                    pending_id: pending.id.clone(),
                    attachment_id: pending.assistant_attachment_id.clone(),
                    filename: pending.filename.clone(),
                    source_path_sha256: pending.source_path_sha256.clone(),
                    content_revision: pending.content_revision.clone(),
                });
            }
            if let Some(file) = manifest
                .files
                .iter()
                .find(|f| f.attachment_id == attachment.id)
                && !missing.iter().any(|f| f.pending_id == file.pending_id)
            {
                missing.push(file.clone());
            }
        }
    }
    mark_missing_in_session(db, session, &w, &missing).await?;
    if manifest.files.is_empty() && rows.len() < manifest.count {
        // Incomplete old manifests cannot invent missing IDs or recovery lineage.
        // Settle an explicit bounded failure instead of stranding the reply.
        db.collection::<Document>(WATCHES)
            .update_one(
                doc! {"_id":&w.id},
                doc! {"$set":{"artifact_result.delivery_reason":"delivery_missing"}},
            )
            .session(&mut *session)
            .await?;
    }
    row.active_turn
        .as_mut()
        .ok_or_else(invalid)?
        .attachments
        .retain(|a| !remove.contains(&a.id));
    Ok(())
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
    let mut expiry: Option<DateTime<Utc>> = None;
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
        let saved_expiry = saved
            .get_datetime("expires_at")
            .map_err(|_| invalid())?
            .to_chrono();
        expiry = Some(expiry.map_or(saved_expiry, |expiry| expiry.max(saved_expiry)));
    }
    Ok(expiry.unwrap_or(
        manifest
            .materialized_expires_at
            .unwrap_or(manifest.expires_at),
    ))
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
    if !matches!(
        w.reason.as_deref(),
        Some("completed" | "operation_failed" | "result_expired")
    ) {
        return Ok((Vec::new(), "delivery_failed"));
    }
    let Some(manifest) = &w.artifact_result else {
        return Ok((Vec::new(), "delivery_failed"));
    };
    let _reservation = byte_budget::reserve(sandbox::RESERVATION).await?;
    let rows = pending_rows_in_session(db, session, w, manifest, true).await?;
    if manifest.files.is_empty() && rows.len() != manifest.count {
        return Ok((Vec::new(), "delivery_failed"));
    }
    let missing: Vec<_> = manifest
        .files
        .iter()
        .filter(|f| !rows.iter().any(|r| r.id == f.pending_id))
        .cloned()
        .collect();
    mark_missing_in_session(db, session, w, &missing).await?;
    let additional = rows
        .iter()
        .filter(|r| {
            r.state != "missing"
                && !manifest.rejected.iter().any(|f| {
                    f.index == r.artifact_index
                        && matches!(
                            f.reason,
                            crate::models::generated_artifact::RejectionReason::Missing
                        )
                })
                && !existing.iter().any(|a| a.id == r.assistant_attachment_id)
        })
        .count();
    if existing.len() + additional > super::assistant_nyxagent::MAX_TURN_ATTACHMENTS {
        return Ok((Vec::new(), "delivery_capacity_exhausted"));
    }
    let policy = super::assistant_upload_retention::artifact_policy_in_session(db, session).await?;
    let mut attachments = Vec::new();
    let mut total = 0usize;
    let mut retention_expired = false;
    for row in rows {
        if row.state == "missing"
            || manifest.rejected.iter().any(|f| {
                f.index == row.artifact_index
                    && matches!(
                        f.reason,
                        crate::models::generated_artifact::RejectionReason::Missing
                    )
            })
        {
            continue;
        }
        if row.state != "materialized"
            && (row.expires_at <= Utc::now() || manifest.expires_at <= Utc::now())
        {
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
        // Pending acquisition is source-bounded. Once copied, these private
        // bytes use ordinary retention from their own attachment creation time.
        let expiry = Utc::now() + Duration::days(i64::from(normal_days));
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
            let saved = db
                .collection::<Document>(ATTACHMENTS)
                .find_one(doc! {"_id":&row.assistant_attachment_id})
                .projection(doc! {"data_encrypted":0})
                .session(&mut *session)
                .await?;
            if let Some(saved) = &saved
                && ([
                    ("user_id", w.user_id.as_str()),
                    ("conversation_id", w.conversation_id.as_str()),
                    ("turn_id", turn_id),
                    ("origin", "generated_artifact"),
                    ("source_watch_id", w.id.as_str()),
                    ("source_turn_id", w.turn_id.as_str()),
                    (
                        "source_operation_id",
                        w.operation_id.as_deref().ok_or_else(invalid)?,
                    ),
                    (
                        "source_result_identity",
                        manifest.source_result_identity.as_str(),
                    ),
                    ("content_type", row.content_type.as_str()),
                ]
                .iter()
                .any(|(key, value)| saved.get_str(key).ok() != Some(*value))
                    || saved.get_i64("size").ok() != Some(row.size)
                    || saved.get_str("content_revision").ok() != row.content_revision.as_deref())
            {
                return Err(invalid());
            }
            match super::assistant_upload_retention::generated_available_in_session(db, session, doc! {"_id":&row.assistant_attachment_id,"user_id":&w.user_id,"conversation_id":&w.conversation_id}, policy).await {
                Ok(()) => {},
                Err(AppError::AssistantAttachmentExpired) => {
                    retention_expired = true;
                    continue;
                },
                Err(error) => return Err(error),
            }
            meta.artifact =
                bson::from_document(saved.ok_or_else(missing)?).map_err(|_| invalid())?;
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
        db.collection::<Document>(ARTIFACTS).update_one(doc!{"_id":&row.id,"state":"pending"},doc!{"$set":{"state":"materialized","expires_at":bson::DateTime::from_chrono(expiry),"data_encrypted":null,"secret_context_versions":{}}}).session(&mut *session).await?;
        attachments.push(meta);
    }
    if let Some(expiry) = attachments
        .iter()
        .filter_map(|a| a.artifact.expires_at)
        .max()
    {
        // Keep exact watch authority alive through saved attachment retention.
        // The immutable manifest/source expiry and worker deadline do not move.
        db.collection::<Document>(WATCHES).update_one(doc! {"_id":&w.id},
            doc! {"$max":{"artifact_result.materialized_expires_at":bson::DateTime::from_chrono(expiry),
                "expires_at":bson::DateTime::from_chrono(expiry)}})
            .session(&mut *session).await?;
    }
    if attachments.is_empty() && retention_expired {
        return Ok((attachments, "retention_expired"));
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
        db.collection::<Document>(WATCHES)
            .update_one(
                doc! {"_id":&w.id,"state":"queued"},
                doc! {"$set":{"artifact_event_turn_id":&turn.turn_id}},
            )
            .session(&mut *session)
            .await?;
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
    let pending = pending_rows_in_session(db, session, &w, manifest, false).await?;
    if manifest.files.is_empty() && pending.len() != manifest.count {
        return Ok(()); // Explicit legacy failure snapshot, no invented recovery IDs.
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
        || (manifest.delivery_reason == "materialized"
            && ids.len()
                + manifest
                    .rejected
                    .iter()
                    .filter(|f| {
                        matches!(
                            f.reason,
                            crate::models::generated_artifact::RejectionReason::Missing
                        )
                    })
                    .count()
                != manifest.count)
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

/// Persist failure/rejection copy with the owner reply, independently of watch
/// TTL. This grants neither source access nor a verified recovery association.
pub async fn settled_outcome_in_session(
    db: &Database,
    row: &AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<Option<crate::models::generated_artifact::SavedOutcome>> {
    let Some(id) = super::async_service_operation::event_id(row) else {
        return Ok(None);
    };
    let Some(w) = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {
            "_id":id,"user_id":&row.user_id,"conversation_id":&row.id,
            "state":{"$in":["queued","delivered"]},
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    if w.artifact_mode.is_none() {
        return Ok(None);
    }
    let turn = row.active_turn.as_ref().ok_or_else(invalid)?;
    if !admitted(&w)
        || turn.origin != TurnOrigin::Event
        || w.artifact_event_turn_id.as_deref() != Some(turn.turn_id.as_str())
    {
        return Err(invalid());
    }
    let (state, rejected) = match w.artifact_result {
        Some(manifest) => (manifest.delivery_reason, manifest.rejected),
        None => (
            match w.reason.as_deref() {
                Some("artifact_too_large") => "artifact_too_large",
                Some("result_expired") => "result_expired",
                _ => "source_verification_failed",
            }
            .into(),
            Vec::new(),
        ),
    };
    Ok(Some(crate::models::generated_artifact::SavedOutcome {
        state,
        rejected,
        count: turn
            .attachments
            .iter()
            .filter(|a| a.artifact.source_watch_id.as_deref() == Some(id))
            .count(),
    }))
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
    pub rejected: Vec<crate::models::generated_artifact::RejectedFile>,
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
    let saved_messages: Vec<crate::models::assistant_message::AssistantMessage> = db
        .collection(crate::models::assistant_message::COLLECTION_NAME)
        .find(
            doc! {"_id":{"$in":messages},"user_id":user,"conversation_id":conversation,
            "role":"assistant","origin":"event"},
        )
        .await?
        .try_collect()
        .await?;
    let mut states: std::collections::HashMap<String, DeliveryState> = saved_messages
        .into_iter()
        .filter_map(|message| Some((message.id, message.turn_metadata?.artifact_delivery?)))
        .map(|(id, outcome)| {
            (
                id,
                DeliveryState {
                    state: outcome.state,
                    count: outcome.count,
                    rejected: outcome.rejected,
                    recovery_available: false,
                },
            )
        })
        .collect();
    for delivery in deliveries {
        let watch = watches.iter().find(|w| w.id == delivery.watch_id);
        let state = if (delivery.attachment_ids.is_empty() && delivery.expires_at <= Utc::now())
            || (!delivery.attachment_ids.is_empty()
                && delivery
                    .attachment_ids
                    .iter()
                    .all(|id| expired.contains(id)))
        {
            if delivery.attachment_ids.is_empty() {
                "result_expired"
            } else {
                "retention_expired"
            }
        } else {
            watches
                .iter()
                .find(|w| w.id == delivery.watch_id)
                .and_then(|w| {
                    if matches!(w.state.as_str(), "cancelling" | "cancelled") {
                        Some("cancelled")
                    } else if w.reason.as_deref() == Some("result_expired")
                        && delivery.attachment_ids.is_empty()
                        && w.artifact_result
                            .as_ref()
                            .is_none_or(|m| m.materialized_expires_at.is_none())
                    {
                        Some("result_expired")
                    } else {
                        w.artifact_result
                            .as_ref()
                            .map(|m| m.delivery_reason.as_str())
                    }
                })
                .unwrap_or(if delivery.attachment_ids.is_empty() {
                    "delivery_failed"
                } else {
                    "materialized"
                })
        };
        let rejected = watch
            .and_then(|w| w.artifact_result.as_ref())
            .map(|m| m.rejected.clone())
            .or_else(|| states.get(&delivery.message_id).map(|s| s.rejected.clone()))
            .unwrap_or_default();
        states.insert(
            delivery.message_id,
            DeliveryState {
                state: state.into(),
                count: delivery.attachment_ids.len(),
                rejected,
                recovery_available: matches!(
                    state,
                    "delivery_pending" | "delivery_failed" | "delivery_capacity_exhausted"
                ),
            },
        );
    }
    Ok(states)
}
