//! Storage for the Discord capability issued with an authenticated native receipt.
use super::{DiscordReplySecret, NativeSendAuthority, binding_digest, refused};
use crate::crypto::record_field::historical::HistoricalAad;
use crate::{
    crypto::{aes::EncryptionKeys, record_field::RecordContext},
    errors::{AppError, AppResult},
    models::{
        platform_entry_identity::{NATIVE_RECEIPTS, NativeControlSurface, NativeEntryReceipt},
        platform_native_reply_handle::{COLLECTION_NAME, NativeReplyHandle},
    },
    services::{
        platform_entry_native::discord_interaction_time, record_storage, transaction_service,
    },
};
use bson::{Binary, Document, doc, spec::BinarySubtype};
use chrono::{DateTime, Duration, Utc};
use mongodb::Database;
use zeroize::Zeroizing;

pub(crate) const RECEIPT_MISSING: &str = "native_reply_parent_receipt_missing";

struct Binding<'a> {
    receipt: &'a NativeEntryReceipt,
    digest: String,
    created: String,
    expires: String,
}

impl<'a> Binding<'a> {
    fn new(
        receipt: &'a NativeEntryReceipt,
        created: DateTime<Utc>,
        expires: DateTime<Utc>,
    ) -> AppResult<Self> {
        let digest = binding_digest(receipt)?;
        let interaction = discord_interaction_time(&receipt.event_id)?;
        if !matches!(
            receipt.configuration.control_surface,
            NativeControlSurface::DiscordCommand { .. }
        ) || receipt.event_id != receipt.message_id
            || receipt.content_commitment.len() != 64
            || !receipt
                .content_commitment
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || created < receipt.received_at
            || created > Utc::now()
            || expires < created
            || expires > receipt.expires_at
            || expires > interaction + Duration::minutes(14)
        {
            return Err(refused());
        }
        Ok(Self {
            receipt,
            digest,
            created: created.timestamp_millis().to_string(),
            expires: expires.timestamp_millis().to_string(),
        })
    }

    fn stored(handle: &NativeReplyHandle, receipt: &'a NativeEntryReceipt) -> AppResult<Self> {
        let binding = Self::new(receipt, handle.created_at, handle.expires_at)?;
        if !uuid::Uuid::parse_str(&handle.id).is_ok_and(|id| id.get_version_num() == 4)
            || handle.receipt_id != receipt.id
            || handle.binding_digest != binding.digest
            || handle.ciphertext.is_empty()
            || handle.ciphertext.len() > 16 * 1024
        {
            return Err(refused());
        }
        Ok(binding)
    }

    fn historical(&self) -> AppResult<HistoricalAad<'_>> {
        HistoricalAad::native_reply(&self.digest).ok_or_else(refused)
    }

    fn path(&self) -> [&str; 6] {
        [
            "ciphertext",
            &self.receipt.profile_id,
            &self.receipt.id,
            &self.digest,
            &self.created,
            &self.expires,
        ]
    }

    async fn encrypt(&self, keys: &EncryptionKeys, id: &str, plain: &[u8]) -> AppResult<Vec<u8>> {
        if keys.contextual_record_writes() {
            let parts = self.path();
            let context = RecordContext::new(COLLECTION_NAME, id, "platform", &parts)?;
            keys.encrypt_record(plain, &context).await
        } else {
            self.historical()?.seal(keys, plain).await
        }
    }

    async fn decrypt(
        &self,
        keys: &EncryptionKeys,
        handle: &NativeReplyHandle,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        match handle.ciphertext_context_version {
            Some(3) => {
                let parts = self.path();
                let context = RecordContext::new(COLLECTION_NAME, &handle.id, "platform", &parts)?;
                keys.decrypt_record(&handle.ciphertext, &context).await
            }
            None if keys.allow_legacy_record_reads() => {
                self.historical()?.open(keys, &handle.ciphertext).await
            }
            _ => Err(refused()),
        }
    }

    async fn verify_current(
        &self,
        keys: &EncryptionKeys,
        handle: &NativeReplyHandle,
    ) -> AppResult<()> {
        if handle.ciphertext_context_version != Some(3) {
            return Err(refused());
        }
        let parts = self.path();
        let context = RecordContext::new(COLLECTION_NAME, &handle.id, "platform", &parts)?;
        keys.authenticate_current_record(&handle.ciphertext, &context)
            .await
    }

    fn material(&self, plain: &[u8]) -> AppResult<DiscordReplySecret> {
        let NativeControlSurface::DiscordCommand { application_id, .. } =
            &self.receipt.configuration.control_surface
        else {
            return Err(refused());
        };
        if plain.len() > 4096 {
            return Err(refused());
        }
        let secret: DiscordReplySecret = serde_json::from_slice(plain).map_err(|_| refused())?;
        if secret.application_id != *application_id
            || !application_id.bytes().all(|b| b.is_ascii_digit())
            || secret.token.is_empty()
            || secret.token.len() > 2048
            || !secret
                .token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(refused());
        }
        Ok(secret)
    }
}

pub(super) async fn open_claimed(
    keys: &EncryptionKeys,
    authority: &NativeSendAuthority,
) -> AppResult<DiscordReplySecret> {
    let handle = authority.handle.as_ref().ok_or_else(refused)?;
    let binding = live(handle, &authority.receipt)?;
    binding.material(&binding.decrypt(keys, handle).await?)
}

fn live<'a>(handle: &NativeReplyHandle, receipt: &'a NativeEntryReceipt) -> AppResult<Binding<'a>> {
    if handle.expires_at <= Utc::now() {
        return Err(refused());
    }
    Binding::stored(handle, receipt)
}

pub(super) fn check_live(
    handle: &NativeReplyHandle,
    receipt: &NativeEntryReceipt,
) -> AppResult<()> {
    live(handle, receipt).map(drop)
}

pub(in crate::services) async fn seal(
    keys: &EncryptionKeys,
    receipt: &NativeEntryReceipt,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    plain: &[u8],
) -> AppResult<NativeReplyHandle> {
    let binding = Binding::new(receipt, created_at, expires_at)?;
    let id = uuid::Uuid::new_v4().to_string();
    let ciphertext = binding.encrypt(keys, &id, plain).await?;
    Ok(NativeReplyHandle {
        id,
        receipt_id: receipt.id.clone(),
        binding_digest: binding.digest,
        ciphertext,
        ciphertext_context_version: keys.contextual_record_writes().then_some(3),
        created_at,
        expires_at,
    })
}

async fn parent(
    db: &Database,
    row: &NativeReplyHandle,
) -> AppResult<(Document, NativeEntryReceipt)> {
    let source = db
        .collection::<Document>(NATIVE_RECEIPTS)
        .find_one(doc! {"_id": &row.receipt_id})
        .await?
        .ok_or_else(|| AppError::Conflict(RECEIPT_MISSING.into()))?;
    let receipt = bson::from_document(source.clone()).map_err(|_| refused())?;
    Ok((source, receipt))
}

pub(crate) async fn verify(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row: NativeReplyHandle = bson::from_document(source.clone()).map_err(|_| refused())?;
    let (_, receipt) = parent(db, &row).await?;
    let binding = Binding::stored(&row, &receipt)?;
    binding.verify_current(keys, &row).await?;
    binding
        .material(&binding.decrypt(keys, &row).await?)
        .map(drop)
}

pub(crate) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let row: NativeReplyHandle = bson::from_document(source.clone()).map_err(|_| refused())?;
    let (parent_source, receipt) = parent(db, &row).await?;
    let binding = Binding::stored(&row, &receipt)?;
    let plain = binding.decrypt(keys, &row).await?;
    binding.material(&plain)?;
    if binding.verify_current(keys, &row).await.is_ok() {
        return Ok(false);
    }
    let bytes = binding.encrypt(keys, &row.id, &plain).await?;
    let parts = binding.path();
    keys.authenticate_current_record(
        &bytes,
        &RecordContext::new(COLLECTION_NAME, &row.id, "platform", &parts)?,
    )
    .await?;
    let update = doc! {"$set": {
        "ciphertext": Binary { subtype: BinarySubtype::Generic, bytes },
        "ciphertext_context_version": 3_i32,
    }};
    let args = (
        db.collection::<Document>(NATIVE_RECEIPTS),
        record_storage::exact_document_predicate(&parent_source),
        db.collection::<Document>(COLLECTION_NAME),
        record_storage::exact_document_predicate(source),
        update,
    );
    // Both observations must still coexist after KMS returns. The independent TTL
    // deletions can run on either side of this snapshot. Neither row is upserted.
    transaction_service::run_db_only(
        db,
        &args,
        |tx, (receipts, parent, handles, handle, update)| {
            Box::pin(async move {
                if receipts
                    .find_one(parent.clone())
                    .projection(doc! {"_id":1})
                    .session(&mut *tx)
                    .await?
                    .is_none()
                {
                    return Ok(false);
                }
                Ok(handles
                    .update_one(handle.clone(), update.clone())
                    .session(tx)
                    .await?
                    .matched_count
                    == 1)
            })
        },
    )
    .await
}
