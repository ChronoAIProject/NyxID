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

mod families;
#[cfg(test)]
mod tests;

pub(crate) struct Family {
    entry: &'static Entry,
    /// Every mutable companion for these codecs lives on the source row.
    /// Watching its _id covers changes to any of these paths in either direction.
    pub(crate) source_companions: &'static [&'static str],
}

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
            Adapter::Broker => &["user_id", "client_id", "secret_context_versions", "revoked"],
            Adapter::OAuthWebhook => &["created_by", "secret_context_versions"],
            _ => return None,
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
}

#[derive(Default)]
pub(crate) struct Observation {
    pub(crate) instances: Vec<(&'static str, Storage)>,
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
        self.entry.fields.iter().copied()
    }

    pub(crate) async fn observe(&self, keys: &EncryptionKeys, row: &Document) -> Observation {
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
                _ => Err(invalid()),
            }
        }
        .await;
        out.check(self.entry.fields[0], observed);
        for (field, storage) in &out.instances {
            if matches!(
                storage,
                Storage::Encrypted {
                    contract: Contract::Unbound,
                    ..
                }
            ) {
                out.findings.push((*field, Finding::AmbiguousBinding));
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
