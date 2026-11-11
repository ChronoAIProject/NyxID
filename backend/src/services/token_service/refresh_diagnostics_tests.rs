use super::*;
use crate::handlers::auth::{RefreshRequest, refresh};
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};
use axum::{Json, extract::State, http::HeaderMap, response::IntoResponse};
use serde_json::{Value, json};

async fn fixture() -> (crate::AppState, IssuedTokens) {
    let db = connect_transaction_test_database("refresh_diagnostics").await;
    let state = test_app_state(db);
    let user = Uuid::new_v4().to_string();
    state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&user, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let tokens = create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &user,
        None,
        None,
    )
    .await
    .unwrap();
    (state, tokens)
}

async fn assert_refusal(state: &crate::AppState, token: String, reason: &str, previous: AppError) {
    let error = match refresh(
        State(state.clone()),
        HeaderMap::new(),
        Some(Json(RefreshRequest {
            refresh_token: Some(token),
        })),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("refresh unexpectedly succeeded"),
    };
    let response = error.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    let mut expected = serde_json::to_value(previous.response_body()).unwrap();
    expected["details"] = json!({"reason": reason});
    // Entire original envelope, with only details.reason changed; no identifiers.
    assert_eq!(body, expected);
}

#[tokio::test]
async fn refresh_diagnostics_token_expiry_keeps_2001_contract() {
    let (state, tokens) = fixture().await;
    let claims = jwt::verify_token(&state.jwt_keys, &state.config, &tokens.refresh_token).unwrap();
    let expired = jwt::reissue_refresh_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&claims.sub).unwrap(),
        &claims.jti,
        Utc::now().timestamp() - 700000,
        Utc::now().timestamp() - 120,
    )
    .unwrap();
    assert_refusal(&state, expired, "refresh_expired", AppError::TokenExpired).await;
}

#[tokio::test]
async fn refresh_diagnostics_reuse_and_explicit_revocation_are_distinct() {
    for reused in [false, true] {
        let (state, tokens) = fixture().await;
        if reused {
            refresh_tokens(
                &state.db,
                &state.config,
                &state.jwt_keys,
                &tokens.refresh_token,
                None,
                None,
            )
            .await
            .unwrap();
            state.db.collection::<RefreshToken>(REFRESH_TOKENS).update_many(doc! {"session_id": &tokens.session_id, "revoked":true},
                doc! {"$set":{"revoked_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(REUSE_GRACE_PERIOD_SECS+1))}}).await.unwrap();
        } else {
            state
                .db
                .collection::<RefreshToken>(REFRESH_TOKENS)
                .update_many(
                    doc! {"session_id": &tokens.session_id},
                    doc! {"$set":{"revoked":true}},
                )
                .await
                .unwrap();
        }
        assert_refusal(
            &state,
            tokens.refresh_token,
            if reused {
                "refresh_reused"
            } else {
                "refresh_revoked"
            },
            AppError::Unauthorized("Refresh token has been revoked".into()),
        )
        .await;
    }
}

#[tokio::test]
async fn refresh_diagnostics_parent_expired_revoked_or_removed() {
    for reason in ["session_expired", "session_revoked", "session_unavailable"] {
        let (state, tokens) = fixture().await;
        let sessions = state.db.collection::<Session>(SESSIONS);
        match reason {
            "session_expired" => {
                sessions.update_one(doc! {"_id": &tokens.session_id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
            }
            "session_revoked" => {
                revoke_session(&state.db, &tokens.session_id, None)
                    .await
                    .unwrap();
            }
            _ => {
                sessions
                    .delete_one(doc! {"_id": &tokens.session_id})
                    .await
                    .unwrap();
            }
        }
        assert_refusal(
            &state,
            tokens.refresh_token,
            reason,
            AppError::Unauthorized("Session expired or revoked".into()),
        )
        .await;
    }
}

#[tokio::test]
async fn refresh_diagnostics_rotation_rolls_seven_days_but_never_extends_session() {
    let (state, tokens) = fixture().await;
    assert_eq!(state.config.jwt_refresh_ttl_secs, 7 * 24 * 3600);
    let claims = jwt::verify_token(&state.jwt_keys, &state.config, &tokens.refresh_token).unwrap();
    let now = Utc::now();
    let issued = now - Duration::days(6);
    let expiry = issued + Duration::seconds(state.config.jwt_refresh_ttl_secs);
    let parent_expiry = now + Duration::hours(23);
    state.db.collection::<RefreshToken>(REFRESH_TOKENS).update_one(doc! {"jti": &claims.jti},
        doc! {"$set":{"created_at":bson::DateTime::from_chrono(issued),"expires_at":bson::DateTime::from_chrono(expiry)}}).await.unwrap();
    state.db.collection::<Session>(SESSIONS).update_one(doc! {"_id": &tokens.session_id}, doc! {"$set":{
        "created_at":bson::DateTime::from_chrono(parent_expiry-Duration::seconds(SESSION_TTL_SECS)),
        "expires_at":bson::DateTime::from_chrono(parent_expiry),
        "last_active_at":bson::DateTime::from_chrono(now-Duration::hours(23))}}).await.unwrap();
    let aged = jwt::reissue_refresh_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&claims.sub).unwrap(),
        &claims.jti,
        issued.timestamp(),
        expiry.timestamp(),
    )
    .unwrap();
    let rotated = refresh_tokens(&state.db, &state.config, &state.jwt_keys, &aged, None, None)
        .await
        .unwrap();
    let next = jwt::verify_token(&state.jwt_keys, &state.config, &rotated.refresh_token).unwrap();
    assert!(next.exp >= now.timestamp() + state.config.jwt_refresh_ttl_secs);
    assert_eq!(next.exp - next.iat, state.config.jwt_refresh_ttl_secs);
    let row = state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! {"jti": &next.jti})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.expires_at - row.created_at).num_seconds(),
        state.config.jwt_refresh_ttl_secs
    );
    let parent = state
        .db
        .collection::<Session>(SESSIONS)
        .find_one(doc! {"_id": &tokens.session_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        parent.expires_at.timestamp_millis(),
        parent_expiry.timestamp_millis()
    );
    assert!(parent.last_active_at >= now - Duration::seconds(1));
    // A grace-window retry converges, rather than issuing yet another token.
    let retry = refresh_tokens(&state.db, &state.config, &state.jwt_keys, &aged, None, None)
        .await
        .unwrap();
    assert_eq!(retry.refresh_token, rotated.refresh_token);
    // Active use cannot cross the parent's absolute deadline.
    state
        .db
        .collection::<Session>(SESSIONS)
        .update_one(
            doc! {"_id": &tokens.session_id},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(now-Duration::seconds(1))}},
        )
        .await
        .unwrap();
    assert_refusal(
        &state,
        rotated.refresh_token,
        "session_expired",
        AppError::Unauthorized("Session expired or revoked".into()),
    )
    .await;
}

#[tokio::test]
async fn refresh_diagnostics_list_reports_only_owners_latest_live_token_metadata() {
    let (state, tokens) = fixture().await;
    let claims = jwt::verify_token(&state.jwt_keys, &state.config, &tokens.refresh_token).unwrap();
    let rotated = refresh_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &tokens.refresh_token,
        None,
        None,
    )
    .await
    .unwrap();
    let next = jwt::verify_token(&state.jwt_keys, &state.config, &rotated.refresh_token).unwrap();
    // A revoked row with a later expiry must not affect the displayed deadline.
    state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! {"jti": &claims.jti},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(90))}},
        )
        .await
        .unwrap();
    // Another OAuth client's longer refresh TTL is not the CLI account TTL.
    let mut oauth = state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! {"jti": &next.jti})
        .await
        .unwrap()
        .unwrap();
    oauth.id = Uuid::new_v4().to_string();
    oauth.jti = Uuid::new_v4().to_string();
    oauth.client_id = Uuid::new_v4().to_string();
    oauth.expires_at = Utc::now() + Duration::days(60);
    state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(oauth)
        .await
        .unwrap();
    let mut auth = crate::test_utils::test_auth_user(&claims.sub);
    auth.session_id = Some(Uuid::parse_str(&tokens.session_id).unwrap());
    let Json(rows) = crate::handlers::sessions::list_sessions(State(state.clone()), auth)
        .await
        .unwrap();
    let json = serde_json::to_value(&rows).unwrap();
    assert_eq!(json[0]["session_expires_at"], json[0]["expires_at"]);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(json[0]["refresh_expires_at"].as_str().unwrap())
            .unwrap()
            .timestamp(),
        next.exp
    );
    assert_eq!(json[0]["is_current"], true);
    let Json(other) = crate::handlers::sessions::list_sessions(
        State(state),
        crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
    )
    .await
    .unwrap();
    assert!(other.is_empty());
}

#[tokio::test]
async fn refresh_diagnostics_legacy_binding_race_maps_current_row_without_creating_session() {
    for condition in ["revoked", "expired", "removed"] {
        let (state, tokens) = fixture().await;
        let row = state
            .db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! {"session_id": &tokens.session_id})
            .await
            .unwrap()
            .unwrap();
        let (reason, change) = match condition {
            "revoked" => (
                "refresh_revoked",
                doc! {"revoked":true,"session_id":bson::Bson::Null},
            ),
            "expired" => (
                "refresh_expired",
                doc! {"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1)),"session_id":bson::Bson::Null},
            ),
            _ => ("refresh_not_found", doc! {}),
        };
        if condition == "removed" {
            state
                .db
                .collection::<RefreshToken>(REFRESH_TOKENS)
                .delete_one(doc! {"_id": &row.id})
                .await
                .unwrap();
        } else {
            state
                .db
                .collection::<RefreshToken>(REFRESH_TOKENS)
                .update_one(doc! {"_id": &row.id}, doc! {"$set":change})
                .await
                .unwrap();
        }
        let error = bind_legacy_refresh_session(&state.db, &row)
            .await
            .expect_err("stale row must not bind");
        assert_eq!(
            error.response_body().details.unwrap(),
            json!({"reason":reason})
        );
        assert_eq!(
            error.response_body().message,
            "Unauthorized: Refresh token has expired or been revoked"
        );
        assert_eq!(
            state
                .db
                .collection::<Session>(SESSIONS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn refresh_diagnostics_missing_token_and_unrelated_auth_failure_remain_distinct() {
    let (state, tokens) = fixture().await;
    state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .delete_many(doc! {"session_id": &tokens.session_id})
        .await
        .unwrap();
    assert_refusal(
        &state,
        tokens.refresh_token,
        "refresh_not_found",
        AppError::Unauthorized("Refresh token not found".into()),
    )
    .await;
    assert_refusal(
        &state,
        tokens.access_token,
        "authentication_failed",
        AppError::Unauthorized("Expected refresh token".into()),
    )
    .await;
}
