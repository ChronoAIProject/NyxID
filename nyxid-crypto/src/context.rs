use crate::error::{RciCryptoError, Result};

const KDF_PREFIX: &[u8] = b"nyxid:rci:v1:kdf\0";
const AAD_PREFIX: &[u8] = b"nyxid:rci:v1:aad\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RciContext {
    pub node_id: String,
    pub pending_credential_id: String,
    pub service_slug: String,
    pub injection_method: String,
    pub field_name: String,
    pub target_url: Option<String>,
    pub version: String,
}

impl RciContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node_id: impl Into<String>,
        pending_credential_id: impl Into<String>,
        service_slug: impl Into<String>,
        injection_method: impl Into<String>,
        field_name: impl Into<String>,
        target_url: Option<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            pending_credential_id: pending_credential_id.into(),
            service_slug: service_slug.into(),
            injection_method: injection_method.into(),
            field_name: field_name.into(),
            target_url,
            version: version.into(),
        }
    }

    pub fn kdf_info_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(KDF_PREFIX);
        push_lp(&mut out, "node_id", &self.node_id)?;
        push_lp(
            &mut out,
            "pending_credential_id",
            &self.pending_credential_id,
        )?;
        push_lp(&mut out, "service_slug", &self.service_slug)?;
        push_lp(&mut out, "version", &self.version)?;
        Ok(out)
    }

    pub fn aad_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(AAD_PREFIX);
        push_lp(&mut out, "node_id", &self.node_id)?;
        push_lp(
            &mut out,
            "pending_credential_id",
            &self.pending_credential_id,
        )?;
        push_lp(&mut out, "service_slug", &self.service_slug)?;
        push_lp(&mut out, "injection_method", &self.injection_method)?;
        push_lp(&mut out, "field_name", &self.field_name)?;
        push_lp(
            &mut out,
            "target_url",
            self.target_url.as_deref().unwrap_or(""),
        )?;
        push_lp(&mut out, "version", &self.version)?;
        Ok(out)
    }
}

fn push_lp(out: &mut Vec<u8>, field: &'static str, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let len = u16::try_from(bytes.len()).map_err(|_| RciCryptoError::FieldTooLong { field })?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
