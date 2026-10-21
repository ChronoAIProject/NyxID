use super::*;
use crate::{
    models::consent::Consent,
    services::{consent_service, oauth_resource_service},
    test_utils,
};

struct Fixture {
    db: mongodb::Database,
    config: AppConfig,
    keys: JwtKeys,
    user: Uuid,
    client: String,
}

impl Fixture {
    async fn new() -> Self {
        let db = test_utils::connect_transaction_test_database("legacy_oauth_refresh").await;
        let user = Uuid::new_v4();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(test_utils::test_user(
                &user.to_string(),
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let client = Uuid::new_v4().to_string();
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client.clone(),
                client_name: "Legacy replica".into(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["https://example.test/callback".into()],
                allowed_scopes: "openid profile proxy".into(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".into(),
                client_type: "public".into(),
                is_active: true,
                delegation_scopes: String::new(),
                default_service_catalog_slugs: vec![],
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        Self {
            db,
            config: test_utils::test_app_config(),
            keys: test_utils::cached_test_jwt_keys(),
            user,
            client,
        }
    }

    async fn grant(&self) -> Consent {
        consent_service::grant_consent_with_services(
            &self.db,
            &self.user.to_string(),
            &self.client,
            "openid proxy",
            Some(vec!["live-service".into()]),
        )
        .await
        .unwrap()
    }

    async fn legacy(&self, created_at: chrono::DateTime<Utc>) -> (String, RefreshToken) {
        let (token, jti) =
            jwt::generate_refresh_token(&self.keys, &self.config, &self.user).unwrap();
        let row = RefreshToken {
            id: Uuid::new_v4().to_string(),
            jti,
            client_id: self.client.clone(),
            user_id: self.user.to_string(),
            session_id: None,
            protected_consent: None,
            scope: Some("openid profile proxy stale-scope".into()),
            resource_uris: vec![],
            allowed_service_ids: vec!["stale-service".into()],
            allow_all_services: true,
            expires_at: Utc::now() + Duration::days(7),
            revoked: false,
            replaced_by: None,
            revoked_at: None,
            created_at,
        };
        self.db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(&row)
            .await
            .unwrap();
        (token, row)
    }

    async fn store_legacy(&self, row: &RefreshToken) {
        self.db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .replace_one(doc! {"_id": &row.id}, row)
            .await
            .unwrap();
    }

    async fn refresh(&self, token: &str) -> AppResult<IssuedTokens> {
        refresh_tokens(&self.db, &self.config, &self.keys, token, None, None).await
    }
}

#[tokio::test]
async fn legacy_refresh_upgrades_once_and_narrows_broader_legacy_authority() {
    let f = Fixture::new().await;
    let mut consent = f.grant().await;
    consent.granted_at = Utc::now() - Duration::minutes(2);
    f.db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .replace_one(doc! {"_id":&consent.id}, &consent)
        .await
        .unwrap();
    let (legacy, row) = f.legacy(Utc::now()).await;
    let issued = f.refresh(&legacy).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
    assert_eq!(claims.scope, consent.scopes);
    assert_eq!(
        claims.protected_consent,
        Some(consent_service::protected_reference(&consent).unwrap())
    );
    assert_eq!(
        claims.allowed_service_ids,
        Some(vec!["live-service".into()])
    );
    assert_eq!(claims.allow_all_services, Some(false));
    assert_eq!(claims.resources, Some(vec![]));
    crate::services::oauth_token_authority_service::validate_claims(&f.db, &claims)
        .await
        .unwrap();
    let new_jti = jwt::verify_token(&f.keys, &f.config, &issued.refresh_token)
        .unwrap()
        .jti;
    let successor =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"jti":new_jti})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(successor.scope.as_deref(), Some("openid proxy"));
    assert_eq!(successor.allowed_service_ids, vec!["live-service"]);
    assert!(successor.resource_uris.is_empty());
    assert_eq!(successor.protected_consent, claims.protected_consent);
    let consumed =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"_id":row.id})
            .await
            .unwrap()
            .unwrap();
    assert!(consumed.revoked);
    assert_eq!(consumed.replaced_by.as_deref(), Some(successor.id.as_str()));
    assert!(consumed.protected_consent.is_none());
    assert!(f.refresh(&legacy).await.is_err());
    assert_eq!(
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! {"revoked":false})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_refresh_can_bind_an_unversioned_live_consent() {
    let f = Fixture::new().await;
    let consent = f.grant().await;
    f.db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&consent.id},
            doc! {
                "$unset":{"revision":""},
                "$set":{"granted_at":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(2))}
            },
        )
        .await
        .unwrap();
    let (token, _) = f.legacy(Utc::now()).await;
    let issued = f.refresh(&token).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
    let reference = claims.protected_consent.as_ref().unwrap();
    assert_eq!(reference.consent_id, consent.id);
    assert!(!reference.consent_revision.is_empty());
    consent_service::validate_protected_reference(
        &f.db,
        &f.user.to_string(),
        &f.client,
        &claims.scope,
        Some(reference),
    )
    .await
    .unwrap();
    assert!(f.refresh(&token).await.is_err());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_refresh_cutoff_rejects_old_ambiguous_expired_and_revoked_records() {
    let f = Fixture::new().await;
    let consent = f.grant().await;
    // Exactly 60 seconds is insufficient, including BSON millisecond precision.
    for offset in [-1, 0, 59, 60] {
        let (token, _) = f
            .legacy(consent.granted_at + Duration::seconds(offset))
            .await;
        assert!(f.refresh(&token).await.is_err(), "offset {offset}");
    }
    for field in ["revoked", "expires_at"] {
        let (token, row) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
        let value = if field == "revoked" {
            bson::Bson::Boolean(true)
        } else {
            bson::Bson::DateTime(bson::DateTime::from_chrono(
                Utc::now() - Duration::seconds(1),
            ))
        };
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(doc! {"_id":row.id}, doc! {"$set":{field:value}})
            .await
            .unwrap();
        assert!(f.refresh(&token).await.is_err(), "{field}");
    }
    assert_eq!(
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! {"protected_consent":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_refresh_predating_regrant_or_incremental_replacement_cannot_upgrade() {
    let f = Fixture::new().await;
    let consent = f.grant().await;
    consent_service::revoke_consent(&f.db, &f.user.to_string(), &f.client)
        .await
        .unwrap();
    let mut regrant = f.grant().await;
    assert_ne!(consent.revision, regrant.revision);
    // Insert after revocation so the revoke write cannot be the rejection gate.
    let (before_regrant, old_row) = f.legacy(consent.granted_at - Duration::seconds(1)).await;
    assert!(old_row.created_at < regrant.granted_at);
    assert!(matches!(
        f.refresh(&before_regrant).await,
        Err(AppError::Conflict(_))
    ));
    let old_row =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"_id": &old_row.id})
            .await
            .unwrap()
            .unwrap();
    assert!(!old_row.revoked);
    assert!(old_row.replaced_by.is_none());

    // Make room for a record newer than the regrant cutoff but older than the
    // upcoming incremental replacement, without relying on wall-clock sleeps.
    regrant.granted_at = Utc::now() - Duration::minutes(3);
    f.db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .replace_one(doc! {"_id": &regrant.id}, &regrant)
        .await
        .unwrap();
    let created_at = regrant.granted_at + Duration::seconds(61);
    let (qualifying, _) = f.legacy(created_at).await;
    let issued = f.refresh(&qualifying).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
    assert_eq!(
        claims.protected_consent,
        Some(consent_service::protected_reference(&regrant).unwrap())
    );
    // Same authority and timestamp as the successful control; keep this row
    // unused until the incremental revision has replaced its consent.
    let (before_increment, increment_row) = f.legacy(created_at).await;
    let incremental = consent_service::replace_incremental_consent(
        &f.db,
        &regrant,
        "openid profile proxy",
        vec!["live-service".into()],
    )
    .await
    .unwrap();
    assert_ne!(regrant.revision, incremental.revision);
    assert!(increment_row.created_at > regrant.granted_at + Duration::seconds(60));
    assert!(increment_row.created_at < incremental.granted_at);
    assert!(matches!(
        f.refresh(&before_increment).await,
        Err(AppError::Conflict(_))
    ));
    let unchanged =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"_id": &increment_row.id})
            .await
            .unwrap()
            .unwrap();
    assert!(!unchanged.revoked);
    assert!(unchanged.replaced_by.is_none());
    // An old replica can issue after that replacement. Only its server-side
    // issuance time, beyond the conservative cutoff, qualifies it for upgrade.
    let (after_regrant, _) = f
        .legacy(incremental.granted_at + Duration::seconds(61))
        .await;
    let issued = f.refresh(&after_regrant).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
    assert_eq!(
        claims.protected_consent,
        Some(consent_service::protected_reference(&incremental).unwrap())
    );
    assert_eq!(claims.scope, incremental.scopes);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_refresh_preserves_narrow_scope_resource_and_service_ids() {
    let f = Fixture::new().await;
    let service = test_utils::test_user_service(
        "live-service",
        &f.user.to_string(),
        "live",
        "endpoint",
        None,
        None,
    );
    f.db.collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    let consent = consent_service::grant_consent_with_services(
        &f.db,
        &f.user.to_string(),
        &f.client,
        "openid profile proxy",
        Some(vec!["other-service".into(), "live-service".into()]),
    )
    .await
    .unwrap();
    let (token, mut row) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
    row.scope = Some("proxy openid".into());
    row.resource_uris = vec![oauth_resource_service::user_service_resource_uri(
        &f.config, "live",
    )];
    row.allowed_service_ids = vec!["live-service".into()];
    row.allow_all_services = false;
    f.store_legacy(&row).await;
    let issued = f.refresh(&token).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
    assert_eq!(claims.scope, "proxy openid");
    assert_eq!(claims.resources, Some(row.resource_uris.clone()));
    assert_eq!(issued.resource_uris, row.resource_uris);
    assert_eq!(
        claims.allowed_service_ids,
        Some(row.allowed_service_ids.clone())
    );
    assert_eq!(claims.allow_all_services, Some(false));
    let jti = jwt::verify_token(&f.keys, &f.config, &issued.refresh_token)
        .unwrap()
        .jti;
    let successor =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"jti":jti})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(successor.scope, row.scope);
    assert_eq!(successor.resource_uris, row.resource_uris);
    assert_eq!(successor.allowed_service_ids, row.allowed_service_ids);
    assert!(!successor.allow_all_services);
    assert_eq!(
        successor.protected_consent,
        Some(consent_service::protected_reference(&consent).unwrap())
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_refresh_intersects_service_policies_and_historical_scope_fallback() {
    for (legacy_all, consent_all, expected_ids) in [
        (true, true, vec![]),
        (true, false, vec!["live-service", "consent-only"]),
        (false, true, vec!["legacy-only", "live-service"]),
        (false, false, vec!["live-service"]),
    ] {
        let f = Fixture::new().await;
        let mut consent = f.grant().await;
        consent.scopes = "proxy email openid new-scope".into();
        consent.allow_all_services = consent_all;
        consent.allowed_service_ids = Some(vec!["live-service".into(), "consent-only".into()]);
        f.db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
            .replace_one(doc! {"_id": &consent.id}, &consent)
            .await
            .unwrap();
        let (token, mut row) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
        row.scope = None;
        row.allow_all_services = legacy_all;
        row.allowed_service_ids = vec!["legacy-only".into(), "live-service".into()];
        f.store_legacy(&row).await;
        let issued = f.refresh(&token).await.unwrap();
        let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
        assert_eq!(claims.scope, "openid email proxy", "legacy fallback order");
        assert_eq!(
            claims.allow_all_services.unwrap_or(true),
            legacy_all && consent_all
        );
        let jti = jwt::verify_token(&f.keys, &f.config, &issued.refresh_token)
            .unwrap()
            .jti;
        let successor =
            f.db.collection::<RefreshToken>(REFRESH_TOKENS)
                .find_one(doc! {"jti":jti})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(successor.allowed_service_ids, expected_ids);
        assert_eq!(successor.scope.as_deref(), Some("openid email proxy"));
        assert_eq!(successor.allow_all_services, legacy_all && consent_all);
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn legacy_refresh_rejects_empty_scope_and_unauthorized_resources_before_any_write() {
    let f = Fixture::new().await;
    for (id, slug) in [("live-service", "live"), ("other-service", "other")] {
        f.db.collection(crate::models::user_service::COLLECTION_NAME)
            .insert_one(test_utils::test_user_service(
                id,
                &f.user.to_string(),
                slug,
                "endpoint",
                None,
                None,
            ))
            .await
            .unwrap();
    }
    let consent = f.grant().await;
    // Detect any premature ensure_issuance_reference write, too.
    f.db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .update_one(doc! {"_id": &consent.id}, doc! {"$unset": {"revision": ""}})
        .await
        .unwrap();
    let consent_before =
        f.db.collection::<bson::Document>(crate::models::consent::COLLECTION_NAME)
            .find_one(doc! {"_id": &consent.id})
            .await
            .unwrap()
            .unwrap();
    for case in [
        "empty-scope",
        "unknown-resource",
        "outside-consent",
        "outside-legacy",
    ] {
        let (token, mut row) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
        match case {
            "empty-scope" => row.scope = Some("profile stale-scope".into()),
            "unknown-resource" => row.resource_uris = vec!["https://obsolete.test/resource".into()],
            "outside-consent" => {
                row.resource_uris = vec![oauth_resource_service::user_service_resource_uri(
                    &f.config, "other",
                )]
            }
            "outside-legacy" => {
                row.allow_all_services = false;
                row.allowed_service_ids = vec!["other-service".into()];
                row.resource_uris = vec![oauth_resource_service::user_service_resource_uri(
                    &f.config, "live",
                )];
            }
            _ => unreachable!(),
        }
        f.store_legacy(&row).await;
        let collection = f.db.collection::<bson::Document>(REFRESH_TOKENS);
        let before = collection
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(f.refresh(&token).await, Err(AppError::Conflict(_))),
            "{case}"
        );
        assert_eq!(
            before,
            collection
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .unwrap(),
            "{case}"
        );
        assert_eq!(
            consent_before,
            f.db.collection::<bson::Document>(crate::models::consent::COLLECTION_NAME)
                .find_one(doc! {"_id": &consent.id})
                .await
                .unwrap()
                .unwrap(),
            "{case}"
        );
    }
    assert_eq!(
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        4
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_null_consent_allowlist_upgrades_identity_only_and_rejects_retained_resources() {
    let f = Fixture::new().await;
    f.db.collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(test_utils::test_user_service(
            "live-service",
            &f.user.to_string(),
            "live",
            "endpoint",
            None,
            None,
        ))
        .await
        .unwrap();
    let mut consent = f.grant().await;
    consent.allowed_service_ids = None;
    consent.allow_all_services = false;
    f.db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .replace_one(doc! {"_id": &consent.id}, &consent)
        .await
        .unwrap();
    let (token, _) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
    let issued = f.refresh(&token).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
    assert_eq!(claims.scope, "openid proxy");
    assert_eq!(claims.allow_all_services, Some(false));
    assert_eq!(claims.allowed_service_ids, Some(vec![]));
    assert_eq!(claims.resources, Some(vec![]));
    crate::services::oauth_token_authority_service::validate_claims(&f.db, &claims)
        .await
        .unwrap();
    let jti = jwt::verify_token(&f.keys, &f.config, &issued.refresh_token)
        .unwrap()
        .jti;
    let successor =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"jti":jti})
            .await
            .unwrap()
            .unwrap();
    assert!(!successor.allow_all_services);
    assert!(successor.allowed_service_ids.is_empty());
    for resource in [
        oauth_resource_service::user_service_resource_uri(&f.config, "live"),
        oauth_resource_service::mcp_resource_uri(&f.config),
    ] {
        let (token, mut row) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
        row.resource_uris = vec![resource];
        f.store_legacy(&row).await;
        let collection = f.db.collection::<bson::Document>(REFRESH_TOKENS);
        let before = collection
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            f.refresh(&token).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            before,
            collection
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .unwrap()
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_refresh_strips_machine_scope_and_cannot_create_machine_authority() {
    let f = Fixture::new().await;
    let mut consent = f.grant().await;
    consent.scopes = "openid proxy machine".into();
    f.db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
        .replace_one(doc! {"_id": &consent.id}, &consent)
        .await
        .unwrap();
    let (token, mut row) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
    row.scope = Some("machine proxy openid".into());
    f.store_legacy(&row).await;
    let issued = f.refresh(&token).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &issued.access_token).unwrap();
    assert_eq!(claims.scope, "proxy openid");
    assert_eq!(
        claims.protected_consent,
        Some(consent_service::protected_reference(&consent).unwrap())
    );
    crate::services::oauth_token_authority_service::validate_claims(&f.db, &claims)
        .await
        .unwrap();
    assert!(matches!(
        crate::services::machine_external_grant_service::VerifiedExternalSource::from_verified_oauth(&claims),
        Err(AppError::MachineNotAllowed)
    ));
    let jti = jwt::verify_token(&f.keys, &f.config, &issued.refresh_token)
        .unwrap()
        .jti;
    let successor =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"jti":jti})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(successor.scope.as_deref(), Some("proxy openid"));
    let (machine_only, mut row) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
    row.scope = Some("machine".into());
    f.store_legacy(&row).await;
    assert!(matches!(
        f.refresh(&machine_only).await,
        Err(AppError::Conflict(_))
    ));
    let unchanged =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
    assert!(!unchanged.revoked);
    assert!(unchanged.replaced_by.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_refresh_without_live_consent_cannot_upgrade() {
    let f = Fixture::new().await;
    let (token, row) = f.legacy(Utc::now()).await;
    assert!(f.refresh(&token).await.is_err());
    let unchanged =
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"_id":row.id})
            .await
            .unwrap()
            .unwrap();
    assert!(!unchanged.revoked);
    assert!(unchanged.protected_consent.is_none());
    assert!(unchanged.replaced_by.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_legacy_refresh_has_one_winner_across_replicas() {
    let f = Fixture::new().await;
    let consent = f.grant().await;
    let (legacy, _) = f.legacy(consent.granted_at + Duration::seconds(61)).await;
    let other = mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap()
        .database(f.db.name());
    let (a, b) = tokio::join!(
        f.refresh(&legacy),
        refresh_tokens(&other, &f.config, &f.keys, &legacy, None, None)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        f.db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! {"revoked":false})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_oauth_access_is_rejected_while_first_party_refresh_still_rotates() {
    let f = Fixture::new().await;
    f.grant().await;
    let access = jwt::generate_oauth_access_token_with_consent(
        &f.keys,
        &f.config,
        &f.user,
        "openid proxy",
        None,
        None,
        None,
        None,
        None,
        &f.client,
        None,
    )
    .unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &access).unwrap();
    assert!(
        crate::services::oauth_token_authority_service::validate_claims(&f.db, &claims)
            .await
            .is_err()
    );
    let issued =
        create_session_and_issue_tokens(&f.db, &f.config, &f.keys, &f.user.to_string(), None, None)
            .await
            .unwrap();
    let refreshed = f.refresh(&issued.refresh_token).await.unwrap();
    let claims = jwt::verify_token(&f.keys, &f.config, &refreshed.access_token).unwrap();
    assert!(claims.client_id.is_none());
    assert!(claims.protected_consent.is_none());
    assert!(claims.sid.is_some());
    assert_ne!(refreshed.refresh_token, issued.refresh_token);
    f.db.drop().await.unwrap();
}
