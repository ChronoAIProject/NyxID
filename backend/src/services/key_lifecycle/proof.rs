//! Open each proof with one configured key. Selector equality cannot establish
//! possession. A role change preserves the logical reference and its context.
use super::{HEAD, conflict, contract::Key};
use crate::{
    config::AppConfig,
    crypto::{
        aes::{EncryptionKeys, RecordContext},
        key_provider::KeyProvider,
        local_key_provider::LocalKeyProvider,
    },
    errors::{AppError, AppResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use zeroize::Zeroizing;

pub(super) struct Slot {
    role: &'static str,
    kind: &'static str,
    resource: Option<String>,
    keys: EncryptionKeys,
}

impl Slot {
    pub(super) fn primary(&self) -> bool {
        self.role == "primary"
    }

    pub(super) fn pinned(&self, config: &AppConfig) -> bool {
        if self.kind != "aws-kms" {
            return true;
        }
        // Enrollment resolves aliases for possession checks. Live AWS writers
        // pass the configured selector to KMS on every wrap.
        let selector = if self.primary() {
            &config.aws_kms_key_arn
        } else {
            &config.aws_kms_key_arn_previous
        };
        selector.is_some() && *selector == self.resource
    }

    pub(super) fn strict_reader(mut self) -> EncryptionKeys {
        self.keys.set_allow_legacy_record_reads(false);
        self.keys
    }

    fn new(
        role: &'static str,
        kind: &'static str,
        resource: Option<String>,
        provider: Arc<dyn KeyProvider>,
    ) -> Self {
        let mut keys = EncryptionKeys::with_provider(provider);
        // This isolated one-key codec only writes the fixed enrollment challenge.
        keys.set_contextual_record_writes(true);
        Self {
            role,
            kind,
            resource,
            keys,
        }
    }
}

pub(super) async fn slots(config: &AppConfig) -> AppResult<Vec<Slot>> {
    let mut result = Vec::new();
    let local_primary = config.key_provider == "local";
    if !local_primary {
        cloud_slots(config, &mut result).await?;
    }
    for (role, value) in [
        (
            if local_primary { "primary" } else { "fallback" },
            config.encryption_key.as_ref(),
        ),
        (
            if local_primary {
                "previous"
            } else {
                "fallback_previous"
            },
            config
                .encryption_key_previous
                .as_ref()
                .filter(|previous| Some(*previous) != config.encryption_key.as_ref()),
        ),
    ] {
        if let Some(value) = value {
            let mut bytes = Zeroizing::new([0; 32]);
            hex::decode_to_slice(value, bytes.as_mut()).map_err(|_| unavailable())?;
            result.push(Slot::new(
                role,
                "local",
                None,
                Arc::new(LocalKeyProvider::new(*bytes, None)),
            ));
        }
    }
    if result.is_empty()
        || (config.encryption_key_previous.is_some() && config.encryption_key.is_none())
    {
        return Err(unavailable());
    }
    Ok(result)
}

fn unavailable() -> AppError {
    conflict("Exact enrollment key unavailable")
}

async fn cloud_slots(config: &AppConfig, result: &mut Vec<Slot>) -> AppResult<()> {
    let _ = (config, &result);
    match config.key_provider.as_str() {
        #[cfg(feature = "aws-kms")]
        "aws-kms" => {
            use crate::crypto::aws_kms_provider::AwsKmsProvider;
            let sdk = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let client = aws_sdk_kms::Client::new(&sdk);
            for (role, name) in [(
                "primary",
                config.aws_kms_key_arn.as_deref().ok_or_else(unavailable)?,
            )]
            .into_iter()
            .chain(
                config
                    .aws_kms_key_arn_previous
                    .as_deref()
                    .map(|n| ("previous", n)),
            ) {
                let described = client
                    .describe_key()
                    .key_id(name)
                    .send()
                    .await
                    .map_err(|_| unavailable())?;
                let arn = described
                    .key_metadata()
                    .and_then(|key| key.arn())
                    .ok_or_else(unavailable)?;
                let mut exact = config.clone();
                exact.aws_kms_key_arn = Some(arn.to_owned());
                exact.aws_kms_key_arn_previous = None;
                let provider = AwsKmsProvider::from_config(&exact).await;
                result.push(Slot::new(
                    role,
                    "aws-kms",
                    Some(arn.to_owned()),
                    Arc::new(provider),
                ));
            }
            Ok(())
        }
        #[cfg(feature = "gcp-kms")]
        "gcp-kms" => {
            use crate::crypto::gcp_kms_provider::GcpKmsProvider;
            use futures::FutureExt;
            use std::panic::AssertUnwindSafe;
            for (role, name) in [(
                "primary",
                config.gcp_kms_key_name.as_deref().ok_or_else(unavailable)?,
            )]
            .into_iter()
            .chain(
                config
                    .gcp_kms_key_name_previous
                    .as_deref()
                    .map(|n| ("previous", n)),
            ) {
                let parts = name.split('/').collect::<Vec<_>>();
                if !matches!(parts.as_slice(),["projects",p,"locations",l,"keyRings",r,"cryptoKeys",k] if [p,l,r,k].iter().all(|s| !s.is_empty()))
                {
                    return Err(unavailable());
                }
                let mut exact = config.clone();
                exact.gcp_kms_key_name = Some(name.to_owned());
                exact.gcp_kms_key_name_previous = None;
                // The startup constructor panics on failed client initialization.
                // Refuse enrollment and keep the provider future owned by the request.
                let provider = AssertUnwindSafe(GcpKmsProvider::from_config(&exact))
                    .catch_unwind()
                    .await
                    .map_err(|_| unavailable())?;
                result.push(Slot::new(
                    role,
                    "gcp-kms",
                    Some(name.to_owned()),
                    Arc::new(provider),
                ));
            }
            Ok(())
        }
        _ => Err(unavailable()),
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Proof {
    pub key: Key,
    binding: String,
    envelope: bson::Binary,
}

fn resource_binding(deployment: &str, kind: &str, resource: Option<&str>) -> String {
    let mut digest = Sha256::new();
    for part in [
        "nyxid/enrollment/resource/v1",
        deployment,
        kind,
        resource.unwrap_or(""),
    ] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    hex::encode(digest.finalize())
}

pub(super) async fn issue(deployment: &str, slots: &[Slot]) -> AppResult<Vec<Proof>> {
    let mut proofs = Vec::new();
    for slot in slots {
        let reference = uuid::Uuid::new_v4().to_string();
        let bound = resource_binding(deployment, slot.kind, slot.resource.as_deref());
        let fields = ["key_possession", bound.as_str()];
        let ctx = RecordContext::new(HEAD, deployment, &reference, &fields)?;
        let bytes = slot
            .keys
            .encrypt_record(b"nyxid/enrollment/v1", &ctx)
            .await
            .map_err(|_| unavailable())?;
        proofs.push(Proof {
            key: Key {
                key_ref: reference,
                role: slot.role.into(),
                provider: slot.kind.into(),
                internal_version: if slot.kind == "local" {
                    "exact"
                } else {
                    "unverified"
                }
                .into(),
            },
            binding: bound,
            envelope: bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes,
            },
        });
    }
    verify(deployment, slots, &proofs).await?;
    Ok(proofs)
}

async fn identify<'a>(deployment: &str, slot: &Slot, proofs: &'a [Proof]) -> AppResult<&'a Key> {
    let bound = resource_binding(deployment, slot.kind, slot.resource.as_deref());
    let mut found = None;
    for proof in proofs
        .iter()
        .filter(|p| p.binding == bound && p.key.provider == slot.kind)
    {
        let fields = ["key_possession", bound.as_str()];
        let ctx = RecordContext::new(HEAD, deployment, &proof.key.key_ref, &fields)?;
        if slot
            .keys
            .authenticate_current_record(&proof.envelope.bytes, &ctx)
            .await
            .is_ok()
            && found.replace(&proof.key).is_some()
        {
            return Err(unavailable());
        }
    }
    found.ok_or_else(unavailable)
}

pub(super) async fn verify(deployment: &str, slots: &[Slot], proofs: &[Proof]) -> AppResult<()> {
    if slots.len() != proofs.len() {
        return Err(unavailable());
    }
    for slot in slots {
        if identify(deployment, slot, proofs).await?.role != slot.role {
            return Err(unavailable());
        }
    }
    Ok(())
}

impl Proof {
    pub(crate) fn reference(
        &self,
        deployment: &str,
        role: &str,
        provider: &str,
        resource: Option<&str>,
    ) -> Option<&str> {
        (self.key.role == role
            && self.key.provider == provider
            && self.binding == resource_binding(deployment, provider, resource))
        .then_some(self.key.key_ref.as_str())
    }
}

#[cfg(test)]
mod tests;
