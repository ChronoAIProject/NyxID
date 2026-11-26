//! Historical field bindings. The owning adapter selects the stored format.
use crate::crypto::aes::EncryptionKeys;
use crate::errors::AppResult;
use std::borrow::Cow;
use zeroize::Zeroizing;

pub(crate) struct HistoricalAad<'a>(Cow<'a, [u8]>);

pub(crate) enum ProposalDomain {
    Assistant,
    External,
}

impl ProposalDomain {
    fn version(self) -> &'static str {
        match self {
            Self::Assistant => "v1",
            Self::External => "v2",
        }
    }
}

impl<'a> HistoricalAad<'a> {
    pub(crate) fn broker_binding(hash: &'a str) -> Option<Self> {
        Self::lower_hex_digest(hash).then_some(Self(Cow::Borrowed(hash.as_bytes())))
    }

    pub(crate) fn native_reply(digest: &'a str) -> Option<Self> {
        Self::lower_hex_digest(digest.strip_prefix("sha256:")?)
            .then_some(Self(Cow::Borrowed(digest.as_bytes())))
    }

    fn lower_hex_digest(hash: &str) -> bool {
        hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }

    pub(crate) fn erasure_capability(id: &str, owner: &str) -> Self {
        let mut aad = b"nyxid:erasure-capability:v1\0".to_vec();
        for part in [
            "personal_erasure_effects",
            id,
            owner,
            "capability_encrypted",
        ] {
            aad.extend_from_slice(&(part.len() as u32).to_be_bytes());
            aad.extend_from_slice(part.as_bytes());
        }
        Self(aad.into())
    }

    pub(crate) fn reattach_window(
        owner: &str,
        conversation: &str,
        credential: &str,
        window: &str,
    ) -> serde_json::Result<Self> {
        serde_json::to_vec(&(
            "nyxid/reattach-window/1",
            owner,
            conversation,
            credential,
            window,
        ))
        .map(|aad| Self(aad.into()))
    }

    pub(crate) fn proposal_payload(domain: ProposalDomain, id: &str) -> Self {
        Self(
            format!("execution-proposal:{}:{id}", domain.version())
                .into_bytes()
                .into(),
        )
    }

    pub(crate) fn proposal_result(domain: ProposalDomain, id: &str) -> Self {
        Self(
            format!("execution-proposal:{}:{id}:result", domain.version())
                .into_bytes()
                .into(),
        )
    }

    pub(crate) fn mobile_delivery(hash: &str, challenge: &str, user: &str) -> Self {
        Self(
            format!("mobile-social-delivery-v1:{hash}:{challenge}:{user}")
                .into_bytes()
                .into(),
        )
    }

    pub(crate) async fn seal(&self, keys: &EncryptionKeys, bytes: &[u8]) -> AppResult<Vec<u8>> {
        keys.encrypt_with_aad(bytes, &self.0).await
    }

    pub(crate) async fn open(
        &self,
        keys: &EncryptionKeys,
        ciphertext: &[u8],
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        keys.decrypt_with_aad(ciphertext, &self.0).await
    }

    #[cfg(test)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests;
