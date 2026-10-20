//! Persist the *existing final domain keys* independently of mutable JWT/AES
//! sources. This changes no derivation, fingerprint bytes or chain format.
use crate::errors::{AppError, AppResult};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;

pub(crate) const DOMAINS: [&str; 6] = [
    "cli_pairing",
    "auth_device",
    "audit_chain",
    "assistant_action",
    "billing_ledger",
    "internal_dispatch",
];
#[derive(Clone)]
pub(crate) struct HmacKeyset(pub(crate) [Zeroizing<[u8; 32]>; 6]);
impl std::fmt::Debug for HmacKeyset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HmacKeyset([REDACTED])")
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    version: u32,
    cli_pairing: Zeroizing<String>,
    auth_device: Zeroizing<String>,
    audit_chain: Zeroizing<String>,
    assistant_action: Zeroizing<String>,
    billing_ledger: Zeroizing<String>,
    internal_dispatch: Zeroizing<String>,
}
fn invalid() -> AppError {
    AppError::Internal(
        "Invalid HMAC keyset; restore the restricted original file and existing domain keys".into(),
    )
}
impl HmacKeyset {
    // These accessors follow DOMAINS and KeyFile's persisted field order.
    pub(crate) fn cli_pairing(&self) -> &Zeroizing<[u8; 32]> {
        &self.0[0]
    }
    pub(crate) fn auth_device(&self) -> &Zeroizing<[u8; 32]> {
        &self.0[1]
    }
    pub(crate) fn audit_chain(&self) -> &Zeroizing<[u8; 32]> {
        &self.0[2]
    }
    pub(crate) fn assistant_action(&self) -> &Zeroizing<[u8; 32]> {
        &self.0[3]
    }
    pub(crate) fn billing_ledger(&self) -> &Zeroizing<[u8; 32]> {
        &self.0[4]
    }
    pub(crate) fn internal_dispatch(&self) -> &Zeroizing<[u8; 32]> {
        &self.0[5]
    }
    #[cfg(test)]
    pub(crate) fn with_changed_domain(&self, domain: &str) -> Self {
        let mut changed = self.clone();
        let key = DOMAINS
            .iter()
            .zip(changed.0.iter_mut())
            .find_map(|(name, key)| (*name == domain).then_some(key))
            .expect("known HMAC domain");
        key[0] ^= 1;
        changed
    }
    pub(crate) fn load(path: &Path) -> AppResult<Self> {
        let file = fs::File::open(path).map_err(|_| invalid())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file.metadata().map_err(|_| invalid())?.permissions().mode() & 0o077 != 0 {
                return Err(invalid());
            }
        }
        // The extra byte detects oversized files without reallocating an
        // application-owned buffer that already contains key material.
        let mut raw = Zeroizing::new(String::with_capacity(4097));
        file.take(4097)
            .read_to_string(&mut raw)
            .map_err(|_| invalid())?;
        if raw.len() > 4096 {
            return Err(invalid());
        }
        let parsed: KeyFile = serde_json::from_str(&raw).map_err(|_| invalid())?;
        if parsed.version != 1 {
            return Err(invalid());
        }
        let values = [
            parsed.cli_pairing,
            parsed.auth_device,
            parsed.audit_chain,
            parsed.assistant_action,
            parsed.billing_ledger,
            parsed.internal_dispatch,
        ];
        let mut keys = std::array::from_fn(|_| Zeroizing::new([0u8; 32]));
        for (key, value) in keys.iter_mut().zip(values.iter()) {
            if value.len() != 64 {
                return Err(invalid());
            }
            hex::decode_to_slice(value.as_bytes(), key.as_mut()).map_err(|_| invalid())?;
        }
        Ok(Self(keys))
    }
    /// Never truncate an existing keyset; publish the path only after sync.
    /// A failed/partial export must be removed/restored by the operator.
    pub(crate) fn export_new(&self, path: &Path) -> AppResult<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|_| invalid())?;
        let write = |file: &mut fs::File| -> std::io::Result<()> {
            file.write_all(b"{\"version\":1")?;
            for (domain, key) in DOMAINS.iter().zip(self.0.iter()) {
                let encoded = Zeroizing::new(hex::encode(key.as_slice()));
                write!(file, ",\"{domain}\":\"{}\"", encoded.as_str())?;
            }
            file.write_all(b"}\n")?;
            file.sync_all()
        };
        write(&mut file).map_err(|_| invalid())
    }
    pub(crate) fn commitments(&self) -> Vec<String> {
        DOMAINS
            .iter()
            .zip(self.0.iter())
            .map(|(domain, key)| {
                let mut hash = Sha256::new();
                hash.update(b"nyxid:hmac-lifecycle:commitment:v1\0");
                hash.update(domain.as_bytes());
                hash.update([0]);
                hash.update(key.as_slice());
                hex::encode(hash.finalize())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricted_export_roundtrips_exact_keys_and_never_overwrites() {
        let keys = HmacKeyset(std::array::from_fn(|i| Zeroizing::new([i as u8 + 17; 32])));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hmac.json");
        keys.export_new(&path).unwrap();
        let loaded = HmacKeyset::load(&path).unwrap();
        // Distinct fixture bytes detect an accessor drifting from the named
        // persisted domain, even when export/load still agree positionally.
        let exported: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for (domain, key) in [
            ("cli_pairing", loaded.cli_pairing()),
            ("auth_device", loaded.auth_device()),
            ("audit_chain", loaded.audit_chain()),
            ("assistant_action", loaded.assistant_action()),
            ("billing_ledger", loaded.billing_ledger()),
            ("internal_dispatch", loaded.internal_dispatch()),
        ] {
            assert_eq!(
                exported[domain].as_str().unwrap(),
                hex::encode(key.as_slice())
            );
        }
        assert_eq!(keys.commitments(), loaded.commitments());
        for (a, b) in keys.0.iter().zip(loaded.0.iter()) {
            assert_eq!(a.as_slice(), b.as_slice());
        }
        assert!(keys.export_new(&path).is_err());
        assert_eq!(
            keys.commitments(),
            HmacKeyset::load(&path).unwrap().commitments()
        );
        assert!(!format!("{keys:?}").contains(&hex::encode([17; 32])));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(HmacKeyset::load(&path).is_err());
        }
    }
    #[test]
    fn malformed_keysets_have_fixed_diagnostics_and_bounded_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        for raw in [
            "distinctive-secret-json".to_string(),
            "x".repeat(4097),
            r#"{"version":2,"auth_device":"distinctive-secret-json"}"#.into(),
        ] {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(&path)
                .unwrap()
                .write_all(raw.as_bytes())
                .unwrap();
            let error = HmacKeyset::load(&path).err().unwrap();
            assert!(!format!("{error:?} {error}").contains("distinctive-secret"));
        }
    }
}
