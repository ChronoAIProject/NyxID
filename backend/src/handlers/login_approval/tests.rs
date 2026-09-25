use super::*;
use crate::models::{session::Session, user::UserType};
use crate::test_utils::{connect_test_database, test_app_state, test_user};
use mongodb::bson::doc;
use serde_json::{Value, json};

async fn exercise(keep: bool, mfa: bool) {
    let Some(db) = connect_test_database("request_only_identity").await else {
        return;
    };
    let state = test_app_state(db.clone());
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let actor_id = uuid::Uuid::new_v4().to_string();
    let mut actor = test_user(&actor_id, UserType::Person);
    let password = format!("Test-1!{}", uuid::Uuid::new_v4());
    actor.password_hash = Some(crate::crypto::password::hash_password(&password).unwrap());
    let email = actor.email.clone();
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(actor)
        .await
        .unwrap();
    let totp = if mfa {
        let setup = mfa_service::setup_totp(&db, &state.encryption_keys, &actor_id, &email)
            .await
            .unwrap();
        let totp = totp_rs::TOTP::from_url(setup.qr_code_url).unwrap();
        mfa_service::verify_totp_setup(
            &db,
            &state.encryption_keys,
            &setup.factor_id,
            &actor_id,
            &totp.generate_current().unwrap(),
        )
        .await
        .unwrap();
        db.collection::<User>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &actor_id},
                doc! {"$set": {"mfa_enabled": true}},
            )
            .await
            .unwrap();
        Some(totp)
    } else {
        None
    };
    let request = device::initiate_v2(
        &db,
        state.auth_device_hmac_key.as_slice(),
        Default::default(),
        true,
    )
    .await
    .unwrap();
    let origin = state.config.frontend_url.clone();
    let (_, router) = crate::routes::build_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router
                .with_state(state)
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::new();
    let begun = client
        .post(format!("{base}/auth/approval"))
        .header("Origin", &origin)
        .json(&json!({"flow":"device", "user_code":request.user_code, "keep_signed_in":keep}))
        .send()
        .await
        .unwrap();
    assert_eq!(begun.status(), StatusCode::OK);
    let cookie = begun.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let begun: Value = begun.json().await.unwrap();
    let id = begun["id"].as_str().unwrap();
    let approval = format!("{base}/auth/approval/{id}");
    let premature = client
        .post(format!("{approval}/approve"))
        .header("Origin", &origin)
        .header("Cookie", &cookie)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(premature.status(), StatusCode::UNAUTHORIZED);
    let wrong_cookie = client.get(&approval).send().await.unwrap();
    assert_eq!(wrong_cookie.status(), StatusCode::UNAUTHORIZED);
    let cross_site = client
        .post(format!("{approval}/password"))
        .header("Origin", "https://untrusted.example")
        .header("Cookie", &cookie)
        .json(&json!({"email":email, "password":password}))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_site.status(), StatusCode::FORBIDDEN);
    let mut signed = client
        .post(format!("{approval}/password"))
        .header("Origin", &origin)
        .header("Cookie", &cookie)
        .json(&json!({"email":email, "password":password, "client":"token"}))
        .send()
        .await
        .unwrap();
    assert_eq!(signed.status(), StatusCode::OK);
    if let Some(totp) = totp {
        assert!(signed.headers().get(header::SET_COOKIE).is_none());
        let pending: Value = signed.json().await.unwrap();
        assert_eq!(pending["verified"], false);
        assert_eq!(pending["mfa_required"], true);
        assert_eq!(
            db.collection::<Session>("sessions")
                .count_documents(doc! {"user_id": &actor_id})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            client
                .get(format!("{approval}/inventory"))
                .header("Cookie", &cookie)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let invalid = client
            .post(format!("{approval}/mfa"))
            .header("Origin", &origin)
            .header("Cookie", &cookie)
            .json(&json!({"code":"invalid"}))
            .send()
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
        signed = client
            .post(format!("{approval}/mfa"))
            .header("Origin", &origin)
            .header("Cookie", &cookie)
            .json(&json!({"code":totp.generate_current().unwrap()}))
            .send()
            .await
            .unwrap();
        assert_eq!(signed.status(), StatusCode::OK);
    }
    let session_cookie = signed
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("nyx_session="))
        .map(|v| v.split(';').next().unwrap().to_string());
    assert_eq!(session_cookie.is_some(), keep);
    let signed: Value = signed.json().await.unwrap();
    assert_eq!(signed["verified"], true);
    assert!(signed.get("access_token").is_none());
    assert_eq!(
        db.collection::<Session>("sessions")
            .count_documents(doc! {"user_id": &actor_id})
            .await
            .unwrap(),
        u64::from(keep)
    );
    let cookie = session_cookie.map_or(cookie.clone(), |s| format!("{cookie}; {s}"));
    let account = client
        .get(format!("{base}/users/me"))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(
        account.status(),
        if keep {
            StatusCode::OK
        } else {
            StatusCode::UNAUTHORIZED
        }
    );
    let denied = client
        .post(format!("{approval}/deny"))
        .header("Origin", &origin)
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::OK);
    assert_eq!(
        client
            .get(&approval)
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let replay = client
        .post(format!("{approval}/approve"))
        .header("Origin", &origin)
        .header("Cookie", &cookie)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        client
            .get(format!("{base}/users/me"))
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        if keep {
            StatusCode::OK
        } else {
            StatusCode::UNAUTHORIZED
        }
    );
    server.abort();
}

#[tokio::test]
async fn request_only_proof_never_becomes_an_account_session() {
    exercise(false, false).await;
}

#[tokio::test]
async fn persistent_choice_keeps_browser_signed_in_after_decision() {
    exercise(true, false).await;
}

#[tokio::test]
async fn request_only_mfa_is_required_before_authorization() {
    exercise(false, true).await;
}

#[tokio::test]
async fn persistent_mfa_creates_session_only_after_second_factor() {
    exercise(true, true).await;
}

#[tokio::test]
async fn social_completion_is_request_bound_and_cannot_replay_or_revive_cancelled_context() {
    let Some(db) = connect_test_database("approval_social").await else {
        return;
    };
    let state = test_app_state(db.clone());
    let actor = test_user(&uuid::Uuid::new_v4().to_string(), UserType::Person);
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(&actor)
        .await
        .unwrap();
    let request = device::initiate_v2(
        &db,
        state.auth_device_hmac_key.as_slice(),
        Default::default(),
        true,
    )
    .await
    .unwrap();
    let (row, secret) = service::begin(
        &db,
        state.auth_device_hmac_key.as_slice(),
        LoginFlow::Device,
        &request.user_code,
        false,
    )
    .await
    .unwrap();
    let csrf = format!("approval:{}:random-state", row.id);
    service::bind_social(&db, &row, &csrf).await.unwrap();
    let result = social_complete(
        &state,
        &csrf,
        &actor,
        &HeaderMap::new(),
        "127.0.0.1:1234".parse().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(result.0, StatusCode::FOUND);
    assert!(result.1.get(header::SET_COOKIE).is_none());
    let target = url::Url::parse(result.1[header::LOCATION].to_str().unwrap()).unwrap();
    assert_eq!(target.path(), "/login/device");
    assert_eq!(target.query_pairs().count(), 1);
    assert_eq!(
        db.collection::<Session>("sessions")
            .count_documents(doc! {"user_id": &actor.id})
            .await
            .unwrap(),
        0
    );
    let verified = service::load(&db, state.auth_device_hmac_key.as_slice(), &row.id, &secret)
        .await
        .unwrap();
    assert!(verified.verified);
    assert!(
        social_complete(
            &state,
            &csrf,
            &actor,
            &HeaderMap::new(),
            "127.0.0.1:1234".parse().unwrap()
        )
        .await
        .is_err()
    );
    let (other, _) = service::begin(
        &db,
        state.auth_device_hmac_key.as_slice(),
        LoginFlow::Device,
        &request.user_code,
        false,
    )
    .await
    .unwrap();
    assert!(
        service::load(
            &db,
            state.auth_device_hmac_key.as_slice(),
            &other.id,
            &secret
        )
        .await
        .is_err()
    );
    let csrf = format!("approval:{}:cancelled-state", other.id);
    service::bind_social(&db, &other, &csrf).await.unwrap();
    service::close(&db, &other).await.unwrap();
    assert!(
        social_complete(
            &state,
            &csrf,
            &actor,
            &HeaderMap::new(),
            "127.0.0.1:1234".parse().unwrap()
        )
        .await
        .is_err()
    );
}
