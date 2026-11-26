//! Retained setup credentials. Serving paths authenticate the live provider
//! key and use binding metadata. Only the storage adapter opens these fields.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bson::{Binary, Document, doc, spec::BinarySubtype};
use mongodb::Database;
use zeroize::Zeroizing;

use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
        token::hash_token,
    },
    errors::{AppError, AppResult},
    models::{
        api_key,
        nyxbot_event_binding::{COLLECTION_NAME, NyxbotEventBinding},
    },
    services::{record_storage, transaction_service},
};

const PROVIDER: &str = "provider_key_ciphertext";
const PRODUCER: &str = "producer_secret_ciphertext";

#[derive(Clone, Copy)]
enum Purpose {
    Provider,
    Producer,
}

impl Purpose {
    fn name(self) -> &'static str {
        match self {
            Self::Provider => PROVIDER,
            Self::Producer => PRODUCER,
        }
    }

    fn path(self, row: &NyxbotEventBinding) -> AppResult<[&str; 4]> {
        let key = match self {
            Self::Provider => row.provider_api_key_id.as_deref().ok_or_else(invalid)?,
            Self::Producer => &row.producer_key_id,
        };
        Ok([self.name(), key, &row.agent_id, &row.agent_owner_id])
    }

    fn bytes(self, row: &NyxbotEventBinding) -> Option<&[u8]> {
        match self {
            Self::Provider => row.provider_key_ciphertext.as_deref(),
            Self::Producer => row.producer_secret_ciphertext.as_deref(),
        }
    }
}

fn invalid() -> AppError {
    AppError::Internal("Invalid event binding credential".into())
}

pub(super) async fn install(
    keys: &EncryptionKeys,
    row: &mut NyxbotEventBinding,
    provider: &str,
    producer: &str,
) -> AppResult<()> {
    for (purpose, secret) in [(Purpose::Provider, provider), (Purpose::Producer, producer)] {
        let path = purpose.path(row)?;
        let context = RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &path)?;
        let bytes = record_field::encrypt_context(keys, &context, secret.as_bytes()).await?;
        match purpose {
            Purpose::Provider => row.provider_key_ciphertext = Some(bytes),
            Purpose::Producer => row.producer_secret_ciphertext = Some(bytes),
        }
        if keys.contextual_record_writes() {
            row.secret_context_versions.insert(purpose.name().into(), 3);
        }
    }
    Ok(())
}

fn snapshot(source: &Document) -> AppResult<NyxbotEventBinding> {
    let row: NyxbotEventBinding = bson::from_document(source.clone()).map_err(|_| invalid())?;
    let live = match row.status.as_str() {
        "preparing" | "ready" if row.revoked_at.is_none() => true,
        "failed" | "revoked" => false,
        _ => return Err(invalid()),
    };
    if row.provider_key_ciphertext.is_some() != live
        || row.producer_secret_ciphertext.is_some() != live
        || row.secret_context_versions.iter().any(|(field, version)| {
            !live || *version != 3 || !matches!(field.as_str(), PROVIDER | PRODUCER)
        })
    {
        return Err(invalid());
    }
    Ok(row)
}

async fn validate_plaintext(
    db: &Database,
    row: &NyxbotEventBinding,
    purpose: Purpose,
    bytes: &[u8],
) -> AppResult<()> {
    match purpose {
        Purpose::Provider => {
            let key = std::str::from_utf8(bytes).map_err(|_| invalid())?;
            let found = db.collection::<Document>(api_key::COLLECTION_NAME)
                .find_one(doc! {"_id":&row.provider_api_key_id,"user_id":&row.user_id,"key_hash":hash_token(key)})
                .projection(doc! {"_id":1}).await?;
            if key.is_empty() || found.is_none() {
                return Err(invalid());
            }
        }
        Purpose::Producer => {
            // Creation stores the standard padded base64 encoding of 32 random bytes.
            let mut decoded = Zeroizing::new([0u8; 32]);
            if bytes.len() != 44 || STANDARD.decode_slice(bytes, decoded.as_mut()).ok() != Some(32)
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

async fn check(
    db: &Database,
    keys: &EncryptionKeys,
    row: &NyxbotEventBinding,
    rewrap: bool,
) -> AppResult<Document> {
    let mut set = Document::new();
    for purpose in [Purpose::Provider, Purpose::Producer] {
        let Some(bytes) = purpose.bytes(row) else {
            continue;
        };
        let field = purpose.name();
        let path = purpose.path(row)?;
        let context = RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &path)?;
        let marker = row.secret_context_versions.get(field).copied();
        let current = marker == Some(3)
            && keys
                .authenticate_current_record(bytes, &context)
                .await
                .is_ok();
        if !current && !rewrap {
            return Err(invalid());
        }
        let secret = record_field::decrypt_context(keys, &context, marker, bytes).await?;
        validate_plaintext(db, row, purpose, &secret).await?;
        if !current {
            let bytes = keys.encrypt_record(&secret, &context).await?;
            keys.authenticate_current_record(&bytes, &context).await?;
            set.insert(
                field,
                Binary {
                    subtype: BinarySubtype::Generic,
                    bytes,
                },
            );
            set.insert(format!("secret_context_versions.{field}"), 3);
        }
    }
    Ok(set)
}

pub(crate) async fn verify(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    check(db, keys, &snapshot(source)?, false).await.map(drop)
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
    let set = check(db, keys, &snapshot(source)?, true).await?;
    if set.is_empty() {
        return Ok(false);
    }
    let result = transaction_service::durable_database(db)
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set":set},
        )
        .await?;
    Ok(result.matched_count == 1)
}
