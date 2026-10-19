//! Known-copy ancestry is preserved independently of randomized ciphertext and
//! changing refresh responses. It detects shared capabilities; it cannot prove
//! that a provider's remote revocation affects only one upstream grant.
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        oauth_grant_lineage::OAuthGrantLineage, user_api_key::UserApiKey,
        user_provider_token::UserProviderToken,
    },
};
use zeroize::Zeroizing;
const VERSION: u8 = 1;

fn invalid() -> AppError {
    AppError::Conflict("OAuth grant provenance requires authenticated repair".into())
}
pub(crate) fn validate(lineage: &OAuthGrantLineage) -> AppResult<()> {
    if lineage.version != VERSION
        || lineage.origins.is_empty()
        || lineage.origins.len() > 4
        || lineage.origins.iter().any(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        || lineage.origins.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn from_material(
    provider: &str,
    access: Option<&str>,
    refresh: Option<&str>,
) -> AppResult<Option<OAuthGrantLineage>> {
    let mut origins = Vec::new();
    for token in [access, refresh]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
    {
        let mut binding = Zeroizing::new(Vec::with_capacity(16 + provider.len() + token.len()));
        binding.extend_from_slice(&(provider.len() as u64).to_be_bytes());
        binding.extend_from_slice(provider.as_bytes());
        binding.extend_from_slice(&(token.len() as u64).to_be_bytes());
        binding.extend_from_slice(token.as_bytes());
        let origin = super::audit_service::keyed_fingerprint(
            super::audit_service::audit_chain_hmac_key(),
            if provider.is_empty() {
                b"oauth-grant-unattributed-origin-v1".as_slice()
            } else {
                b"oauth-grant-origin-v1".as_slice()
            },
            &binding,
        )
        .ok_or_else(|| AppError::Internal("Credential provenance key is unavailable".into()))?;
        origins.push(origin);
    }
    origins.sort();
    origins.dedup();
    if origins.is_empty() {
        return Ok(None);
    }
    let lineage = OAuthGrantLineage {
        version: VERSION,
        origins,
        provider_unattributed: provider.is_empty(),
    };
    validate(&lineage)?;
    Ok(Some(lineage))
}

pub(crate) async fn from_snapshot(
    keys: &EncryptionKeys,
    collection: &str,
    source: &bson::Document,
) -> AppResult<Option<OAuthGrantLineage>> {
    let collection = match collection {
        crate::models::user_provider_token::COLLECTION_NAME => {
            crate::models::user_provider_token::COLLECTION_NAME
        }
        crate::models::user_api_key::COLLECTION_NAME => {
            crate::models::user_api_key::COLLECTION_NAME
        }
        _ => return Err(invalid()),
    };
    if let Some(value) = source
        .get("grant_lineage")
        .filter(|value| **value != bson::Bson::Null)
    {
        let lineage: OAuthGrantLineage = bson::from_bson(value.clone()).map_err(|_| invalid())?;
        validate(&lineage)?;
        return Ok(Some(lineage));
    }
    let oauth = match collection {
        crate::models::user_provider_token::COLLECTION_NAME => {
            source.get_str("token_type").ok() == Some("oauth2")
        }
        crate::models::user_api_key::COLLECTION_NAME => matches!(
            source.get_str("credential_type").ok(),
            Some("oauth2" | "device_code")
        ),
        _ => return Err(invalid()),
    };
    if !oauth {
        return Ok(None);
    }
    let mut tokens: Vec<Zeroizing<String>> = Vec::new();
    for field in ["access_token_encrypted", "refresh_token_encrypted"] {
        let Some(value) = source
            .get(field)
            .filter(|value| **value != bson::Bson::Null)
        else {
            continue;
        };
        let bytes = super::record_field_service::stored_bytes(value)?;
        let plaintext = super::credential_record_service::decrypt_stored_field(
            keys, collection, source, field, &bytes,
        )
        .await?;
        let token = std::str::from_utf8(&plaintext).map_err(|_| invalid())?;
        tokens.push(Zeroizing::new(token.to_owned()));
    }
    if tokens.is_empty() {
        return Ok(None);
    }
    let provider = match source.get("provider_config_id") {
        None | Some(bson::Bson::Null) => "",
        Some(bson::Bson::String(value)) if !value.is_empty() => value.as_str(),
        _ => return Err(invalid()),
    };
    let Some(mut lineage) = from_material(
        provider,
        tokens.first().map(|s| s.as_str()),
        tokens.get(1).map(|s| s.as_str()),
    )?
    else {
        return Ok(None);
    };
    // A legacy copy can already hold different refreshed tokens. Preserve its
    // known source/connection relationship before transfer rotates the handles.
    // These are bounded original links, not a growing list of refresh history.
    let links = if collection == crate::models::user_provider_token::COLLECTION_NAME {
        [
            source.get_str("_id").ok(),
            source.get_str("connection_id").ok(),
        ]
    } else {
        [
            source.get_str("source_id").ok(),
            source.get_str("connection_id").ok(),
        ]
    };
    for link in links.into_iter().flatten().filter(|link| !link.is_empty()) {
        let mut binding = Zeroizing::new(Vec::with_capacity(16 + provider.len() + link.len()));
        binding.extend_from_slice(&(provider.len() as u64).to_be_bytes());
        binding.extend_from_slice(provider.as_bytes());
        binding.extend_from_slice(&(link.len() as u64).to_be_bytes());
        binding.extend_from_slice(link.as_bytes());
        lineage.origins.push(
            super::audit_service::keyed_fingerprint(
                super::audit_service::audit_chain_hmac_key(),
                b"oauth-grant-legacy-link-v1",
                &binding,
            )
            .ok_or_else(|| AppError::Internal("Credential provenance key is unavailable".into()))?,
        );
    }
    lineage.origins.sort();
    lineage.origins.dedup();
    validate(&lineage)?;
    Ok(Some(lineage))
}

pub(crate) async fn for_token(
    keys: &EncryptionKeys,
    row: &UserProviderToken,
) -> AppResult<Option<OAuthGrantLineage>> {
    from_snapshot(
        keys,
        crate::models::user_provider_token::COLLECTION_NAME,
        &bson::to_document(row).map_err(|_| invalid())?,
    )
    .await
}
pub(crate) async fn for_key(
    keys: &EncryptionKeys,
    row: &UserApiKey,
) -> AppResult<Option<OAuthGrantLineage>> {
    from_snapshot(
        keys,
        crate::models::user_api_key::COLLECTION_NAME,
        &bson::to_document(row).map_err(|_| invalid())?,
    )
    .await
}

/// Reconnect within the same connection retains its known ancestry. A fresh
/// independent connection starts from its own response. Never drop known copy
/// origins merely because a provider rotates both token strings.
pub(crate) async fn install_on_key(
    keys: &EncryptionKeys,
    row: &UserApiKey,
    access: &str,
    refresh: Option<&str>,
) -> AppResult<Option<OAuthGrantLineage>> {
    match for_key(keys, row).await? {
        Some(lineage) => Ok(Some(lineage)),
        None => from_material(
            row.provider_config_id.as_deref().unwrap_or(""),
            Some(access),
            refresh,
        ),
    }
}
pub(crate) async fn install_on_token(
    keys: &EncryptionKeys,
    prior: Option<&UserProviderToken>,
    provider: &str,
    access: &str,
    refresh: Option<&str>,
) -> AppResult<Option<OAuthGrantLineage>> {
    if let Some(prior) = prior
        && let Some(lineage) = for_token(keys, prior).await?
    {
        return Ok(Some(lineage));
    }
    from_material(provider, Some(access), refresh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Document, doc};

    #[test]
    fn known_copy_identity_is_provider_bound_redacted_and_distinct_from_grant_isolation() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let first = from_material(
            "provider",
            Some("original-access"),
            Some("original-refresh"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            first,
            from_material(
                "provider",
                Some("original-access"),
                Some("original-refresh")
            )
            .unwrap()
            .unwrap()
        );
        assert_ne!(
            first,
            from_material(
                "provider",
                Some("independent-access"),
                Some("independent-refresh")
            )
            .unwrap()
            .unwrap()
        );
        assert_ne!(
            first,
            from_material(
                "other-provider",
                Some("original-access"),
                Some("original-refresh")
            )
            .unwrap()
            .unwrap()
        );
        let debug = format!("{first:?}");
        assert!(!debug.contains(&first.origins[0]));
        assert!(!debug.contains("original-access"));
    }

    #[test]
    fn unattributed_material_has_explicit_unknown_provenance_without_a_fabricated_provider() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let unknown = from_material("", Some("custom-access"), None)
            .unwrap()
            .unwrap();
        assert!(unknown.provider_unattributed);
        assert_eq!(
            unknown,
            from_material("", Some("custom-access"), None)
                .unwrap()
                .unwrap()
        );
        let attributed = from_material("actual-provider", Some("custom-access"), None)
            .unwrap()
            .unwrap();
        assert!(!attributed.provider_unattributed);
        assert_ne!(unknown.origins, attributed.origins);
        validate(&unknown).unwrap();
    }

    #[tokio::test]
    async fn copied_legacy_origin_survives_real_token_write_transfer_and_strict_rewrap_then_blocks_erasure()
     {
        for legacy_diverged in [false, true] {
            crate::test_utils::init_test_audit_chain_hmac_key();
            let db =
                crate::test_utils::connect_transaction_test_database("grant_lineage_lifecycle")
                    .await;
            super::super::personal_erasure_service::ensure_indexes(&db)
                .await
                .unwrap();
            let keys = crate::test_utils::test_encryption_keys();
            let owners: [String; 3] = std::array::from_fn(|_| uuid::Uuid::new_v4().to_string());
            for owner in &owners {
                db.collection::<crate::models::user::User>("users")
                    .insert_one(crate::test_utils::test_user(
                        owner,
                        crate::models::user::UserType::Person,
                    ))
                    .await
                    .unwrap();
            }
            let provider = uuid::Uuid::new_v4().to_string();
            let token:UserProviderToken=bson::from_document(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":&owners[0],"provider_config_id":&provider,
            "connection_id":uuid::Uuid::new_v4().to_string(),"token_type":"oauth2","status":"active",
            "access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(b"original-access").await.unwrap()},
            "refresh_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(b"original-refresh").await.unwrap()},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).unwrap();
            db.collection::<UserProviderToken>(crate::models::user_provider_token::COLLECTION_NAME)
                .insert_one(&token)
                .await
                .unwrap();
            let original = for_token(&keys, &token).await.unwrap().unwrap();
            let copy = super::super::user_api_key_service::create_api_key_from_provider_token(
                &db,
                &keys,
                &owners[1],
                "Shared copy",
                &provider,
                &token,
            )
            .await
            .unwrap();
            assert_eq!(copy.grant_lineage.as_ref(), Some(&original));
            if legacy_diverged {
                db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME).update_one(doc! {"_id":&copy.id},doc! {"$set":{
                "access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(b"legacy-already-rotated-access").await.unwrap()},
                "refresh_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(b"legacy-already-rotated-refresh").await.unwrap()}
            },"$unset":{"grant_lineage":"","secret_context_versions":""}}).await.unwrap();
            }
            let before = super::super::user_api_key_service::get_api_key(&db, &owners[1], &copy.id)
                .await
                .unwrap();
            let expected = for_key(&keys, &before).await.unwrap().unwrap();
            assert!(
                expected
                    .origins
                    .iter()
                    .any(|origin| original.origins.contains(origin))
            );
            super::super::user_api_key_service::write_oauth_tokens_to_key(
                &db,
                &keys,
                copy.connection_id.as_deref().unwrap(),
                "rotated-copy-access",
                Some("rotated-copy-refresh"),
                None,
                None,
                Some(super::super::oauth_app_source::OAuthAppSource::Platform),
            )
            .await
            .unwrap();
            let live = super::super::user_api_key_service::get_api_key(&db, &owners[1], &copy.id)
                .await
                .unwrap();
            assert_eq!(live.grant_lineage.as_ref(), Some(&expected));
            let mut moved =
                super::super::credential_record_service::rebind_key_owner(&keys, &live, &owners[2])
                    .await
                    .unwrap();
            moved.extend(doc! {"user_id":&owners[2],"connection_id":uuid::Uuid::new_v4().to_string(),"source_id":uuid::Uuid::new_v4().to_string()});
            super::super::service_history::collection::<UserApiKey>(
                &db,
                crate::models::user_api_key::COLLECTION_NAME,
            )
            .update_one(
                doc! {"_id":&copy.id,"user_id":&owners[1]},
                doc! {"$set":moved},
            )
            .await
            .unwrap();
            let source = db
                .collection::<Document>(crate::models::user_provider_token::COLLECTION_NAME)
                .find_one(doc! {"_id":&token.id})
                .await
                .unwrap()
                .unwrap();
            super::super::credential_record_service::migrate_snapshot(
                &db,
                &keys,
                crate::models::user_provider_token::COLLECTION_NAME,
                &source,
            )
            .await
            .unwrap();
            let mut strict = crate::test_utils::test_encryption_keys();
            strict.set_allow_legacy_record_reads(false);
            let current =
                super::super::user_api_key_service::get_api_key(&db, &owners[2], &copy.id)
                    .await
                    .unwrap();
            assert_eq!(
                for_key(&strict, &current).await.unwrap().as_ref(),
                Some(&expected)
            );
            let accepted =
                super::super::personal_erasure_service::accept(&db, &owners[0], &owners[0], None)
                    .await;
            assert!(matches!(accepted, Err(AppError::Conflict(_))));
            for _ in 0..64 {
                if db
                    .collection::<Document>("personal_erasure_preparations")
                    .find_one(doc! {"_id":&owners[0],"status":"ready"})
                    .await
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                super::super::personal_erasure_service::prepare_one_for_test(&db, &strict)
                    .await
                    .unwrap();
            }
            let prepared = db
                .collection::<Document>("personal_erasure_preparations")
                .find_one(doc! {"_id":&owners[0],"status":"ready"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(prepared.get_bool("shared_dependency"), Ok(true));
            assert!(matches!(
                super::super::personal_erasure_service::accept(&db, &owners[0], &owners[0], None)
                    .await,
                Err(AppError::Conflict(_))
            ));
            assert!(
                db.collection::<Document>("users")
                    .find_one(doc! {"_id":&owners[0],"is_active":true})
                    .await
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                db.collection::<Document>("personal_erasure_jobs")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                db.collection::<Document>("personal_erasure_effects")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            db.drop().await.unwrap();
        }
    }
}
