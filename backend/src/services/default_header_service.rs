//! Bounded local header decoding. This module never resolves credentials,
//! refreshes OAuth, or performs an upstream request. Authority callers invoke it
//! only after policy/ACL admission, then retain the normal post-materialization
//! authority comparison before provider effects.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::default_request_header::{
        self as headers, DefaultRequestHeader, ResolvedDefaultRequestHeader,
        StoredDefaultRequestHeader,
    },
};
use zeroize::Zeroizing;

// Adapter v4 binds management visibility as well as canonical header identity.
// The envelope remains contextual v3. Explicit v3 sensitive-only records retain
// their exact earlier context; no authentication failure retries another format.
const VERSION: u32 = 4;

fn invalid() -> AppError {
    AppError::Internal("Default header storage is invalid or unavailable".into())
}

/// Context is selected by the caller from the authoritative containing record.
/// Header identity uses the canonical HTTP name, never array position or blob metadata.
pub(crate) async fn decode(
    keys: Option<&EncryptionKeys>,
    collection: &str,
    record: &str,
    owner: &str,
    stored: Option<&[StoredDefaultRequestHeader]>,
) -> AppResult<Vec<ResolvedDefaultRequestHeader>> {
    let stored = stored.unwrap_or_default();
    if stored.len() > headers::MAX_DEFAULT_HEADERS {
        return Err(invalid());
    }
    let mut result = Vec::with_capacity(stored.len());
    for row in stored {
        let name = row.name.to_ascii_lowercase();
        let value = match (
            &row.value_encrypted,
            row.value_context_version,
            row.sensitive,
        ) {
            (Some(encrypted), Some(version @ (3 | VERSION)), sensitive)
                if row.value.is_empty() && (version == VERSION || sensitive) =>
            {
                let keys = keys.ok_or_else(invalid)?;
                let visibility = if sensitive { "masked" } else { "public" };
                let fields = [
                    "default_request_headers",
                    name.as_str(),
                    "value",
                    "visibility",
                    visibility,
                ];
                let context = RecordContext::new(
                    collection,
                    record,
                    owner,
                    if version == 3 { &fields[..3] } else { &fields },
                )?;
                let bytes = keys.decrypt_record(encrypted, &context).await?;
                let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
                if text.len() > headers::MAX_HEADER_VALUE_LEN {
                    return Err(invalid());
                }
                Zeroizing::new(text.to_owned())
            }
            (None, None, _) if keys.is_some_and(EncryptionKeys::allow_legacy_record_reads) => {
                row.value.clone()
            }
            _ => return Err(invalid()),
        };
        headers::validate_header_parts(&row.name, value.as_str())?;
        if row.name.trim() != row.name || value.trim() != value.as_str() {
            return Err(invalid());
        }
        result.push(ResolvedDefaultRequestHeader {
            name: row.name.clone(),
            value,
            overridable: row.overridable,
            sensitive: row.sensitive,
        });
    }
    // Stored metadata is untrusted too: reject duplicate/reserved/malformed names.
    // Validation must not silently normalize a current ciphertext into a new identity.
    let mut seen = std::collections::HashSet::new();
    if result
        .iter()
        .any(|header| !seen.insert(header.name.to_ascii_lowercase()))
    {
        return Err(invalid());
    }
    Ok(result)
}

pub(crate) async fn encode(
    keys: Option<&EncryptionKeys>,
    collection: &str,
    record: &str,
    owner: &str,
    input: Vec<DefaultRequestHeader>,
) -> AppResult<Option<Vec<StoredDefaultRequestHeader>>> {
    let resolved = normalize_input(input, false)?;
    encode_resolved(keys, collection, record, owner, resolved).await
}

fn normalize_input(
    input: Vec<DefaultRequestHeader>,
    allow_mask: bool,
) -> AppResult<Vec<ResolvedDefaultRequestHeader>> {
    if input.len() > headers::MAX_DEFAULT_HEADERS {
        return Err(AppError::ValidationError(
            "Too many default headers (maximum 16)".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    input
        .into_iter()
        .map(|input| {
            let original = input.value;
            let value = Zeroizing::new(original.trim().to_owned());
            let name = input.name.trim().to_owned();
            headers::validate_header_parts(
                &name,
                if allow_mask && value.as_str() == headers::REDACTED_PLACEHOLDER {
                    ""
                } else {
                    value.as_str()
                },
            )?;
            if !seen.insert(name.to_ascii_lowercase()) {
                return Err(AppError::ValidationError(
                    "Duplicate default header name".into(),
                ));
            }
            Ok(ResolvedDefaultRequestHeader {
                name,
                value,
                overridable: input.overridable,
                sensitive: input.sensitive,
            })
        })
        .collect()
}

async fn encode_resolved(
    keys: Option<&EncryptionKeys>,
    collection: &str,
    record: &str,
    owner: &str,
    normalized: Vec<ResolvedDefaultRequestHeader>,
) -> AppResult<Option<Vec<StoredDefaultRequestHeader>>> {
    let mut stored = Vec::with_capacity(normalized.len());
    for header in normalized {
        if header.value.as_str() == headers::REDACTED_PLACEHOLDER {
            return Err(AppError::ValidationError(
                "Masked header requires an existing value under the same name and sensitivity"
                    .into(),
            ));
        }
        let name = header.name.to_ascii_lowercase();
        let visibility = if header.sensitive { "masked" } else { "public" };
        let encrypted = keys
            .ok_or_else(invalid)?
            .encrypt_record(
                header.value.as_bytes(),
                &RecordContext::new(
                    collection,
                    record,
                    owner,
                    &[
                        "default_request_headers",
                        &name,
                        "value",
                        "visibility",
                        visibility,
                    ],
                )?,
            )
            .await?;
        stored.push(StoredDefaultRequestHeader {
            name: header.name,
            value: String::new().into(),
            overridable: header.overridable,
            sensitive: header.sensitive,
            value_context_version: Some(VERSION),
            value_encrypted: Some(encrypted),
        });
    }
    Ok((!stored.is_empty()).then_some(stored))
}

/// A masked edit reads the live old value; the caller must CAS the EXACT stored
/// header array (and owner) when installing this prepared replacement.
pub(crate) async fn prepare_update(
    keys: Option<&EncryptionKeys>,
    collection: &str,
    record: &str,
    owner: &str,
    input: Vec<DefaultRequestHeader>,
    existing: Option<&[StoredDefaultRequestHeader]>,
) -> AppResult<Option<Vec<StoredDefaultRequestHeader>>> {
    let mut input = normalize_input(input, true)?;
    for header in &mut input {
        if header.value.as_str() == headers::REDACTED_PLACEHOLDER {
            if !header.sensitive {
                return Err(AppError::ValidationError(
                    "Making a sensitive header public requires an explicit replacement value"
                        .into(),
                ));
            }
            let matches: Vec<_> = existing
                .unwrap_or_default()
                .iter()
                .filter(|old| old.name.eq_ignore_ascii_case(&header.name) && old.sensitive)
                .collect();
            if matches.len() != 1 {
                return Err(AppError::ValidationError(
                    "Masked header rename requires an explicit replacement value".into(),
                ));
            }
            let mut preserved = decode(
                keys,
                collection,
                record,
                owner,
                Some(std::slice::from_ref(matches[0])),
            )
            .await?;
            header.value = preserved.remove(0).value;
        }
    }
    encode_resolved(keys, collection, record, owner, input).await
}

/// Authenticate under the old authoritative owner and bind the same values to
/// the successor. The caller installs this only under the exact source CAS.
pub(crate) async fn rebind(
    keys: Option<&EncryptionKeys>,
    collection: &str,
    record: &str,
    previous_owner: &str,
    next_owner: &str,
    existing: Option<&[StoredDefaultRequestHeader]>,
) -> AppResult<Option<Vec<StoredDefaultRequestHeader>>> {
    let decoded = decode(keys, collection, record, previous_owner, existing).await?;
    encode_resolved(keys, collection, record, next_owner, decoded).await
}

pub(crate) async fn catalog(
    keys: Option<&EncryptionKeys>,
    service: &crate::models::downstream_service::DownstreamService,
) -> AppResult<Vec<ResolvedDefaultRequestHeader>> {
    decode(
        keys,
        crate::models::downstream_service::COLLECTION_NAME,
        &service.id,
        super::ownership_transfer_service::catalog_owner(service),
        service.default_request_headers.as_deref(),
    )
    .await
}

/// Dedicated management DTOs authenticate storage before exposing an explicitly
/// public value. Visibility is part of the current authenticated context, so a
/// stored flag change cannot disclose a formerly masked secret.
pub(crate) async fn management(
    keys: Option<&EncryptionKeys>,
    collection: &str,
    record: &str,
    owner: &str,
    stored: Option<&[StoredDefaultRequestHeader]>,
) -> AppResult<Option<Vec<DefaultRequestHeader>>> {
    let decoded = decode(keys, collection, record, owner, stored).await?;
    let output: Vec<_> = decoded
        .into_iter()
        .map(|header| DefaultRequestHeader {
            name: header.name,
            value: if header.sensitive {
                headers::REDACTED_PLACEHOLDER.to_owned().into()
            } else {
                header.value
            },
            overridable: header.overridable,
            sensitive: header.sensitive,
        })
        .collect();
    Ok((!output.is_empty()).then_some(output))
}

pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    collection: &str,
    source: &bson::Document,
    owner: &str,
) -> AppResult<()> {
    let id = source.get_str("_id").map_err(|_| invalid())?;
    let stored: Option<Vec<StoredDefaultRequestHeader>> = bson::from_bson(
        source
            .get("default_request_headers")
            .cloned()
            .unwrap_or(bson::Bson::Null),
    )
    .map_err(|_| invalid())?;
    decode(Some(keys), collection, id, owner, stored.as_deref()).await?;
    for header in stored.as_deref().unwrap_or_default() {
        if header.value_context_version != Some(VERSION) || !header.value.is_empty() {
            return Err(invalid());
        }
        let name = header.name.to_ascii_lowercase();
        let visibility = if header.sensitive { "masked" } else { "public" };
        keys.authenticate_current_record(
            header.value_encrypted.as_deref().ok_or_else(invalid)?,
            &RecordContext::new(
                collection,
                id,
                owner,
                &[
                    "default_request_headers",
                    &name,
                    "value",
                    "visibility",
                    visibility,
                ],
            )?,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &bson::Document,
    owner: &str,
) -> AppResult<bool> {
    if verify_current_snapshot(keys, collection, source, owner)
        .await
        .is_ok()
    {
        return Ok(false);
    }
    let id = source.get_str("_id").map_err(|_| invalid())?;
    let stored: Option<Vec<StoredDefaultRequestHeader>> = bson::from_bson(
        source
            .get("default_request_headers")
            .cloned()
            .unwrap_or(bson::Bson::Null),
    )
    .map_err(|_| invalid())?;
    let decoded = decode(Some(keys), collection, id, owner, stored.as_deref()).await?;
    let replacement = encode_resolved(Some(keys), collection, id, owner, decoded).await?;
    let db = super::transaction_service::durable_database(db);
    Ok(db.collection::<bson::Document>(collection)
        .update_one(super::record_storage::exact_document_predicate(source),
            bson::doc! {"$set":{"default_request_headers":bson::to_bson(&replacement).map_err(|_|invalid())?}})
        .await?.matched_count == 1)
}

/// Read the exact BSON array for a compare-and-swap. Typed legacy models add
/// absent optional fields during serialization, so reserializing the model is
/// not an exact source snapshot for pre-migration arrays.
pub(crate) async fn source_filter(
    db: &mongodb::Database,
    collection: &str,
    record: &str,
    owner_field: &str,
    owner: &str,
    expected: Option<&[StoredDefaultRequestHeader]>,
) -> AppResult<bson::Document> {
    let mut filter = bson::doc! {"_id":record};
    filter.insert(owner_field, owner);
    if collection == crate::models::downstream_service::COLLECTION_NAME
        && owner_field == "created_by"
    {
        filter.insert("owner_user_id", bson::Bson::Null);
    }
    let raw = db
        .collection::<bson::Document>(collection)
        .find_one(filter.clone())
        .await?
        .ok_or_else(|| AppError::Conflict("Header owner or record changed".into()))?;
    let source = raw
        .get("default_request_headers")
        .cloned()
        .unwrap_or(bson::Bson::Null);
    let decoded: Option<Vec<StoredDefaultRequestHeader>> =
        bson::from_bson(source.clone()).map_err(|_| invalid())?;
    if decoded.as_deref() != expected {
        return Err(AppError::Conflict(
            "Default headers changed; reload and retry".into(),
        ));
    }
    filter.insert("default_request_headers", source);
    Ok(filter)
}

pub(crate) fn validate_edit(
    input: Vec<DefaultRequestHeader>,
    existing: Option<&[StoredDefaultRequestHeader]>,
) -> AppResult<()> {
    let normalized = normalize_input(input, true)?;
    for header in normalized {
        if header.value.as_str() == headers::REDACTED_PLACEHOLDER
            && (!header.sensitive
                || existing
                    .unwrap_or_default()
                    .iter()
                    .filter(|old| old.sensitive && old.name.eq_ignore_ascii_case(&header.name))
                    .count()
                    != 1)
        {
            return Err(AppError::ValidationError(
                "Masked edits require an unchanged sensitive header name".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(name: &str, value: &str, sensitive: bool) -> DefaultRequestHeader {
        DefaultRequestHeader {
            name: name.into(),
            value: value.to_owned().into(),
            sensitive,
            overridable: false,
        }
    }
    #[tokio::test]
    async fn contextual_headers_masked_edits_and_corrupt_replacement() {
        let keys = crate::test_utils::test_encryption_keys();
        let secret = "distinctive-default-header-72a91";
        let stored = encode(
            Some(&keys),
            "user_services",
            "row-a",
            "owner-a",
            vec![input("X-Key", secret, true)],
        )
        .await
        .unwrap()
        .unwrap();
        assert!(stored[0].value.is_empty());
        assert!(!format!("{stored:?}").contains(secret));
        let management = headers::stored_list_for_response(Some(stored.clone())).unwrap();
        let response = serde_json::to_string(&management).unwrap();
        assert!(!response.contains(secret));
        assert!(!response.contains("encrypted"));
        let decoded = decode(
            Some(&keys),
            "user_services",
            "row-a",
            "owner-a",
            Some(&stored),
        )
        .await
        .unwrap();
        assert_eq!(decoded[0].value.as_str(), secret);
        for (record, owner) in [("row-b", "owner-a"), ("row-a", "owner-b")] {
            assert!(
                decode(Some(&keys), "user_services", record, owner, Some(&stored))
                    .await
                    .is_err()
            );
        }
        let renamed_case = prepare_update(
            Some(&keys),
            "user_services",
            "row-a",
            "owner-a",
            vec![input("x-KEY", headers::REDACTED_PLACEHOLDER, true)],
            Some(&stored),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            decode(
                Some(&keys),
                "user_services",
                "row-a",
                "owner-a",
                Some(&renamed_case)
            )
            .await
            .unwrap()[0]
                .value
                .as_str(),
            secret
        );
        for edit in [
            input("X-Renamed", headers::REDACTED_PLACEHOLDER, true),
            input("X-Key", headers::REDACTED_PLACEHOLDER, false),
        ] {
            assert!(matches!(
                prepare_update(
                    Some(&keys),
                    "user_services",
                    "row-a",
                    "owner-a",
                    vec![edit],
                    Some(&stored)
                )
                .await,
                Err(AppError::ValidationError(_))
            ));
        }
        let mut corrupt = stored.clone();
        corrupt[0].value_encrypted.as_mut().unwrap()[0] ^= 1;
        assert!(
            decode(
                Some(&keys),
                "user_services",
                "row-a",
                "owner-a",
                Some(&corrupt)
            )
            .await
            .is_err()
        );
        assert!(
            prepare_update(
                None,
                "user_services",
                "row-a",
                "owner-a",
                vec![],
                Some(&corrupt)
            )
            .await
            .unwrap()
            .is_none()
        );
        let replacement = prepare_update(
            Some(&keys),
            "user_services",
            "row-a",
            "owner-a",
            vec![input("X-Key", "replacement", true)],
            Some(&corrupt),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            decode(
                Some(&keys),
                "user_services",
                "row-a",
                "owner-a",
                Some(&replacement)
            )
            .await
            .unwrap()[0]
                .value
                .as_str(),
            "replacement"
        );
        assert!(
            prepare_update(
                Some(&keys),
                "user_services",
                "row-a",
                "owner-a",
                vec![input("X-Key", headers::REDACTED_PLACEHOLDER, true)],
                Some(&corrupt)
            )
            .await
            .is_err()
        );
        assert!(matches!(
            encode(
                Some(&keys),
                "user_services",
                "row-a",
                "owner-a",
                vec![input("X-Key", headers::REDACTED_PLACEHOLDER, true)]
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        let overflow = (0..17)
            .map(|i| input(&format!("x-{i}"), "value", false))
            .collect();
        assert!(matches!(
            encode(Some(&keys), "user_services", "row-a", "owner-a", overflow).await,
            Err(AppError::ValidationError(_))
        ));
        assert!(
            serde_json::from_value::<DefaultRequestHeader>(
                serde_json::json!({"name":"x-key","value":"value","value_encrypted":[1]})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn exact_array_cas_rejects_concurrent_replacement_and_legacy_strict_cutover() {
        let db = crate::test_utils::connect_transaction_test_database("header_array_cas").await;
        let mut keys = crate::test_utils::test_encryption_keys();
        let legacy = vec![StoredDefaultRequestHeader {
            name: "x-key".into(),
            value: "legacy-distinctive".to_owned().into(),
            sensitive: true,
            overridable: false,
            value_encrypted: None,
            value_context_version: None,
        }];
        db.collection::<bson::Document>("user_services").insert_one(bson::doc! {"_id":"record","user_id":"owner","default_request_headers":[{"name":"x-key","value":"legacy-distinctive","sensitive":true}]}).await.unwrap();
        let filter = source_filter(
            &db,
            "user_services",
            "record",
            "user_id",
            "owner",
            Some(&legacy),
        )
        .await
        .unwrap();
        let current = prepare_update(
            Some(&keys),
            "user_services",
            "record",
            "owner",
            vec![input("X-Key", headers::REDACTED_PLACEHOLDER, true)],
            Some(&legacy),
        )
        .await
        .unwrap();
        db.collection::<bson::Document>("user_services")
            .update_one(
                bson::doc! {"_id":"record"},
                bson::doc! {"$set":{"default_request_headers":bson::to_bson(&current).unwrap()}},
            )
            .await
            .unwrap();
        assert_eq!(
            db.collection::<bson::Document>("user_services")
                .update_one(
                    filter,
                    bson::doc! {"$set":{"default_request_headers":bson::Bson::Null}}
                )
                .await
                .unwrap()
                .matched_count,
            0
        );
        assert!(
            decode(
                Some(&keys),
                "user_services",
                "record",
                "owner",
                Some(&legacy)
            )
            .await
            .is_ok()
        );
        keys.set_allow_legacy_record_reads(false);
        assert!(
            decode(
                Some(&keys),
                "user_services",
                "record",
                "owner",
                Some(&legacy)
            )
            .await
            .is_err()
        );
        assert!(
            decode(
                Some(&keys),
                "user_services",
                "record",
                "owner",
                current.as_deref()
            )
            .await
            .is_ok()
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod cutover_tests {
    use super::*;
    #[tokio::test]
    async fn reader_stage_cannot_emit_sensitive_v3_until_drained_writer_activation() {
        let mut keys = crate::test_utils::test_encryption_keys();
        keys.set_contextual_record_writes(false);
        let input = || {
            vec![DefaultRequestHeader {
                name: "X-Cutover".into(),
                value: "cutover-secret".to_owned().into(),
                sensitive: true,
                overridable: false,
            }]
        };
        assert!(matches!(
            encode(Some(&keys), "user_services", "row", "owner", input()).await,
            Err(AppError::Conflict(_))
        ));
        let mut public = input();
        public[0].sensitive = false;
        assert!(matches!(
            encode(Some(&keys), "user_services", "row", "owner", public).await,
            Err(AppError::Conflict(_))
        ));
        let legacy: Vec<StoredDefaultRequestHeader> = input().into_iter().map(Into::into).collect();
        assert_eq!(
            decode(Some(&keys), "user_services", "row", "owner", Some(&legacy))
                .await
                .unwrap()[0]
                .value
                .as_str(),
            "cutover-secret"
        );
        keys.set_contextual_record_writes(true);
        let stored = encode(Some(&keys), "user_services", "row", "owner", input())
            .await
            .unwrap()
            .unwrap();
        assert!(stored[0].value.is_empty());
        assert_eq!(stored[0].value_context_version, Some(VERSION));
        assert!(
            decode(Some(&keys), "user_services", "row", "owner", Some(&stored))
                .await
                .is_ok()
        );
    }
}

#[cfg(test)]
mod all_values_tests {
    use super::*;
    use crate::models::{user::User, user_endpoint::UserEndpoint, user_service::UserService};

    #[tokio::test]
    async fn omitted_sensitive_unfamiliar_header_is_encrypted_by_live_writer_and_injected_exactly()
    {
        let db =
            crate::test_utils::connect_transaction_test_database("unmarked_default_header").await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let keys = crate::test_utils::test_encryption_keys();
        let owner = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(crate::models::user_endpoint::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user_endpoint(
                &endpoint_id,
                &owner,
                "header fixture",
                "https://example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user_service(
                &service_id,
                &owner,
                "headers",
                &endpoint_id,
                None,
                None,
            ))
            .await
            .unwrap();
        // The actual accepted API DTO omits the optional sensitive flag.
        let input: DefaultRequestHeader = serde_json::from_value(serde_json::json!({"name":"X-Unfamiliar-Authenticator","value":"unmarked-distinctive-secret"})).unwrap();
        assert!(!input.sensitive);
        let edit = Some(vec![input]);
        super::super::user_service_service::update_user_service_with_keys(
            &db,
            Some(&keys),
            &owner,
            &owner,
            &service_id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&edit),
            None,
            None,
        )
        .await
        .unwrap();
        let source = db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .find_one(bson::doc! {"_id":&service_id})
            .await
            .unwrap()
            .unwrap();
        let stored: UserService = bson::from_document(source.clone()).unwrap();
        let header = &stored.default_request_headers.as_ref().unwrap()[0];
        assert!(!header.sensitive);
        assert!(header.value.is_empty());
        assert!(header.value_encrypted.is_some());
        assert!(!format!("{source:?}").contains("unmarked-distinctive-secret"));
        let resolved = decode(
            Some(&keys),
            crate::models::user_service::COLLECTION_NAME,
            &service_id,
            &owner,
            stored.default_request_headers.as_deref(),
        )
        .await
        .unwrap();
        let merged =
            headers::merge_into_header_list(headers::SecretHeaders::default(), &[&resolved]);
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(|headers: axum::http::HeaderMap| async move {
                assert_eq!(
                    headers["x-unfamiliar-authenticator"],
                    "unmarked-distinctive-secret"
                );
                axum::http::StatusCode::NO_CONTENT
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut request = reqwest::Client::new().get(url);
        for (name, value) in &merged {
            let mut header = reqwest::header::HeaderValue::from_str(value).unwrap();
            header.set_sensitive(true);
            request = request.header(name, header);
        }
        assert_eq!(
            request.send().await.unwrap().status(),
            reqwest::StatusCode::NO_CONTENT
        );
        let displayed = management(
            Some(&keys),
            crate::models::user_service::COLLECTION_NAME,
            &service_id,
            &owner,
            stored.default_request_headers.as_deref(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(displayed[0].value.as_str(), "unmarked-distinctive-secret");
        assert!(!displayed[0].sensitive);
        server.abort();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn mixed_public_masked_and_previous_context_migrate_strictly_without_visibility_downgrade()
     {
        let db =
            crate::test_utils::connect_transaction_test_database("header_all_values_migration")
                .await;
        let mut keys = crate::test_utils::test_encryption_keys();
        let old = keys
            .encrypt_record(
                b"old-context-secret",
                &RecordContext::new(
                    "user_services",
                    "row",
                    "owner",
                    &["default_request_headers", "x-prior", "value"],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let source = bson::doc! {"_id":"row","user_id":"owner","default_request_headers":[
            {"name":"X-Public","value":"visible-public"},
            {"name":"X-Masked","value":"legacy-masked","sensitive":true},
            {"name":"X-Prior","value":"","sensitive":true,"value_context_version":3_i32,"value_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:old}}
        ]};
        db.collection::<bson::Document>("user_services")
            .insert_one(&source)
            .await
            .unwrap();
        assert!(
            verify_current_snapshot(&keys, "user_services", &source, "owner")
                .await
                .is_err()
        );
        assert!(
            migrate_snapshot(&db, &keys, "user_services", &source, "owner")
                .await
                .unwrap()
        );
        let current = db
            .collection::<bson::Document>("user_services")
            .find_one(bson::doc! {"_id":"row"})
            .await
            .unwrap()
            .unwrap();
        keys.set_allow_legacy_record_reads(false);
        verify_current_snapshot(&keys, "user_services", &current, "owner")
            .await
            .unwrap();
        assert!(
            !migrate_snapshot(&db, &keys, "user_services", &current, "owner")
                .await
                .unwrap()
        );
        let stored: Vec<StoredDefaultRequestHeader> =
            bson::from_bson(current.get("default_request_headers").unwrap().clone()).unwrap();
        let view = management(Some(&keys), "user_services", "row", "owner", Some(&stored))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(view[0].value.as_str(), "visible-public");
        assert_eq!(view[1].value.as_str(), headers::REDACTED_PLACEHOLDER);
        assert_eq!(view[2].value.as_str(), headers::REDACTED_PLACEHOLDER);
        let mut tampered = stored;
        tampered[1].sensitive = false;
        assert!(
            management(
                Some(&keys),
                "user_services",
                "row",
                "owner",
                Some(&tampered)
            )
            .await
            .is_err()
        );
        assert!(
            verify_current_snapshot(&keys, "user_services", &source, "owner")
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }
}
