//! Proposal fields use the stored proposal, owner and agent identities. An
//! interface snapshot belongs to the publication operation that captured it.
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    models::assistant_agent_learning::{
        AssistantAgentLearningProposal as Proposal, LearningPublication, PROPOSALS_COLLECTION_NAME,
    },
};
use zeroize::Zeroizing;

async fn open(
    keys: &EncryptionKeys,
    row: &Proposal,
    path: &[&str],
    version: Option<u8>,
    bytes: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let context = RecordContext::new(PROPOSALS_COLLECTION_NAME, &row.id, &row.owner_id, path)?;
    record_field::decrypt_context(keys, &context, version, bytes).await
}

pub(super) async fn body(keys: &EncryptionKeys, row: &Proposal) -> AppResult<Zeroizing<Vec<u8>>> {
    open(
        keys,
        row,
        &["body_encrypted", &row.agent_id],
        row.body_context_version,
        &row.body_encrypted,
    )
    .await
}

async fn optional(
    keys: &EncryptionKeys,
    row: &Proposal,
    path: &[&str],
    version: Option<u8>,
    bytes: Option<&[u8]>,
) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
    match bytes {
        Some(bytes) => open(keys, row, path, version, bytes).await.map(Some),
        None if version.is_none() => Ok(None),
        None => Err(AppError::Internal(
            "Missing encrypted proposal field".into(),
        )),
    }
}

pub(super) async fn scope(
    keys: &EncryptionKeys,
    row: &Proposal,
) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
    optional(
        keys,
        row,
        &["scope_encrypted", &row.agent_id],
        row.scope_context_version,
        row.scope_encrypted.as_deref(),
    )
    .await
}

pub(super) async fn interface(
    keys: &EncryptionKeys,
    row: &Proposal,
    p: &LearningPublication,
) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
    optional(
        keys,
        row,
        &[
            "publication",
            "interface_encrypted",
            &row.agent_id,
            &p.operation_id,
        ],
        p.interface_context_version,
        p.interface_encrypted.as_deref(),
    )
    .await
}
