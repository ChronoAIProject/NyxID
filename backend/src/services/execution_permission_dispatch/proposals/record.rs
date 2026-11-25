//! Retained approval fields use the selected version arm for their record context.
//! Historical assistant arms share the v1 AAD. External requests use the v2 AAD.
use super::{AssistantVersion, ExecutionProposal, REQUESTS, conflict};
use crate::{
    crypto::{aes::EncryptionKeys, record_field::RecordContext},
    errors::AppResult,
    models::{
        approval_request::ApprovalRequest,
        execution_proposal::{Proposal, ProposalV2},
    },
};
use bson::{Bson, Document};
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
            Self::Assistant(AssistantVersion::V4) => "v4",
            Self::V2 => "v2",
        }
    }

    fn aad(self, id: &str, field: Field) -> String {
        let version = match self {
            Self::Assistant(_) => "v1",
            Self::V2 => "v2",
        };
        match field {
            Field::Payload => format!("execution-proposal:{version}:{id}"),
            Field::Result => format!("execution-proposal:{version}:{id}:result"),
        }
    }

    pub(super) fn unset(self, fields: &[Field]) -> Document {
        let mut unset = Document::new();
        for field in fields {
            let path = format!("execution_proposal.{}.{}", self.name(), field.name());
            unset.insert(format!("{path}_context_version"), "");
            unset.insert(path, "");
        }
        unset
    }

    pub(super) async fn seal(
        self,
        keys: &EncryptionKeys,
        id: &str,
        field: Field,
        plain: &[u8],
    ) -> AppResult<Vec<u8>> {
        keys.encrypt_with_aad(plain, self.aad(id, field).as_bytes())
            .await
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
        match marker {
            Some(3) => {
                let path = ["execution_proposal", slot.name(), self.name()];
                let context = RecordContext::new(REQUESTS, &row.id, &row.user_id, &path)?;
                keys.decrypt_record(&value.bytes, &context).await.map(Some)
            }
            None if keys.allow_legacy_record_reads() => keys
                .decrypt_with_aad(&value.bytes, slot.aad(&row.id, self).as_bytes())
                .await
                .map(Some),
            _ => Err(conflict()),
        }
    }
}

/// Pending rows with malformed arms must still produce a blocked finding when
/// a dotted-field lookup alone would classify all eight paths as empty.
pub(crate) fn no_retained_material(row: &Document) -> bool {
    let envelope = match row.get("execution_proposal") {
        None | Some(Bson::Null) => return true,
        Some(Bson::Document(e)) => e,
        _ => return false,
    };
    if envelope.keys().any(|key| {
        key.strip_prefix('v')
            .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            && !matches!(key.as_str(), "v1" | "v2" | "v3" | "v4")
    }) {
        return false;
    }
    let Ok(parsed) = bson::from_document(envelope.clone()) else {
        return false;
    };
    let Ok(arm) = Arm::select(&parsed) else {
        return false;
    };
    [Field::Payload, Field::Result].into_iter().all(|f| {
        let (_, value, marker) = arm.field(f);
        value.is_none() && marker.is_none()
    })
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
