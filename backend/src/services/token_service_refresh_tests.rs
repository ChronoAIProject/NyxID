use super::*;
use crate::test_utils::*;
use axum::response::IntoResponse;
use futures::TryStreamExt;

pub(crate) async fn fixture() -> (mongodb::Database, AppConfig, JwtKeys, IssuedTokens) {
    let db = connect_transaction_test_database("refresh_1992").await;
    let config = test_app_config();
    let keys = cached_test_jwt_keys();
    let user = test_user(
        &Uuid::new_v4().to_string(),
        crate::models::user::UserType::Person,
    );
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(&user)
        .await
        .unwrap();
    let issued = create_session_and_issue_tokens(&db, &config, &keys, &user.id, None, None)
        .await
        .unwrap();
    (db, config, keys, issued)
}
async fn token_row(db: &mongodb::Database, jwt: &str) -> RefreshToken {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(jwt.split('.').nth(1).unwrap())
        .unwrap();
    let claims: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "jti": claims["jti"].as_str().unwrap() })
        .await
        .unwrap()
        .unwrap()
}
async fn session_row(db: &mongodb::Database, id: &str) -> Session {
    db.collection::<Session>(SESSIONS)
        .find_one(doc! { "_id": id })
        .await
        .unwrap()
        .unwrap()
}
async fn renew(
    db: &mongodb::Database,
    config: &AppConfig,
    keys: &JwtKeys,
    token: &str,
) -> AppResult<IssuedTokens> {
    refresh_tokens(db, config, keys, token, None, None).await
}
async fn shorten_session(db: &mongodb::Database, id: &str) {
    db.collection::<Session>(SESSIONS)
        .update_one(
            doc! { "_id": id },
            doc! {
                "$set": {
                    "created_at": bson::DateTime::from_chrono(Utc::now() - Duration::days(60)),
                    "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(1))
                }
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn rotation_grace_and_repeated_renewal_slide_beyond_the_old_absolute_cap() {
    let (db, config, keys, issued) = fixture().await;
    let mut token = issued.refresh_token.clone();
    for _ in 0..3 {
        shorten_session(&db, &issued.session_id).await;
        let next = renew(&db, &config, &keys, &token).await.unwrap();
        let returned = token_row(&db, &next.refresh_token).await;
        let session = session_row(&db, &issued.session_id).await;
        assert_eq!(session.expires_at, returned.expires_at);
        assert!(session.expires_at > session.created_at + Duration::days(30));
        assert_eq!(
            returned.created_at.timestamp(),
            jwt::verify_token(&keys, &config, &next.refresh_token)
                .unwrap()
                .iat
        );
        shorten_session(&db, &issued.session_id).await;
        let reused = renew(&db, &config, &keys, &token).await.unwrap();
        assert_eq!(reused.refresh_token, next.refresh_token);
        assert_eq!(
            session_row(&db, &issued.session_id).await.expires_at,
            returned.expires_at
        );
        token = next.refresh_token;
    }
}
#[tokio::test]
async fn session_deadline_never_shortens_and_creation_covers_long_refresh_lifetimes() {
    let (db, mut config, keys, issued) = fixture().await;
    let original = session_row(&db, &issued.session_id).await.expires_at;
    renew(&db, &config, &keys, &issued.refresh_token)
        .await
        .unwrap();
    assert_eq!(
        session_row(&db, &issued.session_id).await.expires_at,
        original
    );
    config.jwt_refresh_ttl_secs = Duration::days(45).num_seconds();
    let owner = session_row(&db, &issued.session_id).await.user_id;
    let long = create_session_and_issue_tokens(&db, &config, &keys, &owner, None, None)
        .await
        .unwrap();
    assert_eq!(
        session_row(&db, &long.session_id).await.expires_at,
        token_row(&db, &long.refresh_token).await.expires_at
    );
    let cookie = create_session(&db, &owner, None, None).await.unwrap();
    let row = session_row(&db, &cookie.session_id).await;
    assert_eq!(
        (row.expires_at - row.created_at).num_seconds(),
        SESSION_TTL_SECS
    );
}

#[tokio::test]
async fn legacy_and_mobile_exempt_refresh_keep_fixed_deadlines_through_rotation_and_grace() {
    for (assurance, exemption, slides) in [
        (None, None, false),
        (Some(bson::Bson::Null), None, false),
        (
            Some(bson::Bson::Boolean(false)),
            Some("legacy_migration"),
            false,
        ),
        (
            Some(bson::Bson::Boolean(false)),
            Some("mobile_rollout"),
            false,
        ),
        (
            Some(bson::Bson::Boolean(true)),
            Some("mobile_rollout"),
            true,
        ),
    ] {
        let (db, config, keys, issued) = fixture().await;
        shorten_session(&db, &issued.session_id).await;
        let before = session_row(&db, &issued.session_id).await;
        let mut patch = doc! {"$unset": {"mfa_assured": ""}};
        let mut fields = doc! {"last_active_at": bson::DateTime::from_chrono(
        Utc::now() - Duration::hours(1))};
        if let Some(assurance) = assurance {
            patch.remove("$unset");
            fields.insert("mfa_assured", assurance);
        }
        if let Some(exemption) = exemption {
            fields.insert("mfa_exemption", exemption);
        }
        patch.insert("$set", fields);
        db.collection::<Session>(SESSIONS)
            .update_one(doc! {"_id": &issued.session_id}, patch)
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &before.user_id},
                doc! {"$set": {"mfa_enabled": true}},
            )
            .await
            .unwrap();
        let renewed = renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .unwrap();
        let successor = token_row(&db, &renewed.refresh_token).await;
        let expected = if slides {
            successor.expires_at
        } else {
            before.expires_at
        };
        let after = session_row(&db, &issued.session_id).await;
        assert_eq!(after.expires_at, expected);
        assert!(after.last_active_at > Utc::now() - Duration::minutes(1));
        let recovered = renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .unwrap();
        assert_eq!(recovered.refresh_token, renewed.refresh_token);
        assert_eq!(
            session_row(&db, &issued.session_id).await.expires_at,
            expected
        );
        db.collection::<Session>(SESSIONS)
            .update_one(
                doc! {"_id": &issued.session_id},
                doc! {"$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
                }},
            )
            .await
            .unwrap();
        assert!(
            matches!(renew(&db, &config, &keys, &renewed.refresh_token).await,
            Err(AppError::Unauthorized(message)) if message == "Session has expired")
        );
        assert!(!token_row(&db, &renewed.refresh_token).await.revoked);
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn sidless_migration_and_mobile_exemption_keep_a_fixed_thirty_day_anchor() {
    let (db, mut config, keys, issued) = fixture().await;
    config.jwt_refresh_ttl_secs = Duration::days(45).num_seconds();
    let owner = session_row(&db, &issued.session_id).await.user_id;
    db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": &owner}, doc! {"$set": {"mfa_enabled": true}})
        .await
        .unwrap();
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! {"session_id": &issued.session_id},
            doc! {"$unset": {"session_id": ""}},
        )
        .await
        .unwrap();
    let migrated = renew(&db, &config, &keys, &issued.refresh_token)
        .await
        .unwrap();
    let anchor = session_row(&db, &migrated.session_id).await;
    assert_eq!(
        anchor.mfa_exemption,
        Some(crate::models::session::MfaExemption::LegacyMigration)
    );
    assert_eq!(
        (anchor.expires_at - anchor.created_at).num_seconds(),
        SESSION_TTL_SECS
    );
    assert!(token_row(&db, &migrated.refresh_token).await.expires_at > anchor.expires_at);
    for token in [&issued.refresh_token, &migrated.refresh_token] {
        let next = renew(&db, &config, &keys, token).await.unwrap();
        assert_eq!(next.session_id, anchor.id);
        assert_eq!(
            session_row(&db, &anchor.id).await.expires_at,
            anchor.expires_at
        );
    }
    let mobile = create_mobile_social_tokens(&db, &config, &keys, &owner, None, None, true)
        .await
        .unwrap();
    let row = session_row(&db, &mobile.session_id).await;
    assert_eq!(
        (row.expires_at - row.created_at).num_seconds(),
        SESSION_TTL_SECS
    );
    renew(&db, &config, &keys, &mobile.refresh_token)
        .await
        .unwrap();
    assert_eq!(
        session_row(&db, &mobile.session_id).await.expires_at,
        row.expires_at
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn concurrent_rotations_converge_on_one_successor_and_slide_its_expiry() {
    let (db, config, keys, issued) = fixture().await;
    shorten_session(&db, &issued.session_id).await;
    let results = futures::future::join_all(
        (0..8).map(|_| renew(&db, &config, &keys, &issued.refresh_token)),
    )
    .await;
    let pairs: Vec<_> = results.into_iter().map(Result::unwrap).collect();
    assert!(
        pairs
            .iter()
            .all(|pair| pair.refresh_token == pairs[0].refresh_token)
    );
    assert_eq!(
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        session_row(&db, &issued.session_id).await.expires_at,
        token_row(&db, &pairs[0].refresh_token).await.expires_at
    );
}
#[tokio::test]
async fn expired_at_slide_aborts_cas_and_insert_and_never_returns_credentials() {
    let (db, config, keys, issued) = fixture().await;
    let old = token_row(&db, &issued.refresh_token).await;
    let mut successor = old.clone();
    successor.id = Uuid::new_v4().to_string();
    successor.jti = Uuid::new_v4().to_string();
    db.collection::<Session>(SESSIONS)
        .update_one(
            doc! { "_id": &issued.session_id },
            doc! {
                "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
                }
            },
        )
        .await
        .unwrap();
    let error = rotate_refresh(&db, &old, &successor).await.unwrap_err();
    assert!(matches!(error, AppError::Unauthorized(ref text) if text == "Session has expired"));
    assert!(!token_row(&db, &issued.refresh_token).await.revoked);
    assert_eq!(
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn successor_insert_failure_aborts_the_old_token_cas() {
    let (db, config, keys, issued) = fixture().await;
    let old = token_row(&db, &issued.refresh_token).await;
    let mut duplicate = old.clone();
    duplicate.jti = Uuid::new_v4().to_string();
    assert!(rotate_refresh(&db, &old, &duplicate).await.is_err());
    assert!(!token_row(&db, &issued.refresh_token).await.revoked);
    assert!(
        renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn replacement_chain_follows_rotated_intermediates_and_bounds_missing_links() {
    let (db, config, keys, issued) = fixture().await;
    let b = renew(&db, &config, &keys, &issued.refresh_token)
        .await
        .unwrap();
    let c = renew(&db, &config, &keys, &b.refresh_token).await.unwrap();
    assert_eq!(
        renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .unwrap()
            .refresh_token,
        c.refresh_token
    );
    let b_row = token_row(&db, &b.refresh_token).await;
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .delete_one(doc! { "_id": &b_row.id })
        .await
        .unwrap();
    let old_revoked_at = token_row(&db, &issued.refresh_token).await.revoked_at;
    for _ in 0..2 {
        let error = renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .err()
            .unwrap();
        assert!(is_rotation_conflict(&error));
        assert_eq!(error.into_response().status().as_u16(), 409);
        assert!(!session_row(&db, &issued.session_id).await.revoked);
        assert_eq!(
            token_row(&db, &issued.refresh_token).await.revoked_at,
            old_revoked_at
        );
    }
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "jti": &token_row(&db, &issued.refresh_token).await.jti },
            doc! {
                "$set": {
                    "revoked_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(121))
                }
            },
        )
        .await
        .unwrap();
    assert!(
        renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .is_err()
    );
    assert!(session_row(&db, &issued.session_id).await.revoked);
}
#[tokio::test]
async fn legacy_missing_revocation_time_recovers_active_chain_but_never_missing_link_conflict() {
    for missing in [false, true] {
        let (db, config, keys, issued) = fixture().await;
        let next = renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .unwrap();
        let old = token_row(&db, &issued.refresh_token).await;
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(
                doc! { "_id": &old.id },
                doc! {
                    "$unset": {
                        "revoked_at": ""
                    }
                },
            )
            .await
            .unwrap();
        if missing {
            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .delete_one(doc! { "_id": old.replaced_by.unwrap() })
                .await
                .unwrap();
        }
        let result = renew(&db, &config, &keys, &issued.refresh_token).await;
        if missing {
            assert!(matches!(result, Err(AppError::Unauthorized(_))));
            assert!(session_row(&db, &issued.session_id).await.revoked);
        } else {
            assert_eq!(result.unwrap().refresh_token, next.refresh_token);
        }
    }
}
#[tokio::test]
async fn terminal_successor_and_depth_exhaustion_revoke_instead_of_conflict() {
    for depth in [1, MAX_REPLACEMENT_CHAIN_DEPTH + 1] {
        let (db, config, keys, issued) = fixture().await;
        let mut token = issued.refresh_token.clone();
        for _ in 0..depth {
            token = renew(&db, &config, &keys, &token)
                .await
                .unwrap()
                .refresh_token;
        }
        if depth == 1 {
            let row = token_row(&db, &token).await;
            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .update_one(
                    doc! { "_id": row.id },
                    doc! {
                        "$set": {
                            "revoked": true
                        }
                    },
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            renew(&db, &config, &keys, &issued.refresh_token).await,
            Err(AppError::Unauthorized(_))
        ));
        assert!(session_row(&db, &issued.session_id).await.revoked);
    }
}
#[tokio::test]
async fn expired_successor_is_terminal_and_revokes_the_session() {
    let (db, config, keys, issued) = fixture().await;
    let next = renew(&db, &config, &keys, &issued.refresh_token)
        .await
        .unwrap();
    let successor = token_row(&db, &next.refresh_token).await;
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "_id": successor.id },
            doc! {
                "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
                }
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        renew(&db, &config, &keys, &issued.refresh_token).await,
        Err(AppError::Unauthorized(ref message)) if message == "Refresh token has been revoked"
    ));
    assert!(session_row(&db, &issued.session_id).await.revoked);
}
#[tokio::test]
async fn revoked_expired_missing_and_ineligible_sessions_have_distinct_refresh_reasons() {
    for (state, expected) in [
        ("revoked", "Session has been revoked"),
        ("expired", "Session has expired"),
        ("missing", "Session has ended"),
        ("pending", "Session has ended"),
        ("unknown", "Session has ended"),
        ("owner", "Session has ended"),
    ] {
        let (db, config, keys, issued) = fixture().await;
        let collection = db.collection::<bson::Document>(SESSIONS);
        match state {
            "missing" => {
                collection
                    .delete_one(doc! { "_id": &issued.session_id })
                    .await
                    .unwrap();
            }
            "revoked" => {
                collection
                    .update_one(
                        doc! { "_id": &issued.session_id },
                        doc! {
                            "$set": {
                                "revoked": true
                            }
                        },
                    )
                    .await
                    .unwrap();
            }
            "expired" => {
                collection.update_one(doc! { "_id": &issued.session_id },doc! {
    "$set": {
        "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
    }
}).await.unwrap();
            }
            "owner" => {
                collection
                    .update_one(
                        doc! { "_id": &issued.session_id },
                        doc! {
                            "$set": {
                                "user_id": Uuid::new_v4().to_string()
                            }
                        },
                    )
                    .await
                    .unwrap();
            }
            _ => {
                collection
                    .update_one(
                        doc! { "_id": &issued.session_id },
                        doc! {
                            "$set": {
                                "purpose": if state == "pending"{
                                    "mfa_pending"
                                } else {
                                    "future_purpose"
                                }
                            }
                        },
                    )
                    .await
                    .unwrap();
            }
        }
        let error = renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(
                error,
                AppError::Unauthorized(ref message) if message == expected
            ),
            "{state}: {error}"
        );
        assert_eq!(
            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn live_session_on_refresh_diagnostic_reread_is_transient_without_rotation() {
    let (db, config, keys, issued) = fixture().await;
    db.collection::<Session>(SESSIONS)
        .update_one(
            doc! { "_id": &issued.session_id },
            doc! {
                "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
                }
            },
        )
        .await
        .unwrap();
    let result = super::super::session_authority::RESTORE_BEFORE_END_READ
        .scope(true, renew(&db, &config, &keys, &issued.refresh_token))
        .await;
    assert!(matches!(result, Err(AppError::OAuthAuthorityUnavailable)));
    assert!(!token_row(&db, &issued.refresh_token).await.revoked);
    assert_eq!(
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

async fn oauth_fixture(db: &mongodb::Database, issued: &IssuedTokens) -> String {
    let row = token_row(db, &issued.refresh_token).await;
    let client = Uuid::new_v4().to_string();
    db.collection::<bson::Document>(OAUTH_CLIENTS)
        .insert_one(doc! {
            "_id": &client,
            "client_name": "test",
            "client_secret_hash": "",
            "redirect_uris": [],
            "allowed_scopes": "openid",
            "grant_types": "refresh_token",
            "client_type": "public",
            "is_active": true,
            "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    let consent = super::super::consent_service::grant_consent_with_services(
        db,
        &row.user_id,
        &client,
        "openid",
        None,
    )
    .await
    .unwrap();
    let reference = super::super::consent_service::ensure_issuance_reference(db, &consent)
        .await
        .unwrap();
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "_id": row.id },
            doc! {
                "$set": {
                    "client_id": &client,
                    "session_id": null,
                    "scope": "openid",
                    "protected_consent": bson::to_bson(&reference).unwrap()
                }
            },
        )
        .await
        .unwrap();
    client
}
#[tokio::test]
async fn post_commit_transient_read_write_and_unknown_results_keep_successor_for_grace_recovery() {
    for failure in [
        AppError::OAuthAuthorityUnavailable,
        AppError::Internal("majority write failed".into()),
        super::super::transaction_service::indeterminate_commit(),
    ] {
        let (db, config, keys, issued) = fixture().await;
        oauth_fixture(&db, &issued).await;
        let initial = session_row(&db, &issued.session_id).await.expires_at;
        let result = POST_COMMIT_FAILURE
            .scope(
                std::cell::RefCell::new(Some(failure)),
                renew(&db, &config, &keys, &issued.refresh_token),
            )
            .await;
        assert!(matches!(result, Err(AppError::OAuthAuthorityUnavailable)));
        let rows: Vec<RefreshToken> = db
            .collection(REFRESH_TOKENS)
            .find(doc! { "revoked": false })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let retry = renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .unwrap();
        assert_eq!(token_row(&db, &retry.refresh_token).await.id, rows[0].id);
        assert_eq!(
            session_row(&db, &issued.session_id).await.expires_at,
            initial
        );
    }
}
#[tokio::test]
async fn post_commit_definitive_denial_revokes_successor_and_regrant_never_resurrects_old_chain() {
    for transient_first in [false, true] {
        let (db, config, keys, issued) = fixture().await;
        let client = oauth_fixture(&db, &issued).await;
        let failure = if transient_first {
            AppError::OAuthAuthorityUnavailable
        } else {
            super::super::consent_service::grant_changed()
        };
        assert!(
            POST_COMMIT_FAILURE
                .scope(
                    std::cell::RefCell::new(Some(failure)),
                    renew(&db, &config, &keys, &issued.refresh_token)
                )
                .await
                .is_err()
        );
        let active = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! { "revoked": false })
            .await
            .unwrap();
        assert_eq!(active, u64::from(transient_first));
        let user = token_row(&db, &issued.refresh_token).await.user_id;
        super::super::consent_service::revoke_consent(&db, &user, &client)
            .await
            .unwrap();
        super::super::consent_service::grant_consent_with_services(
            &db, &user, &client, "openid", None,
        )
        .await
        .unwrap();
        assert!(
            renew(&db, &config, &keys, &issued.refresh_token)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn malformed_session_storage_is_a_database_error_never_expiry() {
    let (db, config, keys, issued) = fixture().await;
    db.collection::<bson::Document>(SESSIONS)
        .update_one(
            doc! { "_id": &issued.session_id },
            doc! {
                "$set": {
                    "created_at": "invalid BSON datetime"
                }
            },
        )
        .await
        .unwrap();
    let error = renew(&db, &config, &keys, &issued.refresh_token)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, AppError::DatabaseError(_)));
    assert_eq!(error.into_response().status().as_u16(), 500);
}
#[tokio::test]
async fn assurance_and_legacy_sentinel_gates_prevent_rotation_and_grace_sliding() {
    for grace in [false, true] {
        for change in ["unassured", "pending", "unknown", "sentinel"] {
            let (db, config, keys, issued) = fixture().await;
            if grace {
                renew(&db, &config, &keys, &issued.refresh_token)
                    .await
                    .unwrap();
            }
            shorten_session(&db, &issued.session_id).await;
            let before = session_row(&db, &issued.session_id).await;
            let patch = match change {
                "unassured" => doc! { "mfa_assured": false },
                "pending" => doc! { "purpose": "mfa_pending" },
                "unknown" => doc! { "purpose": "future-purpose" },
                _ => doc! { "user_agent": "mfa_pending" },
            };
            db.collection::<Session>(SESSIONS)
                .update_one(doc! { "_id": &issued.session_id }, doc! { "$set": patch })
                .await
                .unwrap();
            if change == "unassured" {
                db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                    .update_one(
                        doc! { "_id": &before.user_id },
                        doc! {
                            "$set": {
                                "mfa_enabled": true
                            }
                        },
                    )
                    .await
                    .unwrap();
            }
            assert!(
                renew(&db, &config, &keys, &issued.refresh_token)
                    .await
                    .is_err()
            );
            assert_eq!(
                session_row(&db, &issued.session_id).await.expires_at,
                before.expires_at
            );
            assert_eq!(
                db.collection::<RefreshToken>(REFRESH_TOKENS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                if grace { 2 } else { 1 }
            );
        }
    }
}
#[tokio::test]
async fn successful_refresh_preserves_mcp_fallback_but_reuse_revokes_it() {
    let (db, config, keys, issued) = fixture().await;
    let owner = session_row(&db, &issued.session_id).await.user_id;
    let mcp = McpSessionStore::new();
    let id = mcp
        .create_with_parent_session(&owner, true, true, Some(&issued.session_id))
        .await
        .unwrap()
        .unwrap();
    let renewed = refresh_tokens(&db, &config, &keys, &issued.refresh_token, Some(&mcp), None)
        .await
        .unwrap();
    assert!(mcp.get_for_auth(&id).await.unwrap().is_some());
    refresh_tokens(&db, &config, &keys, &issued.refresh_token, Some(&mcp), None)
        .await
        .unwrap();
    assert!(mcp.get_for_auth(&id).await.unwrap().is_some());
    let old = token_row(&db, &issued.refresh_token).await;
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "_id": old.id },
            doc! {
                "$set": {
                    "revoked_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(121))
                }
            },
        )
        .await
        .unwrap();
    assert!(
        refresh_tokens(&db, &config, &keys, &issued.refresh_token, Some(&mcp), None)
            .await
            .is_err()
    );
    assert!(mcp.get_for_auth(&id).await.unwrap().is_none());
    assert!(token_row(&db, &renewed.refresh_token).await.revoked);
}

#[tokio::test]
async fn unknown_commit_retries_only_commit_and_indeterminate_commit_recovers_through_grace() {
    use crate::services::service_history::transaction::{COMMIT_RESPONSE_LOSS, CommitResponseLoss};
    for lost in [1, usize::MAX] {
        let (db, config, keys, issued) = fixture().await;
        let result = COMMIT_RESPONSE_LOSS
            .scope(
                std::cell::RefCell::new(CommitResponseLoss {
                    remaining: lost,
                    attempts: 0,
                    budget: if lost == 1 {
                        std::time::Duration::from_secs(30)
                    } else {
                        std::time::Duration::from_millis(100)
                    },
                }),
                async {
                    let result = renew(&db, &config, &keys, &issued.refresh_token).await;
                    assert!(
                        COMMIT_RESPONSE_LOSS.with(|fault| fault.borrow().attempts)
                            >= if lost == 1 { 2 } else { 1 }
                    );
                    result
                },
            )
            .await;
        if lost == 1 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(AppError::OAuthAuthorityUnavailable)));
        }
        assert_eq!(
            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        let old = token_row(&db, &issued.refresh_token).await;
        let successor = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "_id": old.replaced_by.unwrap() })
            .await
            .unwrap()
            .unwrap();
        assert!(!successor.revoked);
        let retry = renew(&db, &config, &keys, &issued.refresh_token)
            .await
            .unwrap();
        assert_eq!(token_row(&db, &retry.refresh_token).await.id, successor.id);
    }
}
