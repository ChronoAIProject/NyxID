use super::*;
use crate::crypto::record_field::historical::{HistoricalAad, ProposalDomain};
use crate::models::{
    assistant_agent_learning::AssistantAgentLearningProposal,
    assistant_conversation::ReattachWindow,
};
use crate::services::{
    assistant_learning_record::{Field, storage::Validation},
    assistant_nyxagent_runtime::window_record,
    execution_permission_dispatch::proposals::record,
    record_field_service::stored_bytes,
};

pub(super) fn source_companions(entry: &Entry) -> Option<&'static [&'static str]> {
    Some(match entry.adapter {
        Adapter::ApprovalProposal => &["user_id", "execution_proposal"],
        Adapter::ReattachWindow => &[
            "user_id",
            "credential_api_key_id",
            "active_turn.restart_admission",
        ],
        Adapter::LearningProposal => &[
            "owner_id",
            "agent_id",
            "status",
            "source",
            "body_bytes",
            "body_context_version",
            "scope_context_version",
            "scope_digest",
            "publication",
        ],
        _ => return None,
    })
}

fn child<'a>(row: &'a Document, field: &str) -> AppResult<Option<&'a Document>> {
    present(row, field)
        .map(|v| v.as_document().ok_or_else(invalid))
        .transpose()
}

fn proposal_plain(slot: &str, field: &str, plain: &[u8]) -> AppResult<()> {
    use crate::models::execution_proposal::{ExecutionResult, PreparedEffect, RetainedEffect};
    if plain.len() > crate::services::execution_permission_dispatch::proposals::MAX_PAYLOAD {
        return Err(invalid());
    }
    match (slot, field) {
        ("v2", "result") => serde_json::from_slice::<ExecutionResult>(plain).map(drop),
        ("v2", "payload") => serde_json::from_slice::<RetainedEffect>(plain).map(drop),
        (_, "payload") => serde_json::from_slice::<PreparedEffect>(plain).map(drop),
        // The assistant result reader accepts bounded text using from_utf8_lossy.
        (_, "result") => return Ok(()),
        _ => return Err(invalid()),
    }
    .map_err(|_| invalid())
}

impl Source<'_> {
    pub(super) async fn approval(&self, out: &mut Observation) -> AppResult<()> {
        let fields = self.family.entry.fields;
        let envelope = child(self.row, "execution_proposal")?;
        if let Some(e) = envelope {
            out.check(fields[0], record::checked_envelope(e).map(drop));
        }
        for &label in fields {
            let result = async {
                let path: Vec<_> = label.split('.').collect();
                let [_, slot, field] = path.as_slice() else {
                    return Err(invalid());
                };
                let arm = envelope.map(|e| child(e, slot)).transpose()?.flatten();
                let marker =
                    version(arm.and_then(|a| present(a, &format!("{field}_context_version"))))?;
                let Some(raw) = arm.and_then(|a| present(a, field)) else {
                    if marker.is_some() {
                        return Err(invalid());
                    }
                    out.instances.push((label, Storage::Absent));
                    return Ok(());
                };
                let Bson::Binary(bytes) = raw else {
                    return Err(invalid());
                };
                let owner = text(self.row, "user_id")?;
                if let Some(v) = marker {
                    self.record(
                        out,
                        label,
                        owner,
                        &path,
                        Marker::Version(v),
                        &bytes.bytes,
                        |plain| proposal_plain(slot, field, plain),
                    )
                    .await?;
                } else {
                    let domain = if *slot == "v2" {
                        ProposalDomain::External
                    } else {
                        ProposalDomain::Assistant
                    };
                    let aad = if *field == "payload" {
                        HistoricalAad::proposal_payload(domain, self.id)
                    } else {
                        HistoricalAad::proposal_result(domain, self.id)
                    };
                    out.open(
                        self.keys,
                        label,
                        Marker::Unmarked,
                        Binding::Historical(&aad),
                        &bytes.bytes,
                        |plain| proposal_plain(slot, field, plain),
                    )
                    .await;
                }
                Ok(())
            }
            .await;
            out.check(label, result);
        }
        Ok(())
    }

    pub(super) async fn window(&self, out: &mut Observation) -> AppResult<()> {
        let label = self.family.entry.fields[0];
        let admission = child(self.row, "active_turn")?
            .map(|a| child(a, "restart_admission"))
            .transpose()?
            .flatten();
        let raw = admission.map(|a| child(a, "window")).transpose()?.flatten();
        let Some(raw) = raw else {
            out.instances.push((label, Storage::Absent));
            return Ok(());
        };
        let window: ReattachWindow = bson::from_document(raw.clone()).map_err(|_| invalid())?;
        let Some(bytes) = window.input else {
            if window.input_context_version.is_some() {
                return Err(invalid());
            }
            out.instances.push((label, Storage::Absent));
            return Ok(());
        };
        let admission = admission.ok_or_else(invalid)?;
        let credential = text(admission, "credential_id")?;
        if text(self.row, "credential_api_key_id")? != credential
            && admission.get_bool("credential_ended") != Ok(true)
        {
            out.findings.push((label, Finding::Malformed));
        }
        let owner = text(self.row, "user_id")?;
        match window.input_context_version {
            None => {
                let aad = HistoricalAad::reattach_window(owner, self.id, credential, &window.key)
                    .map_err(|_| invalid())?;
                out.open(
                    self.keys,
                    label,
                    Marker::Unmarked,
                    Binding::Historical(&aad),
                    &bytes.bytes,
                    window_record::validate_plain,
                )
                .await;
                Ok(())
            }
            Some(v) => {
                self.record(
                    out,
                    label,
                    owner,
                    &[
                        "active_turn",
                        "restart_admission",
                        "window",
                        "input",
                        credential,
                        &window.key,
                    ],
                    Marker::Version(v.into()),
                    &bytes.bytes,
                    window_record::validate_plain,
                )
                .await
            }
        }
    }

    pub(super) async fn learning(&self, out: &mut Observation) -> AppResult<()> {
        // Decode metadata independently of each retained slot. Invalid bytes
        // in a sibling still leave the other authenticated contributions.
        let mut metadata = self.row.clone();
        for k in [
            "body_encrypted",
            "scope_encrypted",
            "body_context_version",
            "scope_context_version",
        ] {
            metadata.remove(k);
        }
        if let Ok(p) = metadata.get_document_mut("publication") {
            p.remove("interface_encrypted");
            p.remove("interface_context_version");
        }
        let row: AssistantAgentLearningProposal =
            bson::from_document(metadata).map_err(|_| invalid())?;
        let mut validation = Validation::default();
        for &label in self.family.entry.fields {
            let result = async {
                let (field, storage, name, version_name) = match label {
                    "scope_encrypted" => (Field::Scope, self.row, label, "scope_context_version"),
                    "body_encrypted" => (Field::Body, self.row, label, "body_context_version"),
                    _ => {
                        let Some(p) = child(self.row, "publication")? else {
                            out.instances.push((label, Storage::Absent));
                            return Ok(());
                        };
                        (
                            Field::Interface(text(p, "operation_id")?),
                            p,
                            "interface_encrypted",
                            "interface_context_version",
                        )
                    }
                };
                let marker = version(present(storage, version_name))?;
                let raw = if matches!(field, Field::Body) {
                    storage.get(name)
                } else {
                    present(storage, name)
                };
                let bytes = raw.map(stored_bytes).transpose()?;
                if matches!(field, Field::Body) && bytes.as_ref().is_none_or(Vec::is_empty) {
                    if marker.is_some()
                        || row.body_bytes != 0
                        || !matches!(row.status.as_str(), "rejected" | "invalidated" | "pinned")
                    {
                        return Err(invalid());
                    }
                    out.instances.push((label, Storage::Sentinel));
                    return Ok(());
                }
                let Some(bytes) = bytes else {
                    if marker.is_some()
                        || matches!(field, Field::Scope) && row.scope_digest.is_some()
                    {
                        return Err(invalid());
                    }
                    out.instances.push((label, Storage::Absent));
                    return Ok(());
                };
                self.record(
                    out,
                    label,
                    &row.owner_id,
                    &field.path(&row.agent_id),
                    marker.map_or(Marker::Unmarked, Marker::Version),
                    &bytes,
                    |p| validation.field(&row, field, p),
                )
                .await
            }
            .await;
            out.check(label, result);
        }
        if validation.package(&row).is_err() {
            out.findings
                .push(("publication.interface_encrypted", Finding::InvalidValue));
        }
        Ok(())
    }
}
