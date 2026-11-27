use super::companions::Parents;
use super::*;
use crate::crypto::{record_field::historical::HistoricalAad, token::hash_token};
use crate::models::{
    assistant_agent_credential::AssistantAgentCredential, generated_artifact::PendingArtifact,
};
use crate::services::{
    assistant_cma_request_record as cma, assistant_cma_sandbox_service::prepare_creation_request,
    assistant_record_service as assistant, assistant_tool_result_service as tool,
    async_service_operation as operations, generated_artifacts as artifacts,
    personal_erasure_service::provider, platform_public_record_service as public,
    record_field_service::stored_bytes,
};

pub(super) fn source_companions(entry: &Entry) -> Option<&'static [&'static str]> {
    Some(match entry.adapter {
        Adapter::AssistantCredential => &[
            "user_id",
            "api_key_id",
            "conversation_id",
            "secret_context_versions",
        ],
        Adapter::Attachment => &[
            "user_id",
            "origin",
            "turn_id",
            "conversation_id",
            "group_id",
            "content_type",
            "size",
            "chunks",
            "pages",
            "parent_attachment_id",
            "chunk_index",
            "page",
            "render_version",
            "source_turn_id",
            "source_watch_id",
            "source_operation_id",
            "source_result_identity",
            "secret_context_versions",
        ],
        Adapter::GeneratedArtifact => &[
            "user_id",
            "conversation_id",
            "turn_id",
            "watch_id",
            "operation_id",
            "source_result_identity",
            "size",
            "sha256",
            "state",
            "secret_context_versions",
        ],
        Adapter::ToolResult => &[
            "user_id",
            "actor_id",
            "conversation_id",
            "turn_id",
            "api_key_id",
            "agent_id",
            "guest",
            "version",
            "size",
            "boundaries",
            "encryption_version",
        ],
        Adapter::Public if entry.collection == "platform_public_sessions" => &[
            "sponsor_owner_user_id",
            "credential_api_key_id",
            "secret_context_versions",
            "upstream_may_exist",
            "revoked_at",
        ],
        Adapter::Public => &[
            "sponsor_owner_user_id",
            "public_session_id",
            "conversation_id",
            "receipt_id",
            "claim_id",
            "reply_id",
            "secret_context_versions",
        ],
        Adapter::Capability => &["user_id", "context_version", "status"],
        Adapter::CmaRequest => &[
            "user_id",
            "request_context_version",
            "request_hash",
            "submission_state",
            "admission_state",
            "occupies_account_slot",
            "source",
            "idempotency_key",
            "operation_id",
        ],
        Adapter::AsyncResult => &["user_id", "state", "secret_context_versions"],
        _ => return None,
    })
}

fn marked(v: Option<u8>) -> Marker {
    v.map_or(Marker::Unmarked, |v| Marker::Version(v.into()))
}

fn require(value: bool) -> AppResult<()> {
    value.then_some(()).ok_or_else(invalid)
}

fn issued_key(parent: Option<&Document>, owner: &str, plain: &[u8]) -> AppResult<()> {
    let secret = std::str::from_utf8(plain).map_err(|_| invalid())?;
    let key = parent.ok_or_else(invalid)?;
    require(
        !secret.is_empty()
            && key.get_str("user_id") == Ok(owner)
            && key.get_str("key_hash") == Ok(hash_token(secret).as_str()),
    )
}

impl Source<'_> {
    pub(super) async fn retained(&self, out: &mut Observation, parents: &Parents) -> AppResult<()> {
        let field = self.family.entry.fields[0];
        match self.family.entry.adapter {
            Adapter::AssistantCredential => {
                let row: AssistantAgentCredential =
                    bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                require(row.secret_context_versions.keys().all(|k| k == field))?;
                let parent = parents.get("api_keys");
                if parent.is_none() {
                    out.findings.push((field, Finding::MissingParent));
                }
                self.record(
                    out,
                    field,
                    &row.user_id,
                    &assistant::credential_path(&row),
                    marked(row.secret_context_versions.get(field).copied()),
                    &row.key_ciphertext,
                    |p| issued_key(parent, &row.user_id, p),
                )
                .await
            }
            Adapter::Attachment => {
                let root = parents.get("assistant_attachments");
                let identity = match assistant::attachment_at(self.row, root.cloned()) {
                    Ok(identity) => identity,
                    Err(_) if self.row.get_str("parent_attachment_id").is_ok() => {
                        // Retain a successfully authenticated child even when the authoritative
                        // root is missing or contradicts the child's claimed relationship.
                        let field = "data_encrypted";
                        out.findings.push((
                            field,
                            if root.is_none() {
                                Finding::MissingParent
                            } else {
                                Finding::Malformed
                            },
                        ));
                        let path = assistant::attachment_child_path(self.row)?;
                        let path: Vec<_> = path.iter().map(String::as_str).collect();
                        let version_map = self.versions()?;
                        let marker = version(version_map.and_then(|v| v.get(field)))?
                            .map_or(Marker::Unmarked, Marker::Version);
                        let bytes = stored_bytes(self.row.get(field).ok_or_else(invalid)?)?;
                        return self
                            .record(
                                out,
                                field,
                                text(self.row, "user_id")?,
                                &path,
                                marker,
                                &bytes,
                                |_| Ok(()),
                            )
                            .await;
                    }
                    Err(error) => return Err(error),
                };
                let path: Vec<_> = identity.path.iter().map(String::as_str).collect();
                for &name in self.family.entry.fields {
                    if name != identity.field {
                        out.instances.push((name, Storage::Absent));
                    }
                }
                self.record(
                    out,
                    identity.field,
                    text(self.row, "user_id")?,
                    &path,
                    marked(identity.version),
                    &identity.ciphertext,
                    |p| assistant::validate_attachment_plain(&identity, p),
                )
                .await
            }
            Adapter::ToolResult => {
                let row = tool::storage::decode(self.row)?;
                self.record(
                    out,
                    field,
                    &row.user_id,
                    &tool::context_path(&row),
                    marked(row.encryption_version),
                    &row.data_encrypted,
                    |p| {
                        let value = std::str::from_utf8(p).map_err(|_| invalid())?;
                        require(
                            value.len() == row.size as usize
                                && tool::boundaries(value) == row.boundaries,
                        )
                    },
                )
                .await
            }
            Adapter::GeneratedArtifact => {
                let row: PendingArtifact =
                    bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                let Some(bytes) = &row.data_encrypted else {
                    require(
                        matches!(
                            row.state.as_str(),
                            "materialized" | "expired" | "discarded" | "missing"
                        ) && row.secret_context_versions.is_empty(),
                    )?;
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                self.record(
                    out,
                    field,
                    &row.user_id,
                    &artifacts::context_path(&row),
                    marked(row.secret_context_versions.get(field).copied()),
                    bytes,
                    |p| artifacts::storage::validate(&row, p),
                )
                .await
            }
            Adapter::CmaRequest => {
                let Some(request) = cma::snapshot(self.family.collection(), self.row)? else {
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                let v = request.version.map(|v| v as u32);
                out.open(
                    self.keys,
                    field,
                    v.map_or(Marker::Unmarked, Marker::Version),
                    Binding::Record(v, &request.context),
                    &request.ciphertext,
                    |p| require(prepare_creation_request(p)?.hash == request.hash),
                )
                .await;
                Ok(())
            }
            Adapter::Capability => {
                let owner = text(self.row, "user_id")?;
                if !self.row.contains_key(field) && self.row.get_str("status") == Ok("complete") {
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                }
                let bytes = self.row.get_binary_generic(field).map_err(|_| invalid())?;
                match self.row.get("context_version") {
                    Some(Bson::Int32(2)) => {
                        let aad = HistoricalAad::erasure_capability(self.id, owner);
                        out.open(
                            self.keys,
                            field,
                            Marker::Version(2),
                            Binding::Historical(&aad),
                            bytes,
                            provider::validate_capability,
                        )
                        .await;
                        Ok(())
                    }
                    v @ (None | Some(Bson::Int32(3))) => {
                        self.record(
                            out,
                            field,
                            owner,
                            &[field],
                            if v.is_none() {
                                Marker::ImplicitV3
                            } else {
                                Marker::Version(3)
                            },
                            bytes,
                            provider::validate_capability,
                        )
                        .await
                    }
                    _ => Err(invalid()),
                }
            }
            Adapter::AsyncResult => {
                let row = bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                operations::validate_result_storage(&row)?;
                let Some(bytes) = &row.result_encrypted else {
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                self.record(
                    out,
                    field,
                    &row.user_id,
                    &[field],
                    marked(row.secret_context_versions.get(field).copied()),
                    bytes,
                    |p| {
                        require(
                            p.len() <= operations::RESULT_BYTES && std::str::from_utf8(p).is_ok(),
                        )
                    },
                )
                .await
            }
            Adapter::Public => self.public(out, parents).await,
            _ => Err(invalid()),
        }
    }

    async fn public(&self, out: &mut Observation, parents: &Parents) -> AppResult<()> {
        let owner = text(self.row, "sponsor_owner_user_id")?;
        let session = self.family.collection() == "platform_public_sessions";
        // Check the service model (including ingress identity), while still
        // inspecting intact slots if a sibling or lifecycle field is malformed.
        let checked = if session {
            bson::from_document::<crate::models::platform_public_session::PlatformPublicSession>(
                self.row.clone(),
            )
            .map(drop)
        } else {
            bson::from_document::<crate::models::platform_public_turn::PlatformPublicTurn>(
                self.row.clone(),
            )
            .map(drop)
        };
        let first = self.family.entry.fields[0];
        out.check(first, checked.map_err(|_| invalid()));
        let versions = self
            .row
            .get("secret_context_versions")
            .map(|v| v.as_document().ok_or_else(invalid))
            .transpose()?;
        out.check(
            first,
            require(versions.into_iter().flatten().all(|(k, v)| {
                self.family.entry.fields.contains(&k.as_str())
                    && version(Some(v)).ok() == Some(Some(3))
            })),
        );
        if !session {
            match parents.get("platform_public_sessions") {
                None => out.findings.push((first, Finding::MissingParent)),
                Some(p) => out.check(
                    first,
                    require(p.get_str("sponsor_owner_user_id") == Ok(owner)),
                ),
            }
        }
        for &field in self.family.entry.fields {
            let result = async {
                let bytes = stored_bytes(self.row.get(field).ok_or_else(invalid)?)?;
                if bytes.is_empty() {
                    require(match field {
                        "credential_ciphertext" => self.row.get_datetime("revoked_at").is_ok(),
                        "upstream_binding_ciphertext" => {
                            self.row.get_bool("upstream_may_exist") == Ok(false)
                        }
                        "reply_ciphertext" => present(self.row, "reply_id").is_none(),
                        _ => false,
                    })?;
                    out.instances.push((field, Storage::Sentinel));
                    return Ok(());
                }
                let marker = version(versions.and_then(|v| v.get(field)))?
                    .map_or(Marker::Unmarked, Marker::Version);
                if session {
                    let parent = parents.get("api_keys");
                    if field == "credential_ciphertext" && parent.is_none() {
                        out.findings.push((field, Finding::MissingParent));
                    }
                    self.record(
                        out,
                        field,
                        owner,
                        &[field, "api_key", text(self.row, "credential_api_key_id")?],
                        marker,
                        &bytes,
                        |p| {
                            if field == "credential_ciphertext" {
                                issued_key(parent, owner, p)
                            } else {
                                public::validate_encoded_binding(
                                    std::str::from_utf8(p).map_err(|_| invalid())?,
                                )
                            }
                        },
                    )
                    .await
                } else {
                    self.record(
                        out,
                        field,
                        owner,
                        &[
                            field,
                            "session",
                            text(self.row, "public_session_id")?,
                            "conversation",
                            text(self.row, "conversation_id")?,
                            "receipt",
                            text(self.row, "receipt_id")?,
                            "claim",
                            text(self.row, "claim_id")?,
                        ],
                        marker,
                        &bytes,
                        |p| require(std::str::from_utf8(p).is_ok()),
                    )
                    .await
                }
            }
            .await;
            out.check(field, result);
        }
        Ok(())
    }
}
