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

#[derive(Clone, Copy)]
pub(super) enum Field<'a> {
    Body,
    Scope,
    Interface(&'a str),
}

impl<'a> Field<'a> {
    fn path(self, agent: &'a str) -> Vec<&'a str> {
        match self {
            Self::Body => vec!["body_encrypted", agent],
            Self::Scope => vec!["scope_encrypted", agent],
            Self::Interface(operation) => {
                vec!["publication", "interface_encrypted", agent, operation]
            }
        }
    }

    pub(super) async fn seal(
        self,
        keys: &EncryptionKeys,
        id: &str,
        owner: &str,
        agent: &str,
        bytes: &[u8],
    ) -> AppResult<(Vec<u8>, Option<u8>)> {
        let path = self.path(agent);
        let context = RecordContext::new(PROPOSALS_COLLECTION_NAME, id, owner, &path)?;
        Ok((
            record_field::encrypt_context(keys, &context, bytes).await?,
            keys.contextual_record_writes().then_some(3),
        ))
    }
}
async fn open(
    keys: &EncryptionKeys,
    row: &Proposal,
    field: Field<'_>,
    version: Option<u8>,
    bytes: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let path = field.path(&row.agent_id);
    let context = RecordContext::new(PROPOSALS_COLLECTION_NAME, &row.id, &row.owner_id, &path)?;
    record_field::decrypt_context(keys, &context, version, bytes).await
}

pub(super) async fn body(keys: &EncryptionKeys, row: &Proposal) -> AppResult<Zeroizing<Vec<u8>>> {
    open(
        keys,
        row,
        Field::Body,
        row.body_context_version,
        &row.body_encrypted,
    )
    .await
}

async fn optional(
    keys: &EncryptionKeys,
    row: &Proposal,
    field: Field<'_>,
    version: Option<u8>,
    bytes: Option<&[u8]>,
) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
    match bytes {
        Some(bytes) => open(keys, row, field, version, bytes).await.map(Some),
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
        Field::Scope,
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
        Field::Interface(&p.operation_id),
        p.interface_context_version,
        p.interface_encrypted.as_deref(),
    )
    .await
}

#[path = "assistant_learning_record_storage.rs"]
pub(super) mod storage;
