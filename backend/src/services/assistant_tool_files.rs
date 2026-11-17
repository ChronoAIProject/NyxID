//! Binary tool results stay encrypted server-side. Model results contain handles only.
use super::{
    assistant_acknowledgement_service::ChatAuthority, assistant_upload_service as uploads,
    attachment_extraction::MAX_BYTES, content_type,
};
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{assistant_attachment::AssistantAttachment, assistant_conversation::TurnAttachment},
};
use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde_json::json;
use uuid::Uuid;

const COLLECTION: &str = crate::models::assistant_attachment::COLLECTION_NAME;
// Leave one MiB for BSON metadata and envelope overhead below MongoDB's limit.
pub(crate) const INLINE_LIMIT: usize = 15 * 1024 * 1024;
pub(crate) const CHUNK: usize = 4 * 1024 * 1024;

pub(crate) fn eligible(chat: Option<&ChatAuthority>) -> Option<&ChatAuthority> {
    chat.filter(|chat| {
        !chat.guest && chat.turn_live && !chat.turn_stopped && chat.turn_id.is_some()
    })
}

pub(crate) fn candidate(status: u16, mime: &str) -> bool {
    (200..300).contains(&status)
        && content_type::is_binary_content_type(mime)
        && !content_type::is_text_content_type(mime)
        && super::mcp_service::tool_image_type(Some(mime)).is_none()
}

pub(crate) fn omitted(reason: &str) -> String {
    json!({"attachment":null,"status":"not_retained","reason":reason}).to_string()
}

/// A short cancellable budget wait; exhaustion preserves the legacy response reader.
pub(crate) async fn reserve() -> Option<uploads::byte_budget::ByteReservation> {
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        uploads::byte_budget::reserve(MAX_BYTES),
    )
    .await
    .ok()
    .and_then(Result::ok)
}

/// A binary MIME alone is not enough: many providers label textual results as
/// application/*. Preserve their original reader unless the bytes are non-UTF-8
/// or Content-Disposition explicitly identifies a file.
pub(crate) fn is_file(status: u16, mime: &str, disposition: Option<&str>, bytes: &[u8]) -> bool {
    candidate(status, mime)
        && (std::str::from_utf8(bytes).is_err()
            || disposition.is_some_and(|value| {
                let mut parts = disposition_parts(value).into_iter();
                parts
                    .next()
                    .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
                    || parts.any(|part| {
                        part.trim().split_once('=').is_some_and(|(key, _)| {
                            key.trim().eq_ignore_ascii_case("filename")
                                || key.trim().eq_ignore_ascii_case("filename*")
                        })
                    })
            }))
}

/// No bytes or lossy strings enter the model-facing result for a retained file.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn response_text(
    db: &Database,
    keys: &EncryptionKeys,
    chat: &ChatAuthority,
    status: u16,
    mime: &str,
    disposition: Option<&str>,
    fallback: &str,
    bytes: &[u8],
) -> Option<String> {
    if !is_file(status, mime, disposition, bytes) {
        return None;
    }
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let name = filename(disposition, fallback);
    Some(match retain(db, keys, chat, &name, mime, bytes).await {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(code = error.error_code(), "Tool file retention unavailable");
            omitted("attachment_storage_or_authority_unavailable")
        }
    })
}

/// RFC 5987 UTF-8 extended names take precedence. Reject malformed encodings;
/// normalize both slash styles and controls with the ordinary upload sanitizer.
pub(crate) fn filename(disposition: Option<&str>, fallback: &str) -> String {
    let mut regular = None;
    let mut extended = None;
    if let Some(value) = disposition {
        for part in disposition_parts(value).into_iter().skip(1) {
            let Some((key, value)) = part.trim().split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            if key.trim().eq_ignore_ascii_case("filename*") {
                if let Some((charset, rest)) = value.split_once('\'')
                    && charset.eq_ignore_ascii_case("utf-8")
                    && let Some((_, encoded)) = rest.split_once('\'')
                    && valid_percent_encoding(encoded)
                    && let Ok(decoded) = urlencoding::decode(encoded)
                {
                    extended = Some(decoded.into_owned());
                }
            } else if key.trim().eq_ignore_ascii_case("filename") {
                regular = Some(value.to_owned());
            }
        }
    }
    uploads::safe_name(
        extended
            .as_deref()
            .or(regular.as_deref())
            .unwrap_or(fallback),
    )
}

fn disposition_parts(value: &str) -> Vec<&str> {
    // Split only outside quoted strings (filenames may contain semicolons).
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quoted {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
        }
        if ch == ';' && !quoted {
            parts.push(&value[start..index]);
            start = index + 1;
        }
    }
    parts.push(&value[start..]);
    parts
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return false;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    true
}

/// Used only under the response reader's reservation. The final DB transaction
/// commits the slot, root and chunks together and fences Stop, deletion and erasure.
pub(crate) async fn retain(
    db: &Database,
    keys: &EncryptionKeys,
    chat: &ChatAuthority,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> AppResult<String> {
    if eligible(Some(chat)).is_none() {
        return Ok(omitted("live_owner_turn_required"));
    }
    if bytes.len() > MAX_BYTES {
        return Ok(omitted("file_exceeds_20_mib"));
    }
    if bytes.is_empty() {
        return Ok(omitted("empty_file"));
    }
    super::assistant_resource_service::require_live(db, chat).await?;
    let mut filter = super::assistant_resource_service::live_filter(chat)?;
    filter.insert(
        format!(
            "active_turn.attachments.{}",
            super::assistant_nyxagent::MAX_TURN_ATTACHMENTS - 1
        ),
        doc! {"$exists":false},
    );
    let conversations =
        db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME);
    if conversations
        .find_one(filter.clone())
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(omitted("turn_attachment_limit_or_turn_ended"));
    }
    let meta = TurnAttachment {
        artifact: Default::default(),
        id: Uuid::new_v4().to_string(),
        origin: "tool".into(),
        label: uploads::safe_name(name),
        // Old image readers select image/* without validating file metadata.
        // Non-text image types outside the legacy raster path are opaque files.
        content_type: if mime.is_empty()
            || content_type::normalize_content_type(mime).starts_with("image/")
        {
            "application/octet-stream".into()
        } else {
            content_type::normalize_content_type(mime)
        },
        size: bytes.len() as i64,
        pages: None,
        chat_delivery: Some(false),
        image_input: None,
        image_input_reason: None,
        image_inspection: Some(false),
    };
    let mut row = AssistantAttachment {
        artifact: Default::default(),
        id: meta.id.clone(),
        origin: "tool".into(),
        user_id: chat.user_id.clone(),
        conversation_id: chat.conversation_id.clone(),
        turn_id: chat
            .turn_id
            .clone()
            .ok_or(AppError::AssistantTurnRequired)?,
        content_type: meta.content_type.clone(),
        size: meta.size,
        chunks: if bytes.len() <= INLINE_LIMIT {
            0
        } else {
            bytes.len().div_ceil(CHUNK)
        },
        data_encrypted: Vec::new(),
        secret_context_versions: Default::default(),
        created_at: Utc::now(),
    };
    row.data_encrypted = super::assistant_record_service::encrypt_image(
        keys,
        &row,
        if row.chunks == 0 { bytes } else { &[] },
    )
    .await?;
    super::credential_record_service::mark_version(
        &mut row.secret_context_versions,
        keys,
        "data_encrypted",
    );
    let mut documents = vec![bson::to_document(&row).map_err(|_| unavailable())?];
    if row.chunks > 0 {
        for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
            let id = Uuid::new_v4().to_string();
            let mut versions = std::collections::BTreeMap::new();
            super::credential_record_service::mark_version(&mut versions, keys, "data_encrypted");
            documents.push(doc! {
                "_id": &id,
                "user_id": &row.user_id,
                "conversation_id": &row.conversation_id,
                "group_id": bson::Bson::Null,
                "parent_attachment_id": &row.id,
                "chunk_index": index as i64,
                "secret_context_versions": bson::to_bson(&versions).map_err(|_| unavailable())?,
                "data_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: super::assistant_record_service::encrypt_chunk(keys, &id, &row.user_id, &row.id, index as i64, chunk).await?,
                },
            });
        }
    }
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<bool> = async {
        super::erasure_admission::fence_in_session(db, &chat.user_id, &mut session).await?;
        let claimed = conversations.update_one(filter, doc!{"$push":{"active_turn.attachments":bson::to_bson(&meta).map_err(|_|unavailable())?}})
            .session(&mut session).await?;
        if claimed.matched_count != 1 { return Ok(false); }
        db.collection::<bson::Document>(COLLECTION).insert_many(documents).session(&mut session).await?;
        Ok(true)
    }.await;
    match result {
        Ok(true) => super::transaction_service::commit(&mut session).await?,
        Ok(false) => {
            session.abort_transaction().await?;
            return Ok(omitted("turn_attachment_limit_or_turn_ended"));
        }
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    }
    Ok(format!(
        "{}\nUse attachment_id with a file upload operation or nyx__attachment_read; never send file bytes or base64.",
        json!({"attachment_id":meta.id,"name":meta.label,"type":meta.content_type,"size":meta.size})
    ))
}

/// Complete legacy roots use the original decryptor without extra reads.
/// Chunk identities and lengths are checked before returning one protected buffer.
pub(crate) async fn payload(
    db: &Database,
    keys: &EncryptionKeys,
    row: &AssistantAttachment,
) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
    let root = super::assistant_record_service::decrypt_image(keys, row).await?;
    if row.chunks == 0 {
        return Ok(root);
    }
    let size = usize::try_from(row.size).map_err(|_| unavailable())?;
    if row.origin != "tool"
        || row.content_type.starts_with("image/")
        || size <= INLINE_LIMIT
        || size > MAX_BYTES
        || row.chunks != size.div_ceil(CHUNK)
        || !root.is_empty()
    {
        return Err(unavailable());
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(size));
    for index in 0..row.chunks {
        let chunk = db
            .collection::<bson::Document>(COLLECTION)
            .find_one(doc! {
                "parent_attachment_id": &row.id, "user_id": &row.user_id,
                "conversation_id": &row.conversation_id, "group_id": bson::Bson::Null,
                "chunk_index": index as i64,
            })
            .await?
            .ok_or_else(unavailable)?;
        let id = chunk.get_str("_id").map_err(|_| unavailable())?;
        let versions = super::record_field_service::stored_versions(&chunk)?;
        let encrypted = super::record_field_service::stored_bytes(
            chunk.get("data_encrypted").ok_or_else(unavailable)?,
        )?;
        let clear = super::assistant_record_service::decrypt_chunk(
            keys,
            id,
            &row.user_id,
            &row.id,
            index as i64,
            versions.get("data_encrypted").copied(),
            &encrypted,
        )
        .await?;
        if clear.len() != (size - index * CHUNK).min(CHUNK) {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&clear);
    }
    Ok(bytes)
}

fn unavailable() -> AppError {
    AppError::NotFound("Attachment not found or expired".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_file_names_decode_extended_and_sanitize_paths_controls_and_bounds() {
        assert_eq!(
            filename(
                Some("attachment; filename=old.pdf; filename*=UTF-8'en'%E6%96%87%E4%BB%B6.pdf"),
                "fallback"
            ),
            "文件.pdf"
        );
        assert_eq!(
            filename(
                Some("attachment; filename=\"../../a\\report\r\n.pdf\""),
                "fallback"
            ),
            "report.pdf"
        );
        assert_eq!(
            filename(
                Some("attachment; filename=good.pdf; filename*=UTF-8''%GG"),
                "fallback"
            ),
            "good.pdf"
        );
        assert_eq!(filename(None, "../../.."), "attachment");
        assert_eq!(filename(None, &"x".repeat(500)).len(), 120);
        assert_eq!(
            filename(Some("attachment; filename=\"semi;colon.pdf\""), "fallback"),
            "semicolon.pdf"
        );
    }
    #[test]
    fn tool_file_utf8_and_disposition_preserve_legacy_text() {
        for mime in [
            "application/x-amz-json-1.0",
            "application/x-amz-json-1.1",
            "application/jwt",
            "application/x-javascript",
            "application/octet-stream",
        ] {
            for bytes in [
                b"{\"text\":true}".as_slice(),
                b"\xef\xbb\xbfhello".as_slice(),
            ] {
                for disposition in [
                    None,
                    Some("inline"),
                    Some("inline; other=\";filename=fake\""),
                ] {
                    assert!(!is_file(200, mime, disposition, bytes));
                }
                for disposition in [
                    "ATTACHMENT",
                    "inline; filename=file",
                    "inline; FILENAME*=UTF-8''file",
                ] {
                    assert!(is_file(200, mime, Some(disposition), bytes));
                }
            }
            assert!(is_file(200, mime, None, b"caf\xe9"));
            assert!(!is_file(400, mime, Some("attachment"), b"caf\xe9"));
        }
        for mime in [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/svg+xml",
            "application/json",
            "",
        ] {
            assert!(!is_file(200, mime, Some("attachment"), b"\xff"));
        }
    }
    #[test]
    fn tool_file_binary_classification_preserves_text_and_json() {
        // Structured text suffixes and legacy tool-image types keep their reader.
        assert!(!candidate(200, "image/svg+xml"));
        for mime in [
            "",
            "application/json",
            "text/plain",
            "application/custom+json",
            "application/custom+xml",
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
        ] {
            assert!(!candidate(200, mime), "{mime}");
        }
        assert!(candidate(200, "application/pdf"));
        assert!(!candidate(200, "application/json"));
        assert!(!candidate(200, "text/plain"));
        assert!(!candidate(404, "application/pdf"));
    }
}
