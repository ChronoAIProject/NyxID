use super::*;
use crate::models::{
    default_request_header as headers, oauth_state::OAuthState,
    platform_credential::PlatformCredential,
};
use crate::services::{
    mfa_service, node_service, oauth_broker_record_service as broker, oauth_client_webhook_record,
    oauth_state_record_service as oauth, platform_credential_service, saved_login_service,
    ssh_ca_record_service as ssh,
};
use std::collections::HashSet;

fn marker(raw: Option<&Bson>) -> AppResult<Marker> {
    Ok(version(raw)?.map_or(Marker::Unmarked, Marker::Version))
}

impl Source<'_> {
    fn metadata(&self) -> Document {
        // Deserialize only source identity/companions. One malformed sibling
        // ciphertext cannot hide the other material in this snapshot.
        ["_id"]
            .iter()
            .chain(self.family.source_companions)
            .filter_map(|&field| self.row.get(field).map(|v| (field.to_owned(), v.clone())))
            .collect()
    }

    fn versions(&self) -> AppResult<Option<&Document>> {
        self.row
            .get(credentials::VERSIONS)
            .map(|v| v.as_document().ok_or_else(invalid))
            .transpose()
    }

    pub(super) async fn credentials(&self, out: &mut Observation) -> AppResult<()> {
        let collection = self.family.collection();
        let adapter = self.family.entry.adapter;
        let versions = self.versions()?;
        let registration;
        let owner = match adapter {
            Adapter::OAuthWebhook => {
                let creator = present(self.row, "created_by")
                    .map(|v| v.as_str().ok_or_else(invalid))
                    .transpose()?;
                registration = oauth_client_webhook_record::owner(creator);
                &registration
            }
            Adapter::CatalogCredential => header_owner(self.family.entry, self.row)?,
            _ if collection == "provider_configs" => credentials::PLATFORM_OWNER,
            _ => text(self.row, "user_id")?,
        };
        let client = match adapter {
            Adapter::Connection => Some(text(self.row, "service_id")?),
            Adapter::Broker => Some(text(self.row, "client_id")?),
            _ => None,
        };
        if versions
            .into_iter()
            .flat_map(Document::keys)
            .any(|k| !self.family.entry.fields.contains(&k.as_str()))
        {
            out.findings
                .push((self.family.entry.fields[0], Finding::Malformed));
        }
        for &field in self.family.entry.fields {
            let result = async {
                let marker = marker(versions.and_then(|v| v.get(field)))?;
                let historical = if matches!(adapter, Adapter::Broker) {
                    Some(broker::identity(
                        self.id,
                        owner,
                        client.ok_or_else(invalid)?,
                    )?)
                } else {
                    None
                };
                let Some(raw) = present(self.row, field) else {
                    if marker != Marker::Unmarked
                        || matches!(adapter, Adapter::CatalogCredential)
                        || matches!(adapter, Adapter::Broker)
                            && self.row.get_bool("revoked") != Ok(true)
                        || collection == "user_provider_tokens"
                            && self.row.get_str("status") == Ok("active")
                            && matches!(
                                (self.row.get_str("token_type"), field),
                                (Ok("oauth2"), "access_token_encrypted")
                                    | (Ok("api_key"), "api_key_encrypted")
                            )
                    {
                        return Err(invalid());
                    }
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                let bytes = credentials::stored_bytes(raw)?.ok_or_else(invalid)?;
                if bytes.is_empty()
                    && matches!(adapter, Adapter::CatalogCredential)
                    && marker == Marker::Unmarked
                {
                    out.instances.push((field, Storage::Sentinel));
                    return Ok(());
                }
                if let (Some(binding), Marker::Unmarked) = (&historical, marker) {
                    out.open(
                        self.keys,
                        field,
                        marker,
                        Binding::Historical(binding),
                        &bytes,
                        |p| bounded_text(p, usize::MAX),
                    )
                    .await;
                    return Ok(());
                }
                let components = [field, "service_id", client.unwrap_or_default()];
                let broker_path = [field, client.unwrap_or_default()];
                let path = match adapter {
                    Adapter::Connection => &components[..],
                    Adapter::Broker => &broker_path,
                    _ => &components[..1],
                };
                self.record(out, field, owner, path, marker, &bytes, |p| match adapter {
                    Adapter::Credential => {
                        credentials::validate_field_value(collection, self.row, field, p)
                    }
                    Adapter::CatalogCredential if p.is_empty() && marker == Marker::Unmarked => {
                        Ok(())
                    }
                    Adapter::CatalogCredential => bounded_text(p, 65_536),
                    Adapter::Connection => bounded_text(p, 8192),
                    _ => bounded_text(p, usize::MAX),
                })
                .await
            }
            .await;
            out.check(field, result);
        }
        Ok(())
    }

    pub(super) async fn platform(&self, out: &mut Observation) -> AppResult<()> {
        let label = "secrets";
        let versions = self.versions()?;
        let mut metadata = self.metadata();
        metadata.remove(credentials::VERSIONS);
        let secrets = self.row.get(label);
        let row: PlatformCredential = bson::from_document(metadata).map_err(|_| invalid())?;
        out.check(
            label,
            platform_credential_service::validate_plain_fields(&row),
        );
        let secrets = secrets
            .map(|v| v.as_document().ok_or_else(invalid))
            .transpose()?;
        if versions
            .into_iter()
            .flat_map(Document::keys)
            .any(|k| !secrets.is_some_and(|s| s.contains_key(k)))
        {
            out.findings.push((label, Finding::Malformed));
        }
        if secrets.is_none_or(Document::is_empty) {
            out.instances.push((label, Storage::Absent));
        }
        for (name, value) in secrets.into_iter().flatten() {
            let result = async {
                if row.fields.contains_key(name) {
                    out.findings.push((label, Finding::InvalidValue));
                }
                let Bson::Binary(value) = value else {
                    return Err(invalid());
                };
                self.record(
                    out,
                    label,
                    credentials::PLATFORM_OWNER,
                    &[label, name],
                    marker(versions.and_then(|v| v.get(name)))?,
                    &value.bytes,
                    |p| bounded_text(p, 4096),
                )
                .await
            }
            .await;
            out.check(label, result);
        }
        Ok(())
    }

    pub(super) async fn headers(&self, out: &mut Observation) -> AppResult<()> {
        let field = "default_request_headers";
        let owner = header_owner(self.family.entry, self.row)?;
        let rows = present(self.row, field)
            .map(|v| v.as_array().ok_or_else(invalid))
            .transpose()?;
        if rows.is_none_or(Vec::is_empty) {
            out.instances.push((field, Storage::Absent));
        }
        if rows.is_some_and(|rows| rows.len() > headers::MAX_DEFAULT_HEADERS) {
            out.findings.push((field, Finding::Malformed));
        }
        let mut names = HashSet::new();
        for raw in rows.into_iter().flatten() {
            let result = async {
                let row: headers::StoredDefaultRequestHeader =
                    bson::from_bson(raw.clone()).map_err(|_| invalid())?;
                let name = row.name.to_ascii_lowercase();
                if !names.insert(name.clone()) {
                    out.findings.push((field, Finding::Malformed));
                }
                use crate::services::default_header_service::{
                    validate_stored_value, value_context, value_path,
                };
                let path = value_path(&name, row.sensitive);
                match value_context(self.family.collection(), self.id, owner, &row, &path)? {
                    None => {
                        validate_stored_value(&row.name, &row.value)?;
                        out.instances.push((field, Storage::Plaintext));
                    }
                    Some(context) => {
                        let marker =
                            Marker::Version(row.value_context_version.ok_or_else(invalid)?);
                        out.open(
                            self.keys,
                            field,
                            marker,
                            Binding::Record(Some(3), &context),
                            row.value_encrypted.as_deref().ok_or_else(invalid)?,
                            |p| {
                                validate_stored_value(
                                    &row.name,
                                    std::str::from_utf8(p).map_err(|_| invalid())?,
                                )
                            },
                        )
                        .await;
                    }
                }
                Ok(())
            }
            .await;
            out.check(field, result);
        }
        Ok(())
    }

    pub(super) async fn personal(&self, out: &mut Observation) -> AppResult<()> {
        let owner = text(self.row, "user_id")?;
        let adapter = self.family.entry.adapter;
        let version_field = if matches!(adapter, Adapter::Node) {
            "signing_secret_context_version"
        } else {
            "secret_context_version"
        };
        let marker = marker(present(self.row, version_field))?;
        for &field in self.family.entry.fields {
            let result = async {
                let Some(raw) = present(self.row, field) else {
                    let absent = match adapter {
                        Adapter::Mfa => {
                            self.row.get_str("factor_type") == Ok("recovery_codes")
                                && marker == Marker::Unmarked
                        }
                        Adapter::Node => marker == Marker::Unmarked,
                        _ => field != "username_encrypted",
                    };
                    if !absent {
                        return Err(invalid());
                    }
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                let bytes = credentials::stored_bytes(raw)?.ok_or_else(invalid)?;
                self.record(
                    out,
                    field,
                    owner,
                    &[field],
                    marker,
                    &bytes,
                    |p| match adapter {
                        Adapter::Node => node_service::validate_signing_secret_hash(
                            p,
                            self.id,
                            text(self.row, "signing_secret_hash")?,
                        )
                        .map(drop),
                        Adapter::Mfa if self.row.get_str("factor_type") == Ok("totp") => {
                            mfa_service::validate_factor_seed(p, owner)
                        }
                        Adapter::SavedLogin => saved_login_service::validate_plaintext(field, p),
                        _ => Err(invalid()),
                    },
                )
                .await
            }
            .await;
            out.check(field, result);
        }
        Ok(())
    }

    pub(super) async fn ssh(&self, out: &mut Observation) -> AppResult<()> {
        let field = self.family.entry.fields[0];
        let config = present(self.row, "ssh_config")
            .map(|v| {
                bson::from_bson::<crate::models::downstream_service::SshServiceConfig>(v.clone())
                    .map_err(|_| invalid())
            })
            .transpose()?;
        let Some(config) = config else {
            out.instances.push((field, Storage::Absent));
            return Ok(());
        };
        let Some(bytes) = config.ca_private_key_encrypted else {
            if config.ca_context_version.is_some()
                || config.ca_public_key.is_some()
                || config.certificate_auth_enabled
            {
                return Err(invalid());
            }
            out.instances.push((field, Storage::Absent));
            return Ok(());
        };
        let public = config.ca_public_key.ok_or_else(invalid)?;
        let marker = config
            .ca_context_version
            .map_or(Marker::Unmarked, |v| Marker::Version(v.into()));
        self.record(
            out,
            field,
            "platform",
            &[field, "ca_public_key_sha256", &ssh::public_digest(&public)],
            marker,
            &bytes,
            |p| ssh::validate_key(&public, p),
        )
        .await
    }

    pub(super) async fn oauth(&self, out: &mut Observation) -> AppResult<()> {
        let state: OAuthState = bson::from_document(self.metadata()).map_err(|_| invalid())?;
        for &field in self.family.entry.fields {
            let result = async {
                let Some(raw) = present(self.row, field) else {
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                let encoded = raw.as_str().ok_or_else(invalid)?;
                let (marker, version, hex) = match encoded.strip_prefix(oauth::PREFIX) {
                    Some(hex) => (Marker::HexPrefixV3, Some(3), hex),
                    None => (Marker::Unmarked, None, encoded),
                };
                let path = oauth::path(&state, field);
                let context = oauth::context(&state, &path)?;
                out.open(
                    self.keys,
                    field,
                    marker,
                    Binding::Record(version, &context),
                    &hex::decode(hex).map_err(|_| invalid())?,
                    oauth::validate,
                )
                .await;
                Ok(())
            }
            .await;
            out.check(field, result);
        }
        let pending = async {
            let Some(raw) = present(self.row, "pending_callback") else {
                out.instances.push((CALLBACK, Storage::Absent));
                return Ok(());
            };
            let pending: crate::models::oauth_state::PendingOAuthCallback =
                bson::from_bson(raw.clone()).map_err(|_| invalid())?;
            let path = ["pending_callback", &state.provider_config_id, &pending.id];
            let bytes = hex::decode(&pending.encrypted_code).map_err(|_| invalid())?;
            self.record(
                out,
                CALLBACK,
                &state.user_id,
                &path,
                Marker::ImplicitV3,
                &bytes,
                |p| {
                    uuid::Uuid::parse_str(&pending.id).map_err(|_| invalid())?;
                    bounded_text(p, 8192)
                },
            )
            .await
        }
        .await;
        out.check(CALLBACK, pending);
        Ok(())
    }
}
