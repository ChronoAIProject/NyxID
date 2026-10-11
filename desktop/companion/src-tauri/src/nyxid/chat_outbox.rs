use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Version;

use super::chat::valid_conversation_id;

const CHAT_OUTBOX_FILE_NAME: &str = "nyxid-chat-outbox.json";
const CHAT_OUTBOX_SCHEMA_VERSION: u8 = 1;
const MAX_CHAT_OUTBOX_BYTES: usize = 16 * 1024;
const MAX_PENDING_OWNERS: usize = 32;
const MAX_OWNER_ID_BYTES: usize = 1024;
const MAX_TURN_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChatOutbox {
    schema_version: u8,
    pending: Vec<PendingChatAdmission>,
}

impl Default for ChatOutbox {
    fn default() -> Self {
        Self {
            schema_version: CHAT_OUTBOX_SCHEMA_VERSION,
            pending: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PendingChatAdmission {
    pub(crate) owner_user_id: String,
    pub(crate) request_id: String,
    pub(crate) conversation_id: Option<String>,
    pub(crate) turn_id: Option<String>,
    pub(crate) created_at: String,
}

impl PendingChatAdmission {
    pub(crate) fn new(
        owner_user_id: String,
        request_id: String,
        conversation_id: Option<String>,
    ) -> Result<Self, ChatOutboxError> {
        let pending = Self {
            owner_user_id,
            request_id,
            conversation_id,
            turn_id: None,
            created_at: Utc::now().to_rfc3339(),
        };
        pending.validate()?;
        Ok(pending)
    }

    fn validate(&self) -> Result<(), ChatOutboxError> {
        if !valid_owner_id(&self.owner_user_id)
            || !valid_request_id(&self.request_id)
            || self
                .conversation_id
                .as_deref()
                .is_some_and(|id| !valid_conversation_id(id))
            || self
                .turn_id
                .as_deref()
                .is_some_and(|id| !valid_bounded_metadata(id, MAX_TURN_ID_BYTES))
            || DateTime::parse_from_rfc3339(&self.created_at).is_err()
        {
            return Err(ChatOutboxError::Invalid);
        }
        if self.turn_id.is_some() && self.conversation_id.is_none() {
            return Err(ChatOutboxError::Invalid);
        }
        Ok(())
    }
}

impl ChatOutbox {
    fn validate(&self) -> Result<(), ChatOutboxError> {
        if self.schema_version != CHAT_OUTBOX_SCHEMA_VERSION {
            return Err(ChatOutboxError::UnsupportedSchema(self.schema_version));
        }
        if self.pending.len() > MAX_PENDING_OWNERS {
            return Err(ChatOutboxError::TooManyOwners);
        }
        let mut owners = HashSet::with_capacity(self.pending.len());
        for pending in &self.pending {
            pending.validate()?;
            if !owners.insert(pending.owner_user_id.as_str()) {
                return Err(ChatOutboxError::DuplicateOwner);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ChatOutboxStore {
    path: PathBuf,
}

impl ChatOutboxStore {
    pub(crate) fn in_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            path: directory.into().join(CHAT_OUTBOX_FILE_NAME),
        }
    }

    #[cfg(test)]
    fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(crate) fn pending_for_owner(
        &self,
        owner_user_id: &str,
    ) -> Result<Option<PendingChatAdmission>, ChatOutboxError> {
        if !valid_owner_id(owner_user_id) {
            return Err(ChatOutboxError::Invalid);
        }
        Ok(self
            .load()?
            .pending
            .into_iter()
            .find(|pending| pending.owner_user_id == owner_user_id))
    }

    pub(crate) fn begin(
        &self,
        pending: PendingChatAdmission,
    ) -> Result<PendingChatAdmission, ChatOutboxError> {
        pending.validate()?;
        let mut outbox = self.load()?;
        if let Some(existing) = outbox
            .pending
            .iter()
            .find(|existing| existing.owner_user_id == pending.owner_user_id)
        {
            if existing.request_id == pending.request_id
                && existing.conversation_id == pending.conversation_id
            {
                return Ok(existing.clone());
            }
            return Err(ChatOutboxError::PendingExists);
        }
        if outbox.pending.len() == MAX_PENDING_OWNERS {
            return Err(ChatOutboxError::TooManyOwners);
        }
        outbox.pending.push(pending.clone());
        self.save(&outbox)?;
        Ok(pending)
    }

    pub(crate) fn record_receipt(
        &self,
        owner_user_id: &str,
        request_id: &str,
        conversation_id: &str,
        turn_id: &str,
    ) -> Result<(), ChatOutboxError> {
        if !valid_owner_id(owner_user_id)
            || !valid_request_id(request_id)
            || !valid_conversation_id(conversation_id)
            || !valid_bounded_metadata(turn_id, MAX_TURN_ID_BYTES)
        {
            return Err(ChatOutboxError::Invalid);
        }
        let mut outbox = self.load()?;
        let pending = outbox
            .pending
            .iter_mut()
            .find(|pending| {
                pending.owner_user_id == owner_user_id && pending.request_id == request_id
            })
            .ok_or(ChatOutboxError::PendingMissing)?;
        if pending
            .conversation_id
            .as_deref()
            .is_some_and(|current| current != conversation_id)
            || pending
                .turn_id
                .as_deref()
                .is_some_and(|current| current != turn_id)
        {
            return Err(ChatOutboxError::ReceiptMismatch);
        }
        pending.conversation_id = Some(conversation_id.to_owned());
        pending.turn_id = Some(turn_id.to_owned());
        self.save(&outbox)
    }

    pub(crate) fn clear(
        &self,
        owner_user_id: &str,
        request_id: &str,
    ) -> Result<bool, ChatOutboxError> {
        if !valid_owner_id(owner_user_id) || !valid_request_id(request_id) {
            return Err(ChatOutboxError::Invalid);
        }
        let mut outbox = self.load()?;
        let original_len = outbox.pending.len();
        outbox.pending.retain(|pending| {
            pending.owner_user_id != owner_user_id || pending.request_id != request_id
        });
        if outbox.pending.len() == original_len {
            return Ok(false);
        }
        self.save(&outbox)?;
        Ok(true)
    }

    fn load(&self) -> Result<ChatOutbox, ChatOutboxError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ChatOutbox::default());
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        if metadata.len() > MAX_CHAT_OUTBOX_BYTES as u64 {
            return Err(ChatOutboxError::TooLarge);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take((MAX_CHAT_OUTBOX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_CHAT_OUTBOX_BYTES {
            return Err(ChatOutboxError::TooLarge);
        }
        let outbox: ChatOutbox = serde_json::from_slice(&bytes)?;
        outbox.validate()?;
        Ok(outbox)
    }

    fn save(&self, outbox: &ChatOutbox) -> Result<(), ChatOutboxError> {
        outbox.validate()?;
        let bytes = serde_json::to_vec_pretty(outbox)?;
        if bytes.len() > MAX_CHAT_OUTBOX_BYTES {
            return Err(ChatOutboxError::TooLarge);
        }
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "chat outbox path has no parent",
            )
        })?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".nyxid-chat-outbox.")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        sync_directory(parent)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub(crate) enum ChatOutboxError {
    #[error("chat outbox storage failed: {0}")]
    Io(#[from] io::Error),
    #[error("chat outbox JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("chat outbox JSON exceeds {MAX_CHAT_OUTBOX_BYTES} bytes")]
    TooLarge,
    #[error("chat outbox schema version {0} is unsupported")]
    UnsupportedSchema(u8),
    #[error("chat outbox metadata is invalid")]
    Invalid,
    #[error("chat outbox contains more than {MAX_PENDING_OWNERS} owners")]
    TooManyOwners,
    #[error("chat outbox contains duplicate owners")]
    DuplicateOwner,
    #[error("this owner already has an unresolved chat admission")]
    PendingExists,
    #[error("the pending chat admission is missing")]
    PendingMissing,
    #[error("the admission receipt does not match persisted metadata")]
    ReceiptMismatch,
}

fn valid_owner_id(value: &str) -> bool {
    valid_bounded_metadata(value, MAX_OWNER_ID_BYTES)
}

fn valid_request_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.get_version() == Some(Version::Random))
}

fn valid_bounded_metadata(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(owner: &str) -> PendingChatAdmission {
        PendingChatAdmission::new(owner.to_owned(), uuid::Uuid::new_v4().to_string(), None).unwrap()
    }

    #[test]
    fn atomically_persists_only_bounded_admission_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let store = ChatOutboxStore::in_directory(directory.path());
        let pending = pending("owner-1");

        store.begin(pending.clone()).unwrap();

        let raw = fs::read_to_string(&store.path).unwrap();
        assert!(raw.contains(&pending.request_id));
        assert!(raw.contains("owner-1"));
        for forbidden in [
            "messageText",
            "accessToken",
            "refreshToken",
            "credential",
            "serviceSlug",
            "provider",
            "请帮我审批",
        ] {
            assert!(!raw.contains(forbidden), "{forbidden}");
        }
        assert!(directory.path().read_dir().unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        assert_eq!(store.pending_for_owner("owner-1").unwrap(), Some(pending));
    }

    #[test]
    fn rejects_oversized_and_unknown_persisted_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CHAT_OUTBOX_FILE_NAME);
        let store = ChatOutboxStore::at_path(&path);
        fs::write(&path, vec![b'x'; MAX_CHAT_OUTBOX_BYTES + 1]).unwrap();
        assert!(matches!(store.load(), Err(ChatOutboxError::TooLarge)));

        fs::write(
            &path,
            br#"{"schemaVersion":1,"pending":[],"messageText":"secret"}"#,
        )
        .unwrap();
        assert!(matches!(store.load(), Err(ChatOutboxError::Json(_))));
    }

    #[test]
    fn keeps_one_independent_pending_admission_per_owner() {
        let directory = tempfile::tempdir().unwrap();
        let store = ChatOutboxStore::in_directory(directory.path());
        let first = pending("owner-1");
        let second = pending("owner-2");
        store.begin(first.clone()).unwrap();
        store.begin(second.clone()).unwrap();

        assert!(matches!(
            store.begin(pending("owner-1")),
            Err(ChatOutboxError::PendingExists)
        ));
        store.clear("owner-2", &second.request_id).unwrap();
        assert_eq!(store.pending_for_owner("owner-1").unwrap(), Some(first));
        assert_eq!(store.pending_for_owner("owner-2").unwrap(), None);
    }

    #[test]
    fn receipt_updates_are_identity_bound_and_request_clear_is_scoped() {
        let directory = tempfile::tempdir().unwrap();
        let store = ChatOutboxStore::in_directory(directory.path());
        let first = pending("owner-1");
        let second = pending("owner-2");
        store.begin(first.clone()).unwrap();
        store.begin(second.clone()).unwrap();
        let conversation = "nyxa-0123456789abcdef0123456789abcdef";
        store
            .record_receipt("owner-1", &first.request_id, conversation, "turn-1")
            .unwrap();

        assert!(store.clear("owner-1", &first.request_id).unwrap());
        assert_eq!(store.pending_for_owner("owner-1").unwrap(), None);
        assert_eq!(store.pending_for_owner("owner-2").unwrap(), Some(second));
    }
}
