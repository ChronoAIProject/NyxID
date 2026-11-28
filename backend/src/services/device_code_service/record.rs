//! One-time delivery to the signer of a node device-code request.
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
        device_code::{COLLECTION_NAME, DeviceCode, DeviceCodeStatus},
        node,
    },
    services::{record_storage, transaction_service},
};

pub(super) const API_KEY: &str = "delivery_api_key_encrypted";
pub(super) const REFRESH: &str = "delivery_refresh_token_encrypted";
pub(super) const VERSION: &str = "delivery_context_version";
pub(crate) const ISSUANCE_MISSING: &str = "node_device_issuance_evidence_missing";

fn invalid() -> AppError {
    AppError::Internal("Node device delivery requires authenticated repair".into())
}

pub(crate) struct Binding<'a> {
    row: &'a DeviceCode,
    actor: &'a str,
    org: Option<&'a str>,
    key: &'a str,
    node: &'a str,
    refresh_hash: &'a str,
    signer: String,
}

impl<'a> Binding<'a> {
    pub(super) fn new(
        row: &'a DeviceCode,
        actor: &'a str,
        org: Option<&'a str>,
        key: &'a str,
        node: &'a str,
        refresh_hash: &'a str,
    ) -> AppResult<Self> {
        if row.device_pubkey.len() != 32 {
            return Err(invalid());
        }
        Ok(Self {
            row,
            actor,
            org,
            key,
            node,
            refresh_hash,
            signer: hex::encode(&row.device_pubkey),
        })
    }

    pub(crate) fn stored(row: &'a DeviceCode) -> AppResult<Self> {
        Self::new(
            row,
            row.approved_by_user_id.as_deref().ok_or_else(invalid)?,
            row.approved_org_id.as_deref(),
            row.issued_api_key_id.as_deref().ok_or_else(invalid)?,
            row.issued_node_id.as_deref().ok_or_else(invalid)?,
            row.refresh_token_hash
                .as_deref()
                .ok_or_else(|| AppError::Conflict(ISSUANCE_MISSING.into()))?,
        )
    }

    pub(crate) fn path(&self, field: &'static str) -> [&str; 8] {
        [
            field,
            if self.org.is_some() {
                "org"
            } else {
                "personal"
            },
            self.actor,
            self.key,
            self.node,
            &self.row.device_code_hash,
            &self.signer,
            self.refresh_hash,
        ]
    }

    pub(crate) fn context<'b>(&'b self, path: &'b [&'b str]) -> AppResult<RecordContext<'b>> {
        RecordContext::new(
            COLLECTION_NAME,
            &self.row.id,
            self.org.unwrap_or(self.actor),
            path,
        )
    }

    pub(super) async fn seal_pair(
        &self,
        keys: &EncryptionKeys,
        key: &str,
        refresh: &str,
    ) -> AppResult<Document> {
        let mut set = Document::new();
        for (field, value) in [(API_KEY, key), (REFRESH, refresh)] {
            let path = self.path(field);
            let bytes =
                record_field::encrypt_context(keys, &self.context(&path)?, value.as_bytes())
                    .await?;
            set.insert(
                field,
                Binary {
                    subtype: BinarySubtype::Generic,
                    bytes,
                },
            );
        }
        if keys.contextual_record_writes() {
            set.insert(VERSION, 3);
        }
        Ok(set)
    }

    async fn open(
        &self,
        keys: &EncryptionKeys,
        field: &'static str,
        value: Option<&[u8]>,
        label: &str,
    ) -> AppResult<Zeroizing<String>> {
        let bytes = value.ok_or_else(|| {
            AppError::Internal(format!("approved device code missing delivery {label}"))
        })?;
        let path = self.path(field);
        let plain = record_field::decrypt_context(
            keys,
            &self.context(&path)?,
            self.row.delivery_context_version,
            bytes,
        )
        .await?;
        crate::crypto::guarded_utf8(plain).map_err(|_| {
            AppError::Internal(format!(
                "approved device code delivery {label} is not valid UTF-8"
            ))
        })
    }
}

pub(super) async fn open_pair(
    keys: &EncryptionKeys,
    row: &DeviceCode,
) -> AppResult<(Zeroizing<String>, Zeroizing<String>)> {
    let binding = Binding::stored(row)?;
    let key = binding
        .open(
            keys,
            API_KEY,
            row.delivery_api_key_encrypted.as_deref(),
            "api key",
        )
        .await?;
    let refresh = binding
        .open(
            keys,
            REFRESH,
            row.delivery_refresh_token_encrypted.as_deref(),
            "refresh token",
        )
        .await?;
    Ok((key, refresh))
}

pub(crate) fn retained(source: &Document) -> AppResult<DeviceCode> {
    let row: DeviceCode = bson::from_document(source.clone()).map_err(|_| invalid())?;
    let key = row.delivery_api_key_encrypted.is_some();
    let refresh = row.delivery_refresh_token_encrypted.is_some();
    use DeviceCodeStatus::*;
    if row
        .delivery_context_version
        .is_some_and(|version| version != 3)
        || key != refresh
        || (key && !matches!(row.status, Approved | Expired))
        || (!key
            && (row.status == Approved
                || (row.delivery_context_version.is_some()
                    && !matches!(row.status, Delivered | Expired))))
    {
        return Err(invalid());
    }
    Ok(row)
}

async fn verify_pair(keys: &EncryptionKeys, row: &DeviceCode) -> AppResult<()> {
    if row.delivery_context_version != Some(3) {
        return Err(invalid());
    }
    let binding = Binding::stored(row)?;
    for (field, value) in [
        (API_KEY, row.delivery_api_key_encrypted.as_deref()),
        (REFRESH, row.delivery_refresh_token_encrypted.as_deref()),
    ] {
        let path = binding.path(field);
        keys.authenticate_current_record(value.ok_or_else(invalid)?, &binding.context(&path)?)
            .await?;
    }
    Ok(())
}

pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    let row = retained(source)?;
    if row.delivery_api_key_encrypted.is_none() && row.delivery_context_version.is_none() {
        return Ok(());
    }
    verify_pair(keys, &row).await
}

async fn check_issuance(
    db: &Database,
    binding: &Binding<'_>,
    key: &str,
    refresh: &str,
) -> AppResult<()> {
    let owner = binding.org.unwrap_or(binding.actor);
    let missing = || AppError::Conflict(ISSUANCE_MISSING.into());
    let issued = db
        .collection::<Document>(api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": binding.key})
        .projection(doc! {"user_id": 1, "key_hash": 1, "platform": 1, "allowed_node_ids": 1})
        .await?
        .ok_or_else(missing)?;
    let node = db
        .collection::<Document>(node::COLLECTION_NAME)
        .find_one(doc! {"_id": binding.node})
        .projection(doc! {"user_id": 1, "metadata.provisioning_source": 1})
        .await?
        .ok_or_else(missing)?;
    if issued.get_str("key_hash").is_err() {
        return Err(missing());
    }
    let node_ids = issued
        .get_array("allowed_node_ids")
        .map_err(|_| invalid())?;
    if issued.get_str("key_hash") != Ok(hash_token(key).as_str())
        || binding.refresh_hash != hash_token(refresh)
        || issued.get_str("user_id") != Ok(owner)
        || issued.get_str("platform") != Ok("device-code")
        || !matches!(node_ids.as_slice(), [id] if id.as_str() == Some(binding.node))
        || node.get_str("user_id") != Ok(owner)
        || node
            .get_document("metadata")
            .and_then(|meta| meta.get_str("provisioning_source"))
            != Ok(crate::services::node_service::DEVICE_CODE_PROVISIONING_SOURCE)
    {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    let row = retained(source)?;
    if row.delivery_api_key_encrypted.is_none() && row.delivery_context_version.is_none() {
        return Ok(false);
    }
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let update = if row.delivery_api_key_encrypted.is_none() {
        // A legacy poller clears both fields before it tries to open a contextual pair.
        doc! {"$unset": {VERSION: ""}}
    } else {
        if row.delivery_context_version == Some(3) && verify_pair(keys, &row).await.is_ok() {
            return Ok(false);
        }
        let binding = Binding::stored(&row)?;
        let (key, refresh) = open_pair(keys, &row).await?;
        if row.delivery_context_version.is_none() {
            check_issuance(db, &binding, &key, &refresh).await?;
        }
        let set = binding.seal_pair(keys, &key, &refresh).await?;
        let mut candidate = source.clone();
        candidate.extend(set.clone());
        verify(keys, &candidate).await?;
        doc! {"$set": set}
    };
    Ok(transaction_service::durable_database(db)
        .collection::<Document>(COLLECTION_NAME)
        .update_one(record_storage::exact_document_predicate(source), update)
        .await?
        .matched_count
        == 1)
}
