use super::*;

const RETURN: &str = "nyxid://auth/social/callback";

async fn fixture() -> AppState {
    let db = crate::test_utils::connect_transaction_test_database("social_mobile_flow").await;
    let mut state = crate::test_utils::test_app_state(db);
    state.config.environment = "production".into();
    state.config.google_client_id = Some("fixture".into());
    state.config.google_client_secret = Some("fixture".into());
    state
}

async fn start(state: &AppState, redirect: &str) -> AppResult<String> {
    let query = serde_json::from_value(serde_json::json!({
        "client": "mobile", "redirect_uri": redirect
    }))
    .unwrap();
    let (_, headers, ()) = authorize(
        State(state.clone()),
        None,
        Path("google".into()),
        Query(query),
        HeaderMap::new(),
    )
    .await?;
    let location = url::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
    Ok(location
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned())
}

async fn cancel(state: &AppState, provider: &str, csrf: &str, headers: HeaderMap) -> String {
    let response = callback(
        State(state.clone()),
        None,
        ConnectInfo("127.0.0.1:12345".parse().unwrap()),
        Path(provider.into()),
        Query(CallbackQuery {
            code: None,
            state: Some(csrf.into()),
            error: Some("access_denied".into()),
        }),
        headers,
    )
    .await
    .unwrap_err();
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn mobile_redirect_requires_exact_login_destination() {
    let state = fixture().await;
    for uri in [
        "nyxid://auth/other",
        "nyxid://services/oauth-return",
        "nyxid://auth/social/callback?x",
        "nyxid://auth/social/callback#x",
        "nyxid://auth/social/callback/",
        "exp://127.0.0.1:8081/--/auth/social/callback",
    ] {
        assert!(
            matches!(start(&state, uri).await, Err(AppError::ValidationError(_))),
            "{uri}"
        );
    }
    assert!(start(&state, RETURN).await.is_ok());
}

#[tokio::test]
async fn released_mobile_start_resolves_once_without_a_cookie() {
    let state = fixture().await;
    let csrf = start(&state, RETURN).await.unwrap();
    let cancel_once = || cancel(&state, "google", &csrf, HeaderMap::new());
    let (a, b) = tokio::join!(cancel_once(), cancel_once());
    let outcomes = [a, b];
    assert_eq!(
        outcomes
            .iter()
            .filter(|url| url.starts_with(RETURN) && url.contains("social_auth_denied"))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|url| url.contains("social_auth_csrf"))
            .count(),
        1
    );
}

#[tokio::test]
async fn mobile_code_start_requires_s256_and_the_released_return_uri() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let state = fixture().await;
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let app = private.with_state(state);
    for (method, status) in [
        ("plain", StatusCode::BAD_REQUEST),
        ("S256", StatusCode::FOUND),
    ] {
        let challenge = crate::services::oauth_flow::generate_code_challenge(&"v".repeat(43));
        let uri = format!(
            "/api/v1/auth/social/google/mobile-code?redirect_uri={RETURN}&code_challenge={challenge}&code_challenge_method={method}"
        );
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
}

#[tokio::test]
async fn mobile_state_is_bound_to_its_provider() {
    let state = fixture().await;
    let csrf = start(&state, RETURN).await.unwrap();
    assert!(
        cancel(&state, "github", &csrf, HeaderMap::new())
            .await
            .contains("social_auth_csrf")
    );
    assert!(
        cancel(&state, "google", &csrf, HeaderMap::new())
            .await
            .starts_with(RETURN)
    );
}

#[tokio::test]
async fn mismatched_browser_cookie_preserves_the_original_mobile_attempt() {
    let state = fixture().await;
    let csrf = start(&state, RETURN).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("{SOCIAL_STATE_COOKIE}=another-attempt")
            .parse()
            .unwrap(),
    );
    assert!(
        cancel(&state, "google", &csrf, headers)
            .await
            .contains("social_auth_csrf")
    );
    assert!(
        cancel(&state, "google", &csrf, HeaderMap::new())
            .await
            .starts_with(RETURN)
    );
}

#[tokio::test]
async fn apple_form_post_requires_state_started_for_apple() {
    let state = fixture().await;
    let csrf = start(&state, RETURN).await.unwrap();
    let form = serde_json::from_value(serde_json::json!({
        "state": csrf, "error": "access_denied"
    }))
    .unwrap();
    let response = apple_callback(
        State(state.clone()),
        None,
        ConnectInfo("127.0.0.1:12345".parse().unwrap()),
        HeaderMap::new(),
        axum::Form(form),
    )
    .await
    .unwrap_err();
    assert!(
        response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .contains("social_auth_csrf")
    );
    assert!(
        cancel(&state, "google", &csrf, HeaderMap::new())
            .await
            .starts_with(RETURN)
    );
}

#[tokio::test]
async fn apple_start_recovers_the_nonce_and_target_without_cookies() {
    let mut app = fixture().await;
    app.config.apple_client_id = Some("mobile-fixture".into());
    app.config.apple_team_id = Some("fixture-team".into());
    app.config.apple_key_id = Some("fixture-key".into());
    app.config.apple_private_key_path = Some("unused-fixture-path".into());
    let query =
        serde_json::from_value(serde_json::json!({"client":"mobile","redirect_uri":RETURN}))
            .unwrap();
    let (_, headers, ()) = authorize(
        State(app.clone()),
        None,
        Path("apple".into()),
        Query(query),
        HeaderMap::new(),
    )
    .await
    .unwrap();
    let location = url::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
    let params: std::collections::HashMap<_, _> = location.query_pairs().collect();
    let (target, nonce_hash) = callback_target(
        &app,
        &HeaderMap::new(),
        "apple",
        None,
        Some(&params["state"]),
    )
    .await
    .unwrap();
    assert!(
        matches!(target, SocialRedirectTarget::Mobile {redirect_uri, ..} if redirect_uri == RETURN)
    );
    assert!(nonce_matches_cookie_hash(
        Some(&params["nonce"]),
        nonce_hash.as_deref()
    ));
}

#[tokio::test]
async fn code_delivery_releases_tokens_once_with_the_matching_verifier() {
    use crate::models::user::{User, UserType};
    let app = fixture().await;
    let id = uuid::Uuid::new_v4().to_string();
    let user = crate::test_utils::test_user(&id, UserType::Person);
    app.db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(&user)
        .await
        .unwrap();
    let verifier = crate::services::oauth_flow::generate_code_verifier();
    let target = SocialRedirectTarget::Mobile {
        redirect_uri: RETURN.into(),
        code_challenge: Some(crate::services::oauth_flow::generate_code_challenge(
            &verifier,
        )),
    };
    let issue = || async {
        let (_, headers, ()) = finish_social_sign_in(SocialSignIn {
            state: &app,
            user: &user,
            provider: "google",
            redirect_target: &target,
            signup_has_name: Some(true),
            ip: None,
            ua: None,
            tele_social: &TelemetryContext::pre_auth(&HeaderMap::new()),
        })
        .await
        .unwrap();
        let url = url::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
        assert!(
            url.query_pairs()
                .all(|(key, _)| matches!(key.as_ref(), "status" | "provider" | "code"))
        );
        url.query_pairs()
            .find(|(key, _)| key == "code")
            .unwrap()
            .1
            .into_owned()
    };
    let stored_rows = || async {
        let count = async |name: &str| {
            app.db
                .collection::<bson::Document>(name)
                .count_documents(bson::doc! {"user_id": &id})
                .await
                .unwrap()
        };
        (
            count(crate::models::session::COLLECTION_NAME).await,
            count(crate::models::refresh_token::COLLECTION_NAME).await,
        )
    };
    let expired = issue().await;
    app.db
        .collection::<bson::Document>(
            crate::models::mobile_social::MobileSocialDelivery::COLLECTION_NAME,
        )
        .update_one(
            bson::doc! {"code_hash":hash_token(&expired)},
            bson::doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(
        mobile::redeem(&app.db, &app.encryption_keys, &expired, &verifier)
            .await
            .is_err()
    );
    assert_eq!(stored_rows().await, (0, 0));

    let revoked = issue().await;
    crate::services::admin_user_service::revoke_all_user_sessions(&app.db, &id)
        .await
        .unwrap();
    assert!(matches!(
        mobile::redeem(&app.db, &app.encryption_keys, &revoked, &verifier).await,
        Err(AppError::ValidationError(_))
    ));

    let code = issue().await;
    assert!(
        mobile::redeem(&app.db, &app.encryption_keys, &code, &"w".repeat(43))
            .await
            .is_err()
    );
    assert_eq!(stored_rows().await, (0, 0));
    let redeem = || mobile::redeem(&app.db, &app.encryption_keys, &code, &verifier);
    let (first, second) = tokio::join!(redeem(), redeem());
    let (tokens, loser) = match (first, second) {
        (Ok(tokens), Err(error)) | (Err(error), Ok(tokens)) => (tokens, error),
        _ => panic!("expected one successful redemption"),
    };
    assert!(matches!(loser, AppError::ValidationError(_)));
    assert!(!tokens.access_token.is_empty() && !tokens.refresh_token.is_empty());
    assert_eq!(stored_rows().await, (1, 1));
    let claims =
        crate::crypto::jwt::verify_token(&app.jwt_keys, &app.config, &tokens.access_token).unwrap();
    let sid = claims.sid.unwrap();
    assert_eq!(
        app.db
            .collection::<bson::Document>(crate::models::session::COLLECTION_NAME)
            .count_documents(bson::doc! {"_id": &sid, "user_id": &id, "revoked": false})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        app.db
            .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
            .count_documents(bson::doc! {"session_id": &sid, "user_id": &id, "revoked": false})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn mobile_cookie_and_state_round_trip_on_the_issuing_origin() {
    let app = fixture().await;
    let query =
        serde_json::from_value(serde_json::json!({"client": "mobile", "redirect_uri": RETURN}))
            .unwrap();
    let (_, start_headers, ()) = authorize(
        State(app.clone()),
        None,
        Path("google".into()),
        Query(query),
        HeaderMap::new(),
    )
    .await
    .unwrap();
    let cookie = start_headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("nyx_social_state="))
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let location = url::Url::parse(start_headers[header::LOCATION].to_str().unwrap()).unwrap();
    let raw = location
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1;
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, cookie.parse().unwrap());
    assert!(!state_matches_cookie_hash(
        &raw,
        extract_cookie_value(&headers, SOCIAL_STATE_COOKIE).as_deref()
    ));
    assert!(
        cancel(&app, "google", &raw, headers)
            .await
            .contains("social_auth_denied")
    );
}

#[tokio::test]
async fn mobile_state_expires_before_ttl_cleanup() {
    let app = fixture().await;
    let raw = start(&app, RETURN).await.unwrap();
    app.db
        .collection::<bson::Document>(
            crate::models::mobile_social::MobileSocialFlow::COLLECTION_NAME,
        )
        .update_one(
            bson::doc! {"state_hash":hash_token(&raw)},
            bson::doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(
        cancel(&app, "google", &raw, HeaderMap::new())
            .await
            .contains("social_auth_csrf")
    );
}

#[test]
fn mobile_hint_cookies_alone_resolve_to_browser() {
    let config = crate::test_utils::test_app_config();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("{SOCIAL_CLIENT_COOKIE}=mobile; {SOCIAL_REDIRECT_COOKIE}={RETURN}")
            .parse()
            .unwrap(),
    );
    assert!(matches!(
        resolve_redirect_target(&headers, &config),
        SocialRedirectTarget::Web { .. }
    ));
}
