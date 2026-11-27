//! Pure consumers of a single snapshot document. Only static labels and
//! authenticated evidence escape this module, including on partial failure.
use super::{Adapter, Entry, REGISTRY, header_owner, invalid};
use crate::crypto::{
    aes::{EncryptionKeys, RecordContext},
    record_field::authenticated::{self, Binding, Evidence, Route},
};
use crate::errors::AppResult;
use crate::services::{channel_signing_migration, credential_record_service as credentials};
use bson::{Bson, Document};

mod companions;
mod deliveries;
mod families;
mod nested;
mod retained;
#[cfg(test)]
mod tests;

pub(crate) struct Family {
    entry: &'static Entry,
    /// Mutable source-row inputs in addition to the registered ciphertext slots.
    pub(crate) source_companions: &'static [&'static str],
}

const CALLBACK: &str = "pending_callback.encrypted_code";

pub(crate) fn families() -> impl Iterator<Item = Family> {
    REGISTRY.iter().filter_map(|entry| {
        let companions: &[_] = match entry.adapter {
            Adapter::Credential if entry.collection == "provider_configs" => &[
                "secret_context_versions",
                "channel_signing_preparation",
                "slug",
            ],
            Adapter::Credential if entry.collection == "user_provider_tokens" => {
                &["user_id", "secret_context_versions", "status", "token_type"]
            }
            Adapter::Credential => &["user_id", "secret_context_versions", "credential_type"],
            Adapter::CatalogCredential => {
                &["owner_user_id", "created_by", "secret_context_versions"]
            }
            Adapter::Connection => &["user_id", "service_id", "secret_context_versions"],
            Adapter::Header if entry.collection == "downstream_services" => {
                &["owner_user_id", "created_by"]
            }
            Adapter::Header => &["user_id"],
            Adapter::PlatformCredential => &[
                "provider",
                "updated_by",
                "updated_at",
                "fields",
                "secret_context_versions",
                "channel_signing_preparation",
            ],
            Adapter::Node => &[
                "user_id",
                "signing_secret_context_version",
                "signing_secret_hash",
            ],
            Adapter::Mfa => &["user_id", "secret_context_version", "factor_type"],
            Adapter::SavedLogin => &["user_id", "secret_context_version"],
            Adapter::CatalogCa => &[
                "ssh_config.ca_public_key",
                "ssh_config.ca_context_version",
                "ssh_config.certificate_auth_enabled",
            ],
            Adapter::OAuthState => &[
                "user_id",
                "target_user_id",
                "credential_user_id",
                "connection_id",
                "provider_config_id",
                "created_at",
                "expires_at",
                "pending_callback.id",
                "pending_callback.expires_at",
            ],
            Adapter::Broker => &["user_id", "client_id", "secret_context_versions", "revoked"],
            Adapter::OAuthWebhook => &["created_by", "secret_context_versions"],
            _ => retained::source_companions(entry)
                .or_else(|| deliveries::source_companions(entry))
                .or_else(|| nested::source_companions(entry))?,
        };
        Some(Family {
            entry,
            source_companions: companions,
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Marker {
    Unmarked,
    Version(u32),
    HexPrefixV3,
    ImplicitV3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Contract {
    Unbound,
    HistoricalAad,
    Contextual,
}

pub(crate) enum Storage {
    Absent,
    Sentinel,
    Plaintext,
    Encrypted {
        marker: Marker,
        contract: Contract,
        evidence: Evidence,
        empty: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Finding {
    Malformed,
    Authentication,
    InvalidValue,
    AmbiguousBinding,
    UnknownKey,
    MissingParent,
}

#[derive(Default)]
pub(crate) struct Observation {
    pub(crate) instances: Vec<(&'static str, Storage)>,
    refresh_jti: Option<zeroize::Zeroizing<String>>,
    pub(crate) parents: Vec<crate::services::encrypted_record_census::Row>,
    pub(crate) findings: Vec<(&'static str, Finding)>,
}

impl Observation {
    fn check(&mut self, label: &'static str, value: AppResult<()>) {
        if value.is_err() {
            self.findings.push((label, Finding::Malformed));
        }
    }

    async fn open(
        &mut self,
        keys: &EncryptionKeys,
        label: &'static str,
        marker: Marker,
        binding: Binding<'_>,
        bytes: &[u8],
        check: impl FnOnce(&[u8]) -> AppResult<()>,
    ) {
        let contract = match binding {
            Binding::Record(None, _) => Contract::Unbound,
            Binding::Record(Some(_), _) => Contract::Contextual,
            Binding::Historical(_) | Binding::Oracle(_) => Contract::HistoricalAad,
        };
        match authenticated::observe(keys, bytes, binding).await {
            Err(_) => self.findings.push((label, Finding::Authentication)),
            Ok(result) => {
                let (plain, evidence) = result.into_parts();
                if check(&plain).is_err() {
                    self.findings.push((label, Finding::InvalidValue));
                }
                if evidence.route == Route::Unknown {
                    self.findings.push((label, Finding::UnknownKey));
                }
                self.instances.push((
                    label,
                    Storage::Encrypted {
                        marker,
                        contract,
                        evidence,
                        empty: plain.is_empty(),
                    },
                ));
            }
        }
    }
}

struct Source<'a> {
    keys: &'a EncryptionKeys,
    family: &'a Family,
    row: &'a Document,
    id: &'a str,
}

fn present<'a>(row: &'a Document, field: &str) -> Option<&'a Bson> {
    row.get(field).filter(|v| **v != Bson::Null)
}
fn text<'a>(row: &'a Document, field: &str) -> AppResult<&'a str> {
    row.get_str(field)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(invalid)
}
fn version(value: Option<&Bson>) -> AppResult<Option<u32>> {
    value
        .map(|v| bson::from_bson(v.clone()).map_err(|_| invalid()))
        .transpose()
}
fn bounded_text(bytes: &[u8], max: usize) -> AppResult<()> {
    if bytes.is_empty() || bytes.len() > max || std::str::from_utf8(bytes).is_err() {
        return Err(invalid());
    }
    Ok(())
}

impl Family {
    pub(crate) fn collection(&self) -> &'static str {
        self.entry.collection
    }
    pub(crate) fn fields(&self) -> impl Iterator<Item = &'static str> {
        self.entry
            .fields
            .iter()
            .copied()
            .chain(matches!(self.entry.adapter, Adapter::OAuthState).then_some(CALLBACK))
            .chain((self.collection() == "auth_device_codes").then_some(deliveries::GRANT))
    }

    async fn observe(
        &self,
        keys: &EncryptionKeys,
        row: &Document,
        parents: &companions::Parents,
    ) -> Observation {
        let mut out = Observation::default();
        let observed = async {
            let source = Source {
                keys,
                family: self,
                row,
                id: text(row, "_id")?,
            };
            if matches!(
                self.collection(),
                "platform_credentials" | "provider_configs"
            ) {
                out.check(
                    self.entry.fields[0],
                    channel_signing_migration::verify_state(self.collection(), row),
                );
            }
            match self.entry.adapter {
                Adapter::Credential
                | Adapter::OAuthWebhook
                | Adapter::Connection
                | Adapter::CatalogCredential
                | Adapter::Broker => source.credentials(&mut out).await,
                Adapter::PlatformCredential => source.platform(&mut out).await,
                Adapter::Header => source.headers(&mut out).await,
                Adapter::Node | Adapter::Mfa | Adapter::SavedLogin => {
                    source.personal(&mut out).await
                }
                Adapter::CatalogCa => source.ssh(&mut out).await,
                Adapter::OAuthState => source.oauth(&mut out).await,
                Adapter::LearningProposal => Box::pin(source.learning(&mut out)).await,
                Adapter::ReattachWindow => source.window(&mut out).await,
                Adapter::ApprovalProposal => source.approval(&mut out).await,
                Adapter::AgentLogin | Adapter::AuthDevice | Adapter::NodeDevice => {
                    Box::pin(source.delivery(&mut out, parents)).await
                }
                _ => Box::pin(source.retained(&mut out, parents)).await,
            }
        }
        .await;
        out.check(self.entry.fields[0], observed);
        // Node hashes and SSH public keys independently corroborate old plaintext.
        if !matches!(
            self.entry.adapter,
            Adapter::Node
                | Adapter::CatalogCa
                | Adapter::CmaRequest
                | Adapter::AssistantCredential
                | Adapter::GeneratedArtifact
                | Adapter::NodeDevice
                | Adapter::AuthDevice
        ) {
            for (field, storage) in &out.instances {
                if !(self.collection() == "platform_public_sessions"
                    && *field == "credential_ciphertext")
                    && matches!(
                        storage,
                        Storage::Encrypted {
                            contract: Contract::Unbound,
                            ..
                        }
                    )
                {
                    out.findings.push((*field, Finding::AmbiguousBinding));
                }
            }
        }
        out
    }
}

impl Source<'_> {
    #[allow(
        clippy::too_many_arguments,
        reason = "Every authenticated identity component is explicit at the codec call site"
    )]
    async fn record(
        &self,
        out: &mut Observation,
        label: &'static str,
        owner: &str,
        path: &[&str],
        marker: Marker,
        bytes: &[u8],
        check: impl FnOnce(&[u8]) -> AppResult<()>,
    ) -> AppResult<()> {
        let context = RecordContext::new(self.family.collection(), self.id, owner, path)?;
        let version = match marker {
            Marker::Unmarked => None,
            Marker::Version(v) => Some(v),
            Marker::HexPrefixV3 | Marker::ImplicitV3 => Some(3),
        };
        out.open(
            self.keys,
            label,
            marker,
            Binding::Record(version, &context),
            bytes,
            check,
        )
        .await;
        Ok(())
    }
}
