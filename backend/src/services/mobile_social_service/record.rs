//! Mobile sign-in delivery storage. Only redemption claims a delivery or installs its session.
use super::{MobileSocialDelivery, PendingDelivery, invalid_code, valid_challenge};
use crate::crypto::record_field::historical::HistoricalAad;
use crate::{
    crypto::{aes::EncryptionKeys, record_field::RecordContext},
    errors::{AppError, AppResult},
    services::{record_storage, transaction_service},
};
use bson::{Binary, Document, doc, spec::BinarySubtype};
use mongodb::Database;
use zeroize::Zeroizing;

const COLLECTION: &str = MobileSocialDelivery::COLLECTION_NAME;

pub(in crate::services) struct Binding<'a> {
    id: &'a str,
    user: &'a str,
    path: [&'a str; 3],
}

impl<'a> Binding<'a> {
    pub(in crate::services) fn new(
        id: &'a str,
        user: &'a str,
        hash: &'a str,
        challenge: &'a str,
    ) -> Self {
        Self {
            id,
            user,
            path: ["ciphertext", hash, challenge],
        }
    }

    pub(in crate::services) fn context(&self) -> AppResult<RecordContext<'_>> {
        RecordContext::new(COLLECTION, self.id, self.user, &self.path)
    }

    pub(in crate::services) fn legacy_aad(&self) -> HistoricalAad<'_> {
        let [_, hash, challenge] = self.path;
        HistoricalAad::mobile_delivery(hash, challenge, self.user)
    }

    pub(super) async fn seal(&self, keys: &EncryptionKeys, payload: &[u8]) -> AppResult<Binary> {
        let bytes = if keys.contextual_record_writes() {
            keys.encrypt_record(payload, &self.context()?).await?
        } else {
            self.legacy_aad().seal(keys, payload).await?
        };
        Ok(Binary {
            subtype: BinarySubtype::Generic,
            bytes,
        })
    }

    pub(super) async fn open(
        &self,
        keys: &EncryptionKeys,
        row: &MobileSocialDelivery,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        match row.ciphertext_context_version {
            Some(3) => {
                keys.decrypt_record(&row.ciphertext.bytes, &self.context()?)
                    .await
            }
            None if keys.allow_legacy_record_reads() => {
                self.legacy_aad().open(keys, &row.ciphertext.bytes).await
            }
            _ => Err(invalid_code()),
        }
    }
}

pub(in crate::services) fn row(source: &Document) -> AppResult<MobileSocialDelivery> {
    let row: MobileSocialDelivery =
        bson::from_document(source.clone()).map_err(|_| invalid_code())?;
    if !uuid::Uuid::parse_str(&row.id).is_ok_and(|id| id.get_version_num() == 4)
        || uuid::Uuid::parse_str(&row.user_id).is_err()
        || row.code_hash.len() != 64
        || !row.code_hash.bytes().all(|c| c.is_ascii_hexdigit())
        || !valid_challenge(&row.code_challenge)
        || row.ciphertext.subtype != BinarySubtype::Generic
    {
        return Err(invalid_code());
    }
    Ok(row)
}

pub(in crate::services) fn validate_plain(plain: &[u8], owner: &str) -> AppResult<()> {
    PendingDelivery::decode(plain, owner).map(drop)
}

async fn inspect(
    keys: &EncryptionKeys,
    source: &Document,
    rewrap: bool,
) -> AppResult<Option<Binary>> {
    let row = row(source)?;
    let binding = Binding::new(&row.id, &row.user_id, &row.code_hash, &row.code_challenge);
    let context = binding.context()?;
    let primary = row.ciphertext_context_version == Some(3)
        && keys
            .authenticate_current_record(&row.ciphertext.bytes, &context)
            .await
            .is_ok();
    if !primary && !rewrap {
        return Err(invalid_code());
    }
    let plain = binding.open(keys, &row).await?;
    validate_plain(&plain, &row.user_id)?;
    if primary {
        return Ok(None);
    }
    let sealed = binding.seal(keys, &plain).await?;
    keys.authenticate_current_record(&sealed.bytes, &context)
        .await?;
    Ok(Some(sealed))
}

pub(in crate::services) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    inspect(keys, source, false).await.map(drop)
}

pub(in crate::services) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let Some(ciphertext) = inspect(keys, source, true).await? else {
        return Ok(false);
    };
    Ok(transaction_service::durable_database(db)
        .collection::<Document>(COLLECTION)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set": {
                "ciphertext": ciphertext,
                "ciphertext_context_version": 3_i32,
            }},
        )
        .await?
        .matched_count
        == 1)
}
