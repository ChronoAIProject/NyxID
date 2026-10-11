use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::model::{
    CredentialBundle, PendingLoginRecovery, PendingLoginRecoveryPhase, is_session_id,
};

const KEYRING_SERVICE: &str = "dev.nyxid.companion";
const SESSION_MANIFEST_ACCOUNT: &str = "account-session/manifest-v3";
const SESSION_SLOT_PREFIX: &str = "account-session/slot";
const V2_MANIFEST_ACCOUNT: &str = "account-session/manifest";
const V2_PAYLOAD_PREFIX: &str = "account-session/payload";
const PENDING_MANIFEST_ACCOUNT: &str = "pending-revoke/manifest-v3";
const PENDING_SLOT_PREFIX: &str = "pending-revoke/slot";
const PENDING_LOGIN_ACCOUNT: &str = "pending-login/recovery-v1";
const LEGACY_ACCOUNT: &str = "account-session/default";
const MANIFEST_VERSION: u32 = 3;
const PENDING_LOGIN_VERSION: u32 = 1;

// Windows Credential Manager rejects credential blobs larger than 2560 bytes.
const MAX_KEYRING_SECRET_BYTES: usize = 2400;

fn keyring_secret_fits(raw: &[u8]) -> bool {
    !raw.is_empty() && raw.len() <= MAX_KEYRING_SECRET_BYTES
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreOperation {
    Read,
    Write,
    Delete,
}

#[derive(Clone)]
pub(crate) struct StoreError {
    pub(crate) operation: StoreOperation,
}

impl fmt::Debug for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreError")
            .field("operation", &self.operation)
            .finish()
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "credential store {:?} failed", self.operation)
    }
}

impl std::error::Error for StoreError {}

pub(crate) trait SessionStore: Send + Sync {
    fn load(&self) -> Result<Option<CredentialBundle>, StoreError>;
    fn save(&self, bundle: &CredentialBundle) -> Result<(), StoreError>;
    fn clear(&self) -> Result<(), StoreError>;
    fn load_pending_revoke(&self) -> Result<Option<CredentialBundle>, StoreError>;
    fn save_pending_revoke(&self, bundle: &CredentialBundle) -> Result<(), StoreError>;
    fn clear_pending_revoke(&self) -> Result<(), StoreError>;
    fn load_pending_login(&self) -> Result<Option<PendingLoginRecovery>, StoreError>;
    fn save_pending_login(&self, recovery: &PendingLoginRecovery) -> Result<(), StoreError>;
    fn clear_pending_login(&self) -> Result<(), StoreError>;
}

pub(crate) type SharedSessionStore = Arc<dyn SessionStore>;

trait SecretBackend: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<Vec<u8>>, ()>;
    fn write(&self, account: &str, secret: &[u8]) -> Result<(), ()>;
    fn delete(&self, account: &str) -> Result<(), ()>;
}

struct SystemKeyringBackend;

impl SystemKeyringBackend {
    fn entry(account: &str) -> Result<keyring::Entry, ()> {
        keyring::Entry::new(KEYRING_SERVICE, account).map_err(|_| ())
    }
}

impl SecretBackend for SystemKeyringBackend {
    fn read(&self, account: &str) -> Result<Option<Vec<u8>>, ()> {
        match Self::entry(account)?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(()),
        }
    }

    fn write(&self, account: &str, secret: &[u8]) -> Result<(), ()> {
        Self::entry(account)?.set_secret(secret).map_err(|_| ())
    }

    fn delete(&self, account: &str) -> Result<(), ()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    }
}

pub(crate) struct KeyringSessionStore {
    backend: Arc<dyn SecretBackend>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Slot {
    A,
    B,
}

impl Slot {
    fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }
}

#[derive(Clone, Copy)]
enum Area {
    Session,
    PendingRevoke,
}

impl Area {
    fn manifest_account(self) -> &'static str {
        match self {
            Self::Session => SESSION_MANIFEST_ACCOUNT,
            Self::PendingRevoke => PENDING_MANIFEST_ACCOUNT,
        }
    }

    fn slot_prefix(self) -> &'static str {
        match self {
            Self::Session => SESSION_SLOT_PREFIX,
            Self::PendingRevoke => PENDING_SLOT_PREFIX,
        }
    }

    fn payload_account(self, slot: Slot, kind: &str) -> String {
        format!("{}/{}/{}", self.slot_prefix(), slot.label(), kind)
    }
}

#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
struct StoredManifest {
    version: u32,
    transaction_id: String,
    active_slot: Option<Slot>,
    active_session_id: Option<String>,
    active_access_expires_at: Option<DateTime<Utc>>,
    retired_slot: Option<Slot>,
}

impl StoredManifest {
    fn empty() -> Self {
        Self {
            version: MANIFEST_VERSION,
            transaction_id: uuid::Uuid::new_v4().to_string(),
            active_slot: None,
            active_session_id: None,
            active_access_expires_at: None,
            retired_slot: None,
        }
    }

    fn active(
        bundle: &CredentialBundle,
        transaction_id: String,
        active_slot: Slot,
        retired_slot: Option<Slot>,
    ) -> Self {
        Self {
            version: MANIFEST_VERSION,
            transaction_id,
            active_slot: Some(active_slot),
            active_session_id: Some(bundle.session_id.clone()),
            active_access_expires_at: Some(bundle.access_expires_at),
            retired_slot,
        }
    }

    fn is_valid(&self) -> bool {
        if self.version != MANIFEST_VERSION || !is_session_id(&self.transaction_id) {
            return false;
        }
        match (
            self.active_slot,
            self.active_session_id.as_deref(),
            self.active_access_expires_at,
        ) {
            (None, None, None) => self.retired_slot.is_none(),
            (Some(active), Some(session_id), Some(_)) => {
                is_session_id(session_id) && self.retired_slot != Some(active)
            }
            _ => false,
        }
    }
}

#[derive(Deserialize)]
struct V2Manifest {
    version: u32,
    payload_id: String,
    access_expires_at: DateTime<Utc>,
    session_id: String,
    access_account: String,
    refresh_account: String,
}

#[derive(Deserialize, Zeroize)]
#[zeroize(drop)]
struct LegacyStoredBundle {
    version: u32,
    access_token: String,
    refresh_token: String,
    #[zeroize(skip)]
    access_expires_at: DateTime<Utc>,
    session_id: String,
}

#[derive(Deserialize, Serialize, Zeroize)]
#[zeroize(drop)]
struct StoredLoginRecovery {
    version: u32,
    device_code: String,
    recovery_secret: String,
    attempt_id: String,
    #[zeroize(skip)]
    phase: PendingLoginRecoveryPhase,
}

impl KeyringSessionStore {
    pub(crate) fn shared() -> SharedSessionStore {
        Arc::new(Self {
            backend: Arc::new(SystemKeyringBackend),
        })
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn SecretBackend>) -> Self {
        Self { backend }
    }

    fn read_raw(
        &self,
        account: &str,
        operation: StoreOperation,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
        let raw = self
            .backend
            .read(account)
            .map_err(|()| StoreError { operation })?;
        match raw {
            Some(raw) => {
                let raw = Zeroizing::new(raw);
                if !keyring_secret_fits(&raw) {
                    return Err(StoreError { operation });
                }
                Ok(Some(raw))
            }
            None => Ok(None),
        }
    }

    fn write_raw(
        &self,
        account: &str,
        raw: &[u8],
        operation: StoreOperation,
    ) -> Result<(), StoreError> {
        if !keyring_secret_fits(raw) {
            return Err(StoreError { operation });
        }
        self.backend
            .write(account, raw)
            .map_err(|()| StoreError { operation })
    }

    fn delete_account(&self, account: &str) -> Result<(), StoreError> {
        self.backend.delete(account).map_err(|()| StoreError {
            operation: StoreOperation::Delete,
        })
    }

    fn read_manifest(&self, area: Area) -> Result<Option<StoredManifest>, StoreError> {
        let operation = StoreOperation::Read;
        let Some(raw) = self.read_raw(area.manifest_account(), operation)? else {
            return Ok(None);
        };
        let manifest: StoredManifest =
            serde_json::from_slice(&raw).map_err(|_| StoreError { operation })?;
        manifest
            .is_valid()
            .then_some(manifest)
            .ok_or(StoreError { operation })
            .map(Some)
    }

    fn write_manifest(&self, area: Area, manifest: &StoredManifest) -> Result<(), StoreError> {
        let operation = StoreOperation::Write;
        let raw =
            Zeroizing::new(serde_json::to_vec(manifest).map_err(|_| StoreError { operation })?);
        self.write_raw(area.manifest_account(), &raw, operation)
    }

    fn commit_manifest(&self, area: Area, manifest: &StoredManifest) -> Result<(), StoreError> {
        match self.write_manifest(area, manifest) {
            Ok(()) => Ok(()),
            Err(error) => match self.read_manifest(area) {
                Ok(Some(actual)) if actual.transaction_id == manifest.transaction_id => Ok(()),
                _ => Err(error),
            },
        }
    }

    fn cleanup_slot(&self, area: Area, slot: Slot) -> Result<(), StoreError> {
        let access = self.delete_account(&area.payload_account(slot, "access"));
        let refresh = self.delete_account(&area.payload_account(slot, "refresh"));
        access.and(refresh)
    }

    fn cleanup_all_slots(&self, area: Area) -> Result<(), StoreError> {
        let a = self.cleanup_slot(area, Slot::A);
        let b = self.cleanup_slot(area, Slot::B);
        a.and(b)
    }

    fn recover_manifest(
        &self,
        area: Area,
        mut manifest: StoredManifest,
    ) -> Result<StoredManifest, StoreError> {
        let Some(active_slot) = manifest.active_slot else {
            self.cleanup_all_slots(area)?;
            return Ok(manifest);
        };
        self.cleanup_slot(area, active_slot.other())?;
        if manifest.retired_slot.is_some() {
            manifest.retired_slot = None;
            self.write_manifest(area, &manifest)?;
        }
        Ok(manifest)
    }

    fn load_area(&self, area: Area) -> Result<Option<CredentialBundle>, StoreError> {
        let manifest = match self.read_manifest(area) {
            Ok(Some(manifest)) => self.recover_manifest(area, manifest)?,
            Ok(None) => {
                self.cleanup_all_slots(area)?;
                if matches!(area, Area::Session) {
                    return self.load_legacy();
                }
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if matches!(area, Area::Session) {
            self.delete_account(LEGACY_ACCOUNT)?;
        }
        let Some(slot) = manifest.active_slot else {
            return Ok(None);
        };
        let operation = StoreOperation::Read;
        let access = self
            .read_raw(&area.payload_account(slot, "access"), operation)?
            .ok_or(StoreError { operation })?;
        let refresh = self
            .read_raw(&area.payload_account(slot, "refresh"), operation)?
            .ok_or(StoreError { operation })?;
        let session_id = manifest.active_session_id.ok_or(StoreError { operation })?;
        let access_expires_at = manifest
            .active_access_expires_at
            .ok_or(StoreError { operation })?;
        credential_bundle_from_secrets(
            decode_payload(access, &manifest.transaction_id, operation)?,
            decode_payload(refresh, &manifest.transaction_id, operation)?,
            access_expires_at,
            session_id,
            operation,
        )
        .map(Some)
    }

    fn save_area(&self, area: Area, bundle: &CredentialBundle) -> Result<(), StoreError> {
        if !is_session_id(&bundle.session_id) {
            return Err(StoreError {
                operation: StoreOperation::Write,
            });
        }

        let current = match self.read_manifest(area)? {
            Some(manifest) => self.recover_manifest(area, manifest)?,
            None => {
                self.cleanup_all_slots(area)?;
                StoredManifest::empty()
            }
        };
        let target = current.active_slot.map_or(Slot::A, Slot::other);
        self.cleanup_slot(area, target)?;

        let operation = StoreOperation::Write;
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let access_payload = encode_payload(&transaction_id, &bundle.access_token, operation)?;
        let refresh_payload = encode_payload(&transaction_id, &bundle.refresh_token, operation)?;
        self.write_raw(
            &area.payload_account(target, "access"),
            &access_payload,
            operation,
        )?;
        if let Err(error) = self.write_raw(
            &area.payload_account(target, "refresh"),
            &refresh_payload,
            operation,
        ) {
            let _ = self.cleanup_slot(area, target);
            return Err(error);
        }

        let mut committed =
            StoredManifest::active(bundle, transaction_id, target, current.active_slot);
        self.commit_manifest(area, &committed)?;

        if let Some(retired) = committed.retired_slot
            && self.cleanup_slot(area, retired).is_ok()
        {
            committed.retired_slot = None;
            let _ = self.write_manifest(area, &committed);
        }
        if matches!(area, Area::Session) {
            let _ = self.delete_account(LEGACY_ACCOUNT);
        }
        Ok(())
    }

    fn clear_area(&self, area: Area) -> Result<(), StoreError> {
        // The tombstone is the durable invalidation point. Payload deletion can
        // then be retried after a crash without making an old session readable.
        self.commit_manifest(area, &StoredManifest::empty())?;
        self.cleanup_all_slots(area)?;
        if matches!(area, Area::Session) {
            self.delete_account(LEGACY_ACCOUNT)?;
        }
        self.delete_account(area.manifest_account())
    }

    fn load_legacy(&self) -> Result<Option<CredentialBundle>, StoreError> {
        let operation = StoreOperation::Read;
        let Some(raw) = self.read_raw(LEGACY_ACCOUNT, operation)? else {
            return Ok(None);
        };
        let mut stored: LegacyStoredBundle =
            serde_json::from_slice(&raw).map_err(|_| StoreError { operation })?;
        if stored.version != 1 || !is_session_id(&stored.session_id) {
            stored.zeroize();
            return Err(StoreError { operation });
        }
        let access = Zeroizing::new(std::mem::take(&mut stored.access_token));
        let refresh = Zeroizing::new(std::mem::take(&mut stored.refresh_token));
        let session_id = std::mem::take(&mut stored.session_id);
        credential_bundle_from_secrets(
            access,
            refresh,
            stored.access_expires_at,
            session_id,
            operation,
        )
        .map(Some)
    }

    fn migrate_v2_if_present(&self) -> Result<(), StoreError> {
        let operation = StoreOperation::Read;
        let Some(raw) = self.read_raw(V2_MANIFEST_ACCOUNT, operation)? else {
            return Ok(());
        };
        let manifest: V2Manifest =
            serde_json::from_slice(&raw).map_err(|_| StoreError { operation })?;
        let expected_access = format!("{V2_PAYLOAD_PREFIX}/{}/access", manifest.payload_id);
        let expected_refresh = format!("{V2_PAYLOAD_PREFIX}/{}/refresh", manifest.payload_id);
        if manifest.version != 2
            || !is_session_id(&manifest.payload_id)
            || manifest.access_account != expected_access
            || manifest.refresh_account != expected_refresh
        {
            return Err(StoreError { operation });
        }

        if self.read_manifest(Area::Session)?.is_none() {
            let access = self
                .read_raw(&manifest.access_account, operation)?
                .ok_or(StoreError { operation })?;
            let refresh = self
                .read_raw(&manifest.refresh_account, operation)?
                .ok_or(StoreError { operation })?;
            let bundle = credential_bundle_from_secrets(
                decode_raw_secret(access, operation)?,
                decode_raw_secret(refresh, operation)?,
                manifest.access_expires_at,
                manifest.session_id.clone(),
                operation,
            )?;
            self.save_area(Area::Session, &bundle)?;
        }

        let access = self.delete_account(&manifest.access_account);
        let refresh = self.delete_account(&manifest.refresh_account);
        access.and(refresh)?;
        self.delete_account(V2_MANIFEST_ACCOUNT)
    }

    fn load_pending_login_record(&self) -> Result<Option<PendingLoginRecovery>, StoreError> {
        let operation = StoreOperation::Read;
        let Some(raw) = self.read_raw(PENDING_LOGIN_ACCOUNT, operation)? else {
            return Ok(None);
        };
        let mut stored: StoredLoginRecovery =
            serde_json::from_slice(&raw).map_err(|_| StoreError { operation })?;
        if stored.version != PENDING_LOGIN_VERSION {
            return Err(StoreError { operation });
        }
        let mut recovery = PendingLoginRecovery::new(
            std::mem::take(&mut stored.device_code),
            std::mem::take(&mut stored.recovery_secret),
            std::mem::take(&mut stored.attempt_id),
        )
        .ok_or(StoreError { operation })?;
        recovery.phase = stored.phase;
        Ok(Some(recovery))
    }

    fn save_pending_login_record(&self, recovery: &PendingLoginRecovery) -> Result<(), StoreError> {
        let operation = StoreOperation::Write;
        let stored = StoredLoginRecovery {
            version: PENDING_LOGIN_VERSION,
            device_code: recovery.device_code.to_string(),
            recovery_secret: recovery.recovery_secret.to_string(),
            attempt_id: recovery.attempt_id.clone(),
            phase: recovery.phase,
        };
        let raw =
            Zeroizing::new(serde_json::to_vec(&stored).map_err(|_| StoreError { operation })?);
        match self.write_raw(PENDING_LOGIN_ACCOUNT, &raw, operation) {
            Ok(()) => Ok(()),
            Err(error) => match self.load_pending_login_record() {
                Ok(Some(actual))
                    if actual.attempt_id == recovery.attempt_id
                        && actual.device_code == recovery.device_code
                        && actual.recovery_secret == recovery.recovery_secret
                        && actual.phase == recovery.phase =>
                {
                    Ok(())
                }
                _ => Err(error),
            },
        }
    }

    fn clear_pending_login_record(&self) -> Result<(), StoreError> {
        match self.delete_account(PENDING_LOGIN_ACCOUNT) {
            Ok(()) => Ok(()),
            Err(error) => match self.read_raw(PENDING_LOGIN_ACCOUNT, StoreOperation::Read) {
                Ok(None) => Ok(()),
                _ => Err(error),
            },
        }
    }
}

impl SessionStore for KeyringSessionStore {
    fn load(&self) -> Result<Option<CredentialBundle>, StoreError> {
        self.migrate_v2_if_present()?;
        self.load_area(Area::Session)
    }

    fn save(&self, bundle: &CredentialBundle) -> Result<(), StoreError> {
        self.migrate_v2_if_present()?;
        self.save_area(Area::Session, bundle)
    }

    fn clear(&self) -> Result<(), StoreError> {
        self.migrate_v2_if_present()?;
        self.clear_area(Area::Session)
    }

    fn load_pending_revoke(&self) -> Result<Option<CredentialBundle>, StoreError> {
        self.load_area(Area::PendingRevoke)
    }

    fn save_pending_revoke(&self, bundle: &CredentialBundle) -> Result<(), StoreError> {
        if let Some(existing) = self.load_pending_revoke()?
            && existing.session_id != bundle.session_id
        {
            return Err(StoreError {
                operation: StoreOperation::Write,
            });
        }
        self.save_area(Area::PendingRevoke, bundle)
    }

    fn clear_pending_revoke(&self) -> Result<(), StoreError> {
        self.clear_area(Area::PendingRevoke)
    }

    fn load_pending_login(&self) -> Result<Option<PendingLoginRecovery>, StoreError> {
        self.load_pending_login_record()
    }

    fn save_pending_login(&self, recovery: &PendingLoginRecovery) -> Result<(), StoreError> {
        if let Some(existing) = self.load_pending_login_record()?
            && existing.attempt_id != recovery.attempt_id
        {
            return Err(StoreError {
                operation: StoreOperation::Write,
            });
        }
        self.save_pending_login_record(recovery)
    }

    fn clear_pending_login(&self) -> Result<(), StoreError> {
        self.clear_pending_login_record()
    }
}

fn encode_payload(
    transaction_id: &str,
    secret: &str,
    operation: StoreOperation,
) -> Result<Zeroizing<Vec<u8>>, StoreError> {
    if !is_session_id(transaction_id) || secret.trim().is_empty() {
        return Err(StoreError { operation });
    }
    let mut payload = Zeroizing::new(Vec::with_capacity(transaction_id.len() + 1 + secret.len()));
    payload.extend_from_slice(transaction_id.as_bytes());
    payload.push(b'\n');
    payload.extend_from_slice(secret.as_bytes());
    keyring_secret_fits(&payload)
        .then_some(payload)
        .ok_or(StoreError { operation })
}

fn decode_payload(
    mut raw: Zeroizing<Vec<u8>>,
    expected_transaction_id: &str,
    operation: StoreOperation,
) -> Result<Zeroizing<String>, StoreError> {
    let Some(separator) = raw.iter().position(|byte| *byte == b'\n') else {
        return Err(StoreError { operation });
    };
    if &raw[..separator] != expected_transaction_id.as_bytes() {
        return Err(StoreError { operation });
    }
    let mut secret_bytes = raw.split_off(separator + 1);
    raw.zeroize();
    match String::from_utf8(std::mem::take(&mut secret_bytes)) {
        Ok(secret) => Ok(Zeroizing::new(secret)),
        Err(error) => {
            let mut bytes = error.into_bytes();
            bytes.zeroize();
            Err(StoreError { operation })
        }
    }
}

fn decode_raw_secret(
    mut raw: Zeroizing<Vec<u8>>,
    operation: StoreOperation,
) -> Result<Zeroizing<String>, StoreError> {
    let bytes = std::mem::take(&mut *raw);
    match String::from_utf8(bytes) {
        Ok(secret) => Ok(Zeroizing::new(secret)),
        Err(error) => {
            let mut bytes = error.into_bytes();
            bytes.zeroize();
            Err(StoreError { operation })
        }
    }
}

fn credential_bundle_from_secrets(
    mut access: Zeroizing<String>,
    mut refresh: Zeroizing<String>,
    access_expires_at: DateTime<Utc>,
    session_id: String,
    operation: StoreOperation,
) -> Result<CredentialBundle, StoreError> {
    if !is_session_id(&session_id) || access.trim().is_empty() || refresh.trim().is_empty() {
        return Err(StoreError { operation });
    }
    let access = std::mem::take(&mut *access);
    let refresh = std::mem::take(&mut *refresh);
    CredentialBundle::new(access, refresh, access_expires_at)
        .and_then(|bundle| bundle.for_session_id(session_id))
        .ok_or(StoreError { operation })
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    type MemoryBundle = (String, String, DateTime<Utc>, String);
    type MemoryLoginRecovery = (String, String, String, PendingLoginRecoveryPhase);

    #[derive(Default)]
    pub(crate) struct MemoryStore {
        value: Mutex<Option<MemoryBundle>>,
        pending: Mutex<Option<MemoryBundle>>,
        pending_login: Mutex<Option<MemoryLoginRecovery>>,
        fail_write: Mutex<bool>,
        fail_pending_write: Mutex<bool>,
        fail_pending_login_write: Mutex<bool>,
        fail_delete: Mutex<bool>,
        fail_read: Mutex<bool>,
    }

    impl MemoryStore {
        pub(crate) fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub(crate) fn set_fail_write(&self, value: bool) {
            *self.fail_write.lock().unwrap() = value;
        }

        pub(crate) fn set_fail_pending_write(&self, value: bool) {
            *self.fail_pending_write.lock().unwrap() = value;
        }

        pub(crate) fn set_fail_pending_login_write(&self, value: bool) {
            *self.fail_pending_login_write.lock().unwrap() = value;
        }

        pub(crate) fn set_fail_delete(&self, value: bool) {
            *self.fail_delete.lock().unwrap() = value;
        }

        pub(crate) fn set_fail_read(&self, value: bool) {
            *self.fail_read.lock().unwrap() = value;
        }

        fn load_value(value: &Mutex<Option<MemoryBundle>>) -> Option<CredentialBundle> {
            value
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|(access, refresh, expires, session_id)| {
                    CredentialBundle::new(access.clone(), refresh.clone(), *expires)
                        .and_then(|bundle| bundle.for_session_id(session_id.clone()))
                })
        }

        fn save_value(value: &Mutex<Option<MemoryBundle>>, bundle: &CredentialBundle) {
            *value.lock().unwrap() = Some((
                bundle.access_token.to_string(),
                bundle.refresh_token.to_string(),
                bundle.access_expires_at,
                bundle.session_id.clone(),
            ));
        }

        fn load_login_value(
            value: &Mutex<Option<MemoryLoginRecovery>>,
        ) -> Option<PendingLoginRecovery> {
            value.lock().unwrap().as_ref().and_then(
                |(device_code, recovery_secret, attempt_id, phase)| {
                    let mut recovery = PendingLoginRecovery::new(
                        device_code.clone(),
                        recovery_secret.clone(),
                        attempt_id.clone(),
                    )?;
                    recovery.phase = *phase;
                    Some(recovery)
                },
            )
        }

        fn save_login_value(
            value: &Mutex<Option<MemoryLoginRecovery>>,
            recovery: &PendingLoginRecovery,
        ) {
            *value.lock().unwrap() = Some((
                recovery.device_code.to_string(),
                recovery.recovery_secret.to_string(),
                recovery.attempt_id.clone(),
                recovery.phase,
            ));
        }
    }

    impl SessionStore for MemoryStore {
        fn load(&self) -> Result<Option<CredentialBundle>, StoreError> {
            if *self.fail_read.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Read,
                });
            }
            Ok(Self::load_value(&self.value))
        }

        fn save(&self, bundle: &CredentialBundle) -> Result<(), StoreError> {
            if *self.fail_write.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Write,
                });
            }
            Self::save_value(&self.value, bundle);
            Ok(())
        }

        fn clear(&self) -> Result<(), StoreError> {
            if *self.fail_delete.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Delete,
                });
            }
            self.value.lock().unwrap().take();
            Ok(())
        }

        fn load_pending_revoke(&self) -> Result<Option<CredentialBundle>, StoreError> {
            Ok(Self::load_value(&self.pending))
        }

        fn save_pending_revoke(&self, bundle: &CredentialBundle) -> Result<(), StoreError> {
            if *self.fail_pending_write.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Write,
                });
            }
            if Self::load_value(&self.pending)
                .is_some_and(|existing| existing.session_id != bundle.session_id)
            {
                return Err(StoreError {
                    operation: StoreOperation::Write,
                });
            }
            Self::save_value(&self.pending, bundle);
            Ok(())
        }

        fn clear_pending_revoke(&self) -> Result<(), StoreError> {
            if *self.fail_delete.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Delete,
                });
            }
            self.pending.lock().unwrap().take();
            Ok(())
        }

        fn load_pending_login(&self) -> Result<Option<PendingLoginRecovery>, StoreError> {
            if *self.fail_read.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Read,
                });
            }
            Ok(Self::load_login_value(&self.pending_login))
        }

        fn save_pending_login(&self, recovery: &PendingLoginRecovery) -> Result<(), StoreError> {
            if *self.fail_pending_login_write.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Write,
                });
            }
            if Self::load_login_value(&self.pending_login)
                .is_some_and(|existing| existing.attempt_id != recovery.attempt_id)
            {
                return Err(StoreError {
                    operation: StoreOperation::Write,
                });
            }
            Self::save_login_value(&self.pending_login, recovery);
            Ok(())
        }

        fn clear_pending_login(&self) -> Result<(), StoreError> {
            if *self.fail_delete.lock().unwrap() {
                return Err(StoreError {
                    operation: StoreOperation::Delete,
                });
            }
            self.pending_login.lock().unwrap().take();
            Ok(())
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum FaultAction {
        Read,
        Write,
        Delete,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FaultTiming {
        Before,
        After,
    }

    #[derive(Default)]
    pub(crate) struct MemorySecretBackend {
        entries: Mutex<HashMap<String, Vec<u8>>>,
        faults: Mutex<Vec<(FaultAction, FaultTiming, String)>>,
    }

    impl MemorySecretBackend {
        pub(crate) fn fail_next(&self, action: FaultAction, account: &str) {
            self.faults
                .lock()
                .unwrap()
                .push((action, FaultTiming::Before, account.to_owned()));
        }

        pub(crate) fn fail_next_after(&self, action: FaultAction, account: &str) {
            self.faults
                .lock()
                .unwrap()
                .push((action, FaultTiming::After, account.to_owned()));
        }

        pub(crate) fn contains(&self, account: &str) -> bool {
            self.entries.lock().unwrap().contains_key(account)
        }

        fn take_fault(&self, action: FaultAction, account: &str) -> Option<FaultTiming> {
            let mut faults = self.faults.lock().unwrap();
            let index = faults
                .iter()
                .position(|candidate| candidate.0 == action && candidate.2 == account)?;
            Some(faults.remove(index).1)
        }
    }

    impl SecretBackend for MemorySecretBackend {
        fn read(&self, account: &str) -> Result<Option<Vec<u8>>, ()> {
            if self.take_fault(FaultAction::Read, account).is_some() {
                return Err(());
            }
            Ok(self.entries.lock().unwrap().get(account).cloned())
        }

        fn write(&self, account: &str, secret: &[u8]) -> Result<(), ()> {
            let fault = self.take_fault(FaultAction::Write, account);
            if fault == Some(FaultTiming::Before) {
                return Err(());
            }
            self.entries
                .lock()
                .unwrap()
                .insert(account.to_owned(), secret.to_vec());
            (fault != Some(FaultTiming::After)).then_some(()).ok_or(())
        }

        fn delete(&self, account: &str) -> Result<(), ()> {
            let fault = self.take_fault(FaultAction::Delete, account);
            if fault == Some(FaultTiming::Before) {
                return Err(());
            }
            self.entries.lock().unwrap().remove(account);
            (fault != Some(FaultTiming::After)).then_some(()).ok_or(())
        }
    }

    pub(crate) fn keyring_store(backend: Arc<MemorySecretBackend>) -> KeyringSessionStore {
        KeyringSessionStore::with_backend(backend)
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{FaultAction, MemorySecretBackend, MemoryStore, keyring_store};
    use super::*;

    fn bundle(access: &str) -> CredentialBundle {
        CredentialBundle::new(
            access.to_owned(),
            format!("{access}-refresh"),
            Utc::now() + chrono::Duration::hours(1),
        )
        .unwrap()
    }

    fn login_recovery() -> PendingLoginRecovery {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        PendingLoginRecovery::new(
            format!("nyx_adc_{}", URL_SAFE_NO_PAD.encode([3_u8; 32])),
            URL_SAFE_NO_PAD.encode([4_u8; 32]),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
    }

    #[test]
    fn mock_store_round_trips_and_deletes_one_bundle() {
        let store = MemoryStore::shared();
        let bundle = bundle("access");
        store.save(&bundle).unwrap();
        assert_eq!(
            store.load().unwrap().unwrap().access_token.as_str(),
            "access"
        );
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn access_refresh_and_manifest_write_faults_preserve_the_old_slot() {
        for account in [
            "account-session/slot/b/access",
            "account-session/slot/b/refresh",
            SESSION_MANIFEST_ACCOUNT,
        ] {
            let backend = Arc::new(MemorySecretBackend::default());
            let store = keyring_store(backend.clone());
            store.save(&bundle("old")).unwrap();
            backend.fail_next(FaultAction::Write, account);

            assert!(store.save(&bundle("new")).is_err(), "{account}");
            assert_eq!(store.load().unwrap().unwrap().access_token.as_str(), "old");
            assert!(!backend.contains("account-session/slot/b/access"));
            assert!(!backend.contains("account-session/slot/b/refresh"));
        }
    }

    #[test]
    fn after_mutation_payload_faults_recover_old_slot_without_mixing_tokens() {
        for account in [
            "account-session/slot/b/access",
            "account-session/slot/b/refresh",
        ] {
            let backend = Arc::new(MemorySecretBackend::default());
            let store = keyring_store(backend.clone());
            store.save(&bundle("old")).unwrap();
            backend.fail_next_after(FaultAction::Write, account);

            assert!(store.save(&bundle("new")).is_err(), "{account}");
            assert_eq!(store.load().unwrap().unwrap().access_token.as_str(), "old");
            assert!(!backend.contains("account-session/slot/b/access"));
            assert!(!backend.contains("account-session/slot/b/refresh"));
        }
    }

    #[test]
    fn manifest_after_mutation_error_is_read_back_as_committed() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        store.save(&bundle("old")).unwrap();
        backend.fail_next_after(FaultAction::Write, SESSION_MANIFEST_ACCOUNT);

        store.save(&bundle("new")).unwrap();

        assert_eq!(store.load().unwrap().unwrap().access_token.as_str(), "new");
    }

    #[test]
    fn retired_delete_failure_is_journaled_and_recovered_on_next_load() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        store.save(&bundle("old")).unwrap();
        backend.fail_next(FaultAction::Delete, "account-session/slot/a/access");

        store.save(&bundle("new")).unwrap();

        assert!(backend.contains("account-session/slot/a/access"));
        assert_eq!(store.load().unwrap().unwrap().access_token.as_str(), "new");
        assert!(!backend.contains("account-session/slot/a/access"));
        assert!(!backend.contains("account-session/slot/a/refresh"));
    }

    #[test]
    fn crash_residue_in_inactive_slot_is_removed_by_load() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        store.save(&bundle("active")).unwrap();
        backend
            .write("account-session/slot/b/access", b"crash-access")
            .unwrap();
        backend
            .write("account-session/slot/b/refresh", b"crash-refresh")
            .unwrap();

        assert_eq!(
            store.load().unwrap().unwrap().access_token.as_str(),
            "active"
        );
        assert!(!backend.contains("account-session/slot/b/access"));
        assert!(!backend.contains("account-session/slot/b/refresh"));
    }

    #[test]
    fn manifest_read_failure_preserves_active_slots_for_later_recovery() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        store.save(&bundle("active")).unwrap();
        backend.fail_next(FaultAction::Read, SESSION_MANIFEST_ACCOUNT);

        assert!(store.load().is_err());
        assert!(backend.contains("account-session/slot/a/access"));
        assert!(backend.contains("account-session/slot/a/refresh"));
        assert_eq!(
            store.load().unwrap().unwrap().access_token.as_str(),
            "active"
        );
    }

    #[test]
    fn clear_keeps_tombstone_when_slot_delete_fails_and_recovers() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        store.save(&bundle("active")).unwrap();
        backend.fail_next(FaultAction::Delete, "account-session/slot/a/refresh");

        assert!(store.clear().is_err());
        assert!(backend.contains(SESSION_MANIFEST_ACCOUNT));
        assert!(store.load().unwrap().is_none());
        store.clear().unwrap();
        assert!(!backend.contains(SESSION_MANIFEST_ACCOUNT));
        assert!(!backend.contains("account-session/slot/a/refresh"));
    }

    #[test]
    fn pending_revoke_uses_a_separate_recoverable_double_slot() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        store.save_pending_revoke(&bundle("pending")).unwrap();
        assert_eq!(
            store
                .load_pending_revoke()
                .unwrap()
                .unwrap()
                .access_token
                .as_str(),
            "pending"
        );
        store.clear_pending_revoke().unwrap();
        assert!(!backend.contains(PENDING_MANIFEST_ACCOUNT));
    }

    #[test]
    fn pending_login_recovery_survives_store_recreation_and_phase_update() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        let mut recovery = login_recovery();
        store.save_pending_login(&recovery).unwrap();
        assert!(backend.contains(PENDING_LOGIN_ACCOUNT));

        let restarted = keyring_store(backend.clone());
        let restored = restarted.load_pending_login().unwrap().unwrap();
        assert_eq!(restored.attempt_id, recovery.attempt_id);
        assert_eq!(restored.device_code, recovery.device_code);
        assert_eq!(restored.recovery_secret, recovery.recovery_secret);
        assert_eq!(restored.phase, PendingLoginRecoveryPhase::AwaitingDelivery);

        recovery.mark_delivery_committing();
        restarted.save_pending_login(&recovery).unwrap();
        let committed = keyring_store(backend.clone())
            .load_pending_login()
            .unwrap()
            .unwrap();
        assert_eq!(
            committed.phase,
            PendingLoginRecoveryPhase::DeliveryCommitting
        );

        keyring_store(backend.clone())
            .clear_pending_login()
            .unwrap();
        assert!(!backend.contains(PENDING_LOGIN_ACCOUNT));
    }

    #[test]
    fn pending_login_write_after_mutation_is_read_back_as_committed() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        let recovery = login_recovery();
        backend.fail_next_after(FaultAction::Write, PENDING_LOGIN_ACCOUNT);

        store.save_pending_login(&recovery).unwrap();

        assert_eq!(
            store.load_pending_login().unwrap().unwrap().attempt_id,
            recovery.attempt_id
        );
    }

    #[test]
    fn failed_pending_login_delete_retains_recovery_for_retry() {
        let backend = Arc::new(MemorySecretBackend::default());
        let store = keyring_store(backend.clone());
        store.save_pending_login(&login_recovery()).unwrap();
        backend.fail_next(FaultAction::Delete, PENDING_LOGIN_ACCOUNT);

        assert!(store.clear_pending_login().is_err());
        assert!(store.load_pending_login().unwrap().is_some());
    }

    #[test]
    fn portable_keyring_limit_is_fail_closed() {
        assert!(keyring_secret_fits(&vec![b'x'; MAX_KEYRING_SECRET_BYTES]));
        assert!(!keyring_secret_fits(&vec![
            b'x';
            MAX_KEYRING_SECRET_BYTES + 1
        ]));
        assert!(!keyring_secret_fits(&[]));
    }
}
