//! Retained approval fields use the selected version arm for their record context.
//! Historical assistant arms share the v1 AAD. External requests use the v2 AAD.
use super::{AssistantVersion, ExecutionProposal, REQUESTS, conflict};
use crate::crypto::record_field::historical::{HistoricalAad, ProposalDomain};
use crate::{
    crypto::{aes::EncryptionKeys, record_field::RecordContext},
    errors::AppResult,
    models::{
        approval_request::ApprovalRequest,
        execution_proposal::{Proposal, ProposalV2},
    },
};
use bson::{Bson, Document, doc};
use mongodb::Database;
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
pub(super) enum Slot {
    Assistant(AssistantVersion),
    V2,
}

impl Slot {
    fn name(self) -> &'static str {
        match self {
            Self::Assistant(AssistantVersion::V1) => "v1",
            Self::Assistant(AssistantVersion::V3) => "v3",
            Self::Assistant(AssistantVersion::V4 | AssistantVersion::V5) => "v4",
            Self::V2 => "v2",
        }
    }

    fn aad(self, id: &str, field: Field) -> HistoricalAad<'_> {
        let domain = match self {
            Self::Assistant(_) => ProposalDomain::Assistant,
            Self::V2 => ProposalDomain::External,
        };
        match field {
            Field::Payload => HistoricalAad::proposal_payload(domain, id),
            Field::Result => HistoricalAad::proposal_result(domain, id),
        }
    }

    fn path(self, field: Field) -> [&'static str; 3] {
        ["execution_proposal", self.name(), field.name()]
    }

    pub(super) fn unset(self, fields: &[Field]) -> Document {
        let mut unset = Document::new();
        for field in fields {
            let path = self.path(*field).join(".");
            unset.insert(format!("{path}_context_version"), "");
            unset.insert(path, "");
        }
        unset
    }

    pub(super) async fn seal(
        self,
        keys: &EncryptionKeys,
        id: &str,
        owner: &str,
        field: Field,
        plain: &[u8],
    ) -> AppResult<(Vec<u8>, Option<u8>)> {
        if keys.contextual_record_writes() {
            let path = self.path(field);
            let ctx = RecordContext::new(REQUESTS, id, owner, &path)?;
            Ok((keys.encrypt_record(plain, &ctx).await?, Some(3)))
        } else {
            Ok((self.aad(id, field).seal(keys, plain).await?, None))
        }
    }
    async fn open_value(
        self,
        keys: &EncryptionKeys,
        id: &str,
        owner: &str,
        field: Field,
        value: &bson::Binary,
        marker: Option<u8>,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        match marker {
            Some(3) => {
                let path = self.path(field);
                let context = RecordContext::new(REQUESTS, id, owner, &path)?;
                keys.decrypt_record(&value.bytes, &context).await
            }
            None if keys.allow_legacy_record_reads() => {
                self.aad(id, field).open(keys, &value.bytes).await
            }
            _ => Err(conflict()),
        }
    }
}

pub(super) enum Arm<'a> {
    Assistant(AssistantVersion, &'a Proposal),
    V2(&'a ProposalV2),
}

impl<'a> Arm<'a> {
    fn select(e: &'a ExecutionProposal) -> AppResult<Self> {
        match (e.version, &e.v1, &e.v2, &e.v3, &e.v4) {
            (1, Some(p), None, None, None) => Ok(Self::Assistant(AssistantVersion::V1, p)),
            (2, None, Some(p), None, None) => Ok(Self::V2(p)),
            (3, None, None, Some(p), None) => Ok(Self::Assistant(AssistantVersion::V3, p)),
            (4, None, None, None, Some(p)) => Ok(Self::Assistant(AssistantVersion::V4, p)),
            (5, None, None, None, Some(p)) => Ok(Self::Assistant(AssistantVersion::V5, p)),
            _ => Err(conflict()),
        }
    }

    pub(super) fn of(row: &'a ApprovalRequest) -> AppResult<Self> {
        Self::select(row.execution_proposal.as_ref().ok_or_else(conflict)?)
    }

    fn field(&self, field: Field) -> (Slot, Option<&bson::Binary>, Option<u8>) {
        match (self, field) {
            (Self::Assistant(v, p), Field::Payload) => (
                Slot::Assistant(*v),
                p.payload.as_ref(),
                p.payload_context_version,
            ),
            (Self::Assistant(v, p), Field::Result) => (
                Slot::Assistant(*v),
                p.result.as_ref(),
                p.result_context_version,
            ),
            (Self::V2(p), Field::Payload) => {
                (Slot::V2, p.payload.as_ref(), p.payload_context_version)
            }
            (Self::V2(p), Field::Result) => (Slot::V2, p.result.as_ref(), p.result_context_version),
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::services::execution_permission_dispatch) enum Field {
    Payload,
    Result,
}

impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Payload => "payload",
            Self::Result => "result",
        }
    }

    pub(in crate::services::execution_permission_dispatch) async fn open(
        self,
        keys: &EncryptionKeys,
        row: &ApprovalRequest,
    ) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
        let arm = Arm::of(row)?;
        let (slot, value, marker) = arm.field(self);
        let Some(value) = value else {
            return if marker.is_none() {
                Ok(None)
            } else {
                Err(conflict())
            };
        };
        slot.open_value(keys, &row.id, &row.user_id, self, value, marker)
            .await
            .map(Some)
    }
}

/// Inspect the complete selected arm before publishing either field. A non-proposal
/// approval and a selected arm cleared by retention have no material to rewrap.
async fn inspect(keys: &EncryptionKeys, row: &Document, rewrap: bool) -> AppResult<Document> {
    let mut set = Document::new();
    let envelope = match row.get("execution_proposal") {
        None | Some(Bson::Null) => return Ok(set),
        Some(Bson::Document(e)) => e,
        _ => return Err(conflict()),
    };
    if envelope.keys().any(|key| {
        key.strip_prefix('v')
            .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            && !matches!(key.as_str(), "v1" | "v2" | "v3" | "v4")
    }) {
        return Err(conflict());
    }
    let parsed = bson::from_document(envelope.clone()).map_err(|_| conflict())?;
    let arm = Arm::select(&parsed)?;
    for field in [Field::Payload, Field::Result] {
        let (slot, value, marker) = arm.field(field);
        let Some(value) = value else {
            if marker.is_some() {
                return Err(conflict());
            }
            continue;
        };
        let id = row.get_str("_id").map_err(|_| conflict())?;
        let owner = row.get_str("user_id").map_err(|_| conflict())?;
        let path = slot.path(field);
        let context = RecordContext::new(REQUESTS, id, owner, &path)?;
        if marker == Some(3)
            && keys
                .authenticate_current_record(&value.bytes, &context)
                .await
                .is_ok()
        {
            continue;
        }
        if !rewrap {
            return Err(conflict());
        }
        let plain = slot
            .open_value(keys, id, owner, field, value, marker)
            .await?;
        let bytes = keys.encrypt_record(&plain, &context).await?;
        keys.authenticate_current_record(&bytes, &context).await?;
        let name = path.join(".");
        set.insert(format!("{name}_context_version"), 3_i32);
        set.insert(name, super::binary(bytes));
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
        return Err(conflict());
    }
    let set = inspect(keys, source, true).await?;
    if set.is_empty() {
        return Ok(false);
    }
    let db = crate::services::transaction_service::durable_database(db);
    Ok(db
        .collection::<Document>(REQUESTS)
        .update_one(
            crate::services::record_storage::exact_document_predicate(source),
            doc! {"$set": set},
        )
        .await?
        .matched_count
        == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_aad_vectors_cover_each_selected_slot_and_purpose() {
        let request = "req:λ/%2F";
        for (slot, name, payload, result) in [
            (
                Slot::Assistant(AssistantVersion::V1),
                "v1",
                "execution-proposal:v1:req:λ/%2F",
                "execution-proposal:v1:req:λ/%2F:result",
            ),
            (
                Slot::V2,
                "v2",
                "execution-proposal:v2:req:λ/%2F",
                "execution-proposal:v2:req:λ/%2F:result",
            ),
            (
                Slot::Assistant(AssistantVersion::V3),
                "v3",
                "execution-proposal:v1:req:λ/%2F",
                "execution-proposal:v1:req:λ/%2F:result",
            ),
            (
                Slot::Assistant(AssistantVersion::V4),
                "v4",
                "execution-proposal:v1:req:λ/%2F",
                "execution-proposal:v1:req:λ/%2F:result",
            ),
            (
                Slot::Assistant(AssistantVersion::V5),
                "v4",
                "execution-proposal:v1:req:λ/%2F",
                "execution-proposal:v1:req:λ/%2F:result",
            ),
        ] {
            assert_eq!(slot.name(), name);
            assert_eq!(
                slot.aad(request, Field::Payload).as_bytes(),
                payload.as_bytes()
            );
            assert_eq!(
                slot.aad(request, Field::Result).as_bytes(),
                result.as_bytes()
            );
        }
    }
}
