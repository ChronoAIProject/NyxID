//! Storage identity for the input retained by a background response window.
use crate::crypto::record_field::historical::HistoricalAad;
use crate::{
    crypto::{aes::EncryptionKeys, record_field::RecordContext},
    errors::{AppError, AppResult},
    models::assistant_conversation::{AssistantConversation, COLLECTION_NAME, ReattachWindow},
    services::{record_storage, transaction_service},
};
use bson::{Binary, Bson, Document, doc, spec::BinarySubtype};
use mongodb::Database;
use zeroize::Zeroizing;

pub(super) const INPUT: &str = "active_turn.restart_admission.window.input";
pub(super) const VERSION: &str = "active_turn.restart_admission.window.input_context_version";

fn invalid() -> AppError {
    AppError::Conflict("Reattach window input could not be verified".into())
}

pub(crate) struct SealedInput {
    pub(crate) ciphertext: Binary,
    pub(crate) version: Option<u8>,
}

pub(super) struct Identity<'a> {
    owner: &'a str,
    conversation: &'a str,
    credential: &'a str,
    key: &'a str,
}

impl<'a> Identity<'a> {
    pub(super) fn of(row: &'a AssistantConversation, key: &'a str) -> Self {
        Self {
            owner: &row.user_id,
            conversation: &row.id,
            credential: &row.credential_api_key_id,
            key,
        }
    }

    fn path(&self) -> [&str; 6] {
        [
            "active_turn",
            "restart_admission",
            "window",
            "input",
            self.credential,
            self.key,
        ]
    }

    fn aad(&self) -> AppResult<HistoricalAad<'_>> {
        HistoricalAad::reattach_window(self.owner, self.conversation, self.credential, self.key)
            .map_err(|_| invalid())
    }

    pub(super) async fn seal(&self, keys: &EncryptionKeys, plain: &[u8]) -> AppResult<SealedInput> {
        let (bytes, version) = if keys.contextual_record_writes() {
            let path = self.path();
            let ctx = RecordContext::new(COLLECTION_NAME, self.conversation, self.owner, &path)?;
            (keys.encrypt_record(plain, &ctx).await?, Some(3))
        } else {
            (self.aad()?.seal(keys, plain).await?, None)
        };
        Ok(SealedInput {
            ciphertext: Binary {
                subtype: BinarySubtype::Generic,
                bytes,
            },
            version,
        })
    }

    pub(super) async fn open(
        &self,
        keys: &EncryptionKeys,
        window: &ReattachWindow,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        let value = &window.input.as_ref().ok_or_else(invalid)?.bytes;
        match window.input_context_version {
            Some(3) => {
                let path = self.path();
                let ctx =
                    RecordContext::new(COLLECTION_NAME, self.conversation, self.owner, &path)?;
                keys.decrypt_record(value, &ctx).await
            }
            None if keys.allow_legacy_record_reads() => self.aad()?.open(keys, value).await,
            _ => Err(invalid()),
        }
    }
}

fn child<'a>(parent: &'a Document, field: &str) -> AppResult<Option<&'a Document>> {
    match parent.get(field) {
        None | Some(Bson::Null) => Ok(None),
        Some(Bson::Document(doc)) => Ok(Some(doc)),
        _ => Err(invalid()),
    }
}

async fn inspect(
    keys: &EncryptionKeys,
    source: &Document,
    rewrap: bool,
) -> AppResult<Option<SealedInput>> {
    let Some(active) = child(source, "active_turn")? else {
        return Ok(None);
    };
    let Some(admission) = child(active, "restart_admission")? else {
        return Ok(None);
    };
    let Some(raw) = child(admission, "window")? else {
        return Ok(None);
    };
    let window: ReattachWindow = bson::from_document(raw.clone()).map_err(|_| invalid())?;
    let Some(input) = &window.input else {
        return if window.input_context_version.is_none() {
            Ok(None)
        } else {
            Err(invalid())
        };
    };
    let credential = admission.get_str("credential_id").map_err(|_| invalid())?;
    let current = source
        .get_str("credential_api_key_id")
        .map_err(|_| invalid())?;
    // A replacement may leave the ended admission on the active turn until
    // settlement. Its retained bytes still belong to the admitted credential.
    if credential != current && admission.get_bool("credential_ended") != Ok(true) {
        return Err(invalid());
    }
    let identity = Identity {
        conversation: source.get_str("_id").map_err(|_| invalid())?,
        owner: source.get_str("user_id").map_err(|_| invalid())?,
        credential,
        key: &window.key,
    };
    let path = identity.path();
    let ctx = RecordContext::new(
        COLLECTION_NAME,
        identity.conversation,
        identity.owner,
        &path,
    )?;
    let primary = window.input_context_version == Some(3)
        && keys
            .authenticate_current_record(&input.bytes, &ctx)
            .await
            .is_ok();
    if !primary && !rewrap {
        return Err(invalid());
    }
    let plain = identity.open(keys, &window).await?;
    validate_plain(&plain)?;
    if primary {
        return Ok(None);
    }
    let replacement = identity.seal(keys, &plain).await?;
    keys.authenticate_current_record(&replacement.ciphertext.bytes, &ctx)
        .await?;
    Ok(Some(replacement))
}

pub(crate) fn validate_plain(plain: &[u8]) -> AppResult<()> {
    serde_json::from_slice::<super::Input>(plain)
        .map(drop)
        .map_err(|_| invalid())
}

pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    inspect(keys, source, false).await.map(|_| ())
}

pub(crate) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(invalid());
    }
    let Some(sealed) = inspect(keys, source, true).await? else {
        return Ok(false);
    };
    Ok(transaction_service::durable_database(db)
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set":{
                INPUT: sealed.ciphertext,
                VERSION: 3_i32,
            }},
        )
        .await?
        .matched_count
        == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reattach_window_historical_aad_keeps_the_json_tuple_bytes() {
        let identity = Identity {
            owner: "person:λ",
            conversation: "chat/%2F",
            credential: "key\"",
            key: "window\\01",
        };
        assert_eq!(
            identity.aad().unwrap().as_bytes(),
            r#"["nyxid/reattach-window/1","person:λ","chat/%2F","key\"","window\\01"]"#.as_bytes()
        );
    }
}
