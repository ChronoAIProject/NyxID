//! A storage pass authenticates the complete proposal before the single update.
//! The original BSON also fences the proposal revision, publication and lease.
use super::{Field, PROPOSALS_COLLECTION_NAME, Proposal};
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    models::assistant_agent_learning::DraftScope,
    services::{
        assistant_agent_learning_review,
        assistant_learning_publication::{self as publication, InterfaceSnapshot},
        record_storage, transaction_service,
    },
};
use bson::{Binary, Document, doc, spec::BinarySubtype};
use mongodb::Database;

fn invalid() -> AppError {
    AppError::Conflict("Learning proposal storage could not be verified".into())
}

async fn inspect(keys: &EncryptionKeys, source: &Document, rewrap: bool) -> AppResult<Document> {
    let row: Proposal = bson::from_document(source.clone()).map_err(|_| invalid())?;
    let mut fields = vec![
        (
            Field::Body,
            "body_encrypted",
            "body_context_version",
            row.body_context_version,
            Some(row.body_encrypted.as_slice()),
        ),
        (
            Field::Scope,
            "scope_encrypted",
            "scope_context_version",
            row.scope_context_version,
            row.scope_encrypted.as_deref(),
        ),
    ];
    if let Some(p) = &row.publication {
        fields.push((
            Field::Interface(&p.operation_id),
            "publication.interface_encrypted",
            "publication.interface_context_version",
            p.interface_context_version,
            p.interface_encrypted.as_deref(),
        ));
    }
    let mut set = Document::new();
    let mut draft = None;
    let mut snapshot: Option<InterfaceSnapshot> = None;
    for (field, name, marker, version, bytes) in fields {
        let path = field.path(&row.agent_id);
        let context = RecordContext::new(PROPOSALS_COLLECTION_NAME, &row.id, &row.owner_id, &path)?;
        let Some(bytes) = bytes else {
            if version.is_some() || matches!(field, Field::Scope) && row.scope_digest.is_some() {
                return Err(invalid());
            }
            continue;
        };
        if matches!(field, Field::Body) && bytes.is_empty() {
            if version.is_some()
                || row.body_bytes != 0
                || !matches!(row.status.as_str(), "pinned" | "invalidated" | "rejected")
            {
                return Err(invalid());
            }
            continue;
        }
        let current = version == Some(3)
            && keys
                .authenticate_current_record(bytes, &context)
                .await
                .is_ok();
        if !current && !rewrap {
            return Err(invalid());
        }
        let plain = record_field::decrypt_context(keys, &context, version, bytes).await?;
        match field {
            Field::Body => {
                if row.body_bytes != plain.len() as i64 {
                    return Err(invalid());
                }
                draft = Some(
                    assistant_agent_learning_review::decode_draft(&row, &plain)
                        .map_err(|_| invalid())?,
                );
            }
            Field::Scope => {
                let _: DraftScope = serde_json::from_slice(&plain).map_err(|_| invalid())?;
                if row.scope_digest.as_deref() != Some(&publication::hash(&plain)) {
                    return Err(invalid());
                }
            }
            Field::Interface(_) => {
                snapshot = Some(serde_json::from_slice(&plain).map_err(|_| invalid())?);
            }
        }
        if !current {
            let bytes = keys.encrypt_record(&plain, &context).await?;
            keys.authenticate_current_record(&bytes, &context).await?;
            set.insert(
                name,
                Binary {
                    subtype: BinarySubtype::Generic,
                    bytes,
                },
            );
            set.insert(marker, 3_i32);
        }
    }
    if let Some(p) = &row.publication
        && p.package_format == assistant_agent_learning_review::PACKAGE_FORMAT_SNAPSHOT
    {
        if p.target_kind.as_deref() == Some("update") && snapshot.is_none() {
            return Err(invalid());
        }
        if let Some(body) = draft {
            let package = publication::package_with_snapshot(
                &body,
                &p.operation_id,
                &p.name,
                &p.version,
                snapshot.as_ref(),
            )
            .map_err(|_| invalid())?;
            if publication::hash(&package) != p.sha256 {
                return Err(invalid());
            }
        }
    }
    Ok(set)
}

pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    inspect(keys, source, false).await.map(|_| ())
}

pub(crate) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let set = inspect(keys, source, true).await?;
    if set.is_empty() {
        return Ok(false);
    }
    Ok(transaction_service::durable_database(db)
        .collection::<Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set": set},
        )
        .await?
        .matched_count
        == 1)
}
