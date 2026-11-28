//! One-time deliveries can outlive their redemption window on disk.
use super::{companions::Parents, *};
use crate::crypto::token::hash_token;
use crate::services::{
    agent_login_record_service as login, auth_device_service::record as auth,
    device_code_service::record as node, record_field_service::stored_bytes,
};

pub(super) const GRANT: &str = "agent_key_grant.delivery_credential_encrypted";
const ACCESS: &str = "delivery_access_token_encrypted";
const REFRESH: &str = "delivery_refresh_token_encrypted";

pub(super) fn source_companions(entry: &Entry) -> Option<&'static [&'static str]> {
    Some(match entry.adapter {
        Adapter::AgentLogin => &[
            "approved_user_id",
            "api_key_id",
            "credential_id",
            "status",
            "delivery_context_version",
        ],
        Adapter::AuthDevice => &[
            "approved_user_id",
            "approved_session_id",
            "agent_key_grant",
            "login_approval_id",
            "status",
            "delivery_context_version",
        ],
        Adapter::NodeDevice => &[
            "approved_by_user_id",
            "approved_org_id",
            "issued_api_key_id",
            "issued_node_id",
            "refresh_token_hash",
            "device_code_hash",
            "device_pubkey",
            "status",
            "delivery_context_version",
        ],
        _ => return None,
    })
}

impl Source<'_> {
    async fn node_field(
        &self,
        out: &mut Observation,
        row: &crate::models::device_code::DeviceCode,
        field: &'static str,
        marker: Marker,
        parents: &Parents,
    ) -> AppResult<()> {
        let Some(raw) = present(self.row, field) else {
            out.instances.push((field, Storage::Absent));
            return Ok(());
        };
        let binding = node::Binding::stored(row)?;
        let path = binding.path(field);
        let bytes = stored_bytes(raw)?;
        let owner = row
            .approved_org_id
            .as_deref()
            .or(row.approved_by_user_id.as_deref())
            .ok_or_else(invalid)?;
        if marker == Marker::Unmarked
            && field != REFRESH
            && (!parents.contains_key("api_keys") || !parents.contains_key("nodes"))
        {
            out.findings.push((field, Finding::MissingParent));
        }
        self.record(out, field, owner, &path, marker, &bytes, |plain| {
            let value = std::str::from_utf8(plain).map_err(|_| invalid())?;
            if marker != Marker::Unmarked {
                return Ok(());
            }
            if field == REFRESH {
                return (row.refresh_token_hash.as_deref() == Some(hash_token(value).as_str()))
                    .then_some(())
                    .ok_or_else(invalid);
            }
            let key = parents.get("api_keys").ok_or_else(invalid)?;
            let parent = parents.get("nodes").ok_or_else(invalid)?;
            let nodes = key.get_array("allowed_node_ids").map_err(|_| invalid())?;
            let valid = key.get_str("user_id") == Ok(owner)
                && key.get_str("platform") == Ok("device-code")
                && key.get_str("key_hash") == Ok(hash_token(value).as_str())
                && matches!(nodes.as_slice(), [id] if id.as_str() == row.issued_node_id.as_deref())
                && parent.get_str("user_id") == Ok(owner)
                && parent
                    .get_document("metadata")
                    .and_then(|m| m.get_str("provisioning_source"))
                    == Ok(crate::services::node_service::DEVICE_CODE_PROVISIONING_SOURCE);
            valid.then_some(()).ok_or_else(invalid)
        })
        .await
    }

    pub(super) async fn delivery(&self, out: &mut Observation, parents: &Parents) -> AppResult<()> {
        let marker = version(present(self.row, "delivery_context_version"))?
            .map_or(Marker::Unmarked, Marker::Version);
        let fields = self.family.entry.fields;
        match self.family.entry.adapter {
            Adapter::AgentLogin => {
                let field = fields[0];
                let row = login::row(self.row)?;
                let Some(bytes) = &row.delivery_credential_encrypted else {
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                let (owner, parent, child) = login::binding(&row)?;
                self.record(
                    out,
                    field,
                    owner,
                    &[field, parent, child],
                    marker,
                    bytes,
                    |p| bounded_text(p, 4096),
                )
                .await
            }
            Adapter::NodeDevice => {
                out.check(fields[0], node::retained(self.row).map(drop));
                let mut metadata = self.row.clone();
                for &field in fields {
                    metadata.remove(field);
                }
                let row = bson::from_document(metadata).map_err(|_| invalid())?;
                for &field in fields {
                    let result = self.node_field(out, &row, field, marker, parents).await;
                    out.check(field, result);
                }
                Ok(())
            }
            Adapter::AuthDevice => {
                out.check(
                    fields[0],
                    auth::validate_shape(self.family.collection(), self.row),
                );
                for field in [ACCESS, REFRESH, GRANT] {
                    let mut refresh = None;
                    let result = async {
                        let (raw, path) = if field == GRANT {
                            let grant = present(self.row, "agent_key_grant")
                                .map(|v| v.as_document().ok_or_else(invalid))
                                .transpose()?;
                            let Some(grant) = grant
                                .filter(|g| present(g, "delivery_credential_encrypted").is_some())
                            else {
                                out.instances.push((field, Storage::Absent));
                                return Ok(());
                            };
                            (
                                grant
                                    .get("delivery_credential_encrypted")
                                    .ok_or_else(invalid)?,
                                vec![
                                    field,
                                    text(grant, "api_key_id")?,
                                    text(grant, "credential_id")?,
                                ],
                            )
                        } else {
                            let Some(raw) = present(self.row, field) else {
                                out.instances.push((field, Storage::Absent));
                                return Ok(());
                            };
                            (raw, vec![field, text(self.row, "approved_session_id")?])
                        };
                        let bytes = stored_bytes(raw)?;
                        let owner = text(self.row, "approved_user_id")?;
                        if marker == Marker::Unmarked
                            && field == GRANT
                            && !parents.contains_key("api_key_credentials")
                        {
                            out.findings.push((field, Finding::MissingParent));
                        }
                        self.record(out, field, owner, &path, marker, &bytes, |p| {
                            let value = std::str::from_utf8(p).map_err(|_| invalid())?;
                            if marker == Marker::Unmarked {
                                if field == GRANT {
                                    let key =
                                        parents.get("api_key_credentials").ok_or_else(invalid)?;
                                    if key.get_str("api_key_id") != Ok(path[1])
                                        || key.get_str("login_request_id") != Ok(self.id)
                                        || key.get_str("secret_hash")
                                            != Ok(hash_token(value).as_str())
                                    {
                                        return Err(invalid());
                                    }
                                } else {
                                    refresh = auth::issued_session(owner, field, path[1], value)?;
                                }
                            }
                            Ok(())
                        })
                        .await
                    }
                    .await;
                    if field == REFRESH {
                        out.refresh_jti = refresh;
                    }
                    out.check(field, result);
                }
                Ok(())
            }
            _ => Err(invalid()),
        }
    }
}
