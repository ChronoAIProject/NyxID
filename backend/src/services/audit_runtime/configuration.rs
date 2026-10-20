//! Admin-custody public enrollment and separately provisioned append credential.
//! No network request, automatic enrollment or Mongo trust discovery at startup.
use std::{
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
    sync::Arc,
};

use ed25519_dalek::VerifyingKey;
use nyxid::{
    models::audit_witness::{SignedReceipt, WitnessEvent},
    services::audit_witness::{
        HttpWitness, WitnessClient, ZERO_HASH, receipt_digest, verify_receipt,
    },
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessPin {
    pub protocol: u32,
    pub verification_key: String,
    pub enrollment: SignedReceipt,
}

impl WitnessPin {
    pub fn key(&self) -> Result<VerifyingKey, &'static str> {
        let bytes: [u8; 32] = hex::decode(&self.verification_key)
            .map_err(|_| "Invalid witness public pin")?
            .try_into()
            .map_err(|_| "Invalid witness public pin")?;
        VerifyingKey::from_bytes(&bytes).map_err(|_| "Invalid witness public pin")
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let body = &self.enrollment.body;
        if self.protocol != 1
            || body.chain_id.get_version_num() != 4
            || body.receipt_seq != 0
            || body.previous_receipt != ZERO_HASH
            || body.head.key_version != 1
            || body.event
                != (WitnessEvent::Enrollment {
                    head: body.head.clone(),
                })
        {
            return Err("Invalid witness enrollment pin");
        }
        verify_receipt(&self.enrollment, body.chain_id, &self.key()?)
            .map_err(|_| "Invalid witness enrollment signature")
    }

    pub fn digest(&self) -> String {
        receipt_digest(&self.enrollment)
    }
}

#[derive(Clone)]
pub struct ConfiguredWitness {
    pub pin: WitnessPin,
    pub client: Arc<dyn WitnessClient>,
}

impl std::fmt::Debug for ConfiguredWitness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfiguredWitness([REDACTED])")
    }
}

impl ConfiguredWitness {
    pub fn from_env() -> Result<Option<Self>, &'static str> {
        let read = |name| match std::env::var(name) {
            Ok(v) if !v.is_empty() => Ok(Some(v)),
            Ok(_) => Err("Witness configuration must not be empty"),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("Witness configuration must be UTF-8"),
        };
        Self::load(
            read("AUDIT_WITNESS_URL")?.as_deref(),
            read("AUDIT_WITNESS_PIN_FILE")?.as_deref(),
            read("AUDIT_WITNESS_TOKEN_FILE")?.as_deref(),
        )
    }

    fn load(
        url: Option<&str>,
        pin: Option<&str>,
        token: Option<&str>,
    ) -> Result<Option<Self>, &'static str> {
        let (url, pin, token) = match (url, pin, token) {
            (None, None, None) => return Ok(None),
            (Some(url), Some(pin), Some(token)) => (url, pin, token),
            _ => return Err("Configure all three witness URL/pin-file/token-file settings"),
        };
        // The application must be unable to replace the pin or its ancestors.
        // Open the complete path component-by-component, without following links.
        let pin_bytes = read_custodied(Path::new(pin), false, 16 * 1024)?;
        let pin: WitnessPin =
            serde_json::from_slice(&pin_bytes).map_err(|_| "Invalid witness enrollment pin")?;
        pin.validate()?;
        let token = read_custodied(Path::new(token), true, 65)?;
        let token_text = std::str::from_utf8(&token).map_err(|_| "Invalid witness credential")?;
        let token = Zeroizing::new(
            token_text
                .strip_suffix('\n')
                .unwrap_or(token_text)
                .to_owned(),
        );
        let client = HttpWitness::new(
            url,
            pin.enrollment.body.chain_id,
            &pin.verification_key,
            token,
        )
        .map_err(|_| "Invalid witness endpoint or credential")?;
        Ok(Some(Self {
            pin,
            client: Arc::new(client),
        }))
    }
}

/// Root/admin-owned public pin; private credential may be app-owned. Directory
/// descriptors prevent symlink/rename traversal races. No permission repair.
fn read_custodied(
    path: &Path,
    credential: bool,
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: geteuid takes no arguments.
    let uid = unsafe { libc::geteuid() };
    if uid == 0 || !path.is_absolute() {
        return Err(
            "Witness files require an unprivileged application and absolute custodied paths",
        );
    }
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return Err("Invalid witness custody path");
    }
    let mut parent = File::open("/").map_err(|_| "Witness custody unavailable")?;
    let pieces: Vec<_> = components.collect();
    if pieces.is_empty() {
        return Err("Invalid witness custody path");
    }
    for (index, component) in pieces.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err("Invalid witness custody path");
        };
        let directory = index + 1 < pieces.len();
        let metadata = parent
            .metadata()
            .map_err(|_| "Witness custody unavailable")?;
        if !metadata.is_dir()
            || (!credential && metadata.uid() == uid)
            || metadata.mode() & 0o022 != 0
        {
            return Err("Witness custody path is writable by the application or another principal");
        }
        let name =
            std::ffi::CString::new(name.as_bytes()).map_err(|_| "Invalid witness custody path")?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if directory {
                libc::O_DIRECTORY
            } else {
                libc::O_NONBLOCK
            };
        // SAFETY: parent is open; name is a live NUL-terminated component.
        let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 {
            return Err("Witness custody unavailable");
        }
        // SAFETY: successful openat returns a new owned descriptor.
        parent = unsafe { File::from_raw_fd(descriptor) };
    }
    let metadata = parent
        .metadata()
        .map_err(|_| "Witness custody unavailable")?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || (!credential && (metadata.uid() == uid || metadata.mode() & 0o222 != 0))
        || (credential && (metadata.uid() != uid || metadata.mode() & 0o777 != 0o600))
    {
        return Err("Unsafe witness file custody");
    }
    let mut bytes = Zeroizing::new(Vec::new());
    parent
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Witness file unavailable")?;
    if bytes.len() > limit {
        return Err("Witness file exceeds limit");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_witness_configuration_requires_complete_explicit_configuration() {
        assert!(ConfiguredWitness::load(None, None, None).unwrap().is_none());
        for (url, pin, token) in [
            (Some("https://w.example"), None, None),
            (None, Some("/pin"), Some("/token")),
            (Some("https://w.example"), Some("/pin"), None),
        ] {
            assert!(ConfiguredWitness::load(url, pin, token).is_err());
        }
        assert!(read_custodied(Path::new("relative"), false, 100).is_err());
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("pin.json");
        std::fs::write(&path, b"{}").unwrap();
        assert!(
            read_custodied(&path, false, 100).is_err(),
            "application-owned pins cannot establish custody"
        );
    }
}
