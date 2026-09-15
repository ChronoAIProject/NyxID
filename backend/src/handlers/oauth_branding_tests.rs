use super::*;
use crate::handlers::oauth_authorize_context::{self as context_handler, ContextQuery};
use crate::handlers::oauth_branding::{self as branding_handler, VerifyRequest};
use crate::models::oauth_authorize_context::COLLECTION_NAME as CONTEXTS;
use crate::services::{
    oauth_authorize_context_service as contexts, oauth_branding_service as branding,
};
use axum::extract::ConnectInfo;
use chrono::{Duration, Utc};

async fn start(f: &Fixture, p: &AuthorizeQuery) -> String {
    let response = authorize_inner(&f.state, OptionalAuthUser(None), p, true, None)
        .await
        .unwrap();
    let url = url::Url::parse(&location(&response)).unwrap();
    assert!(url.path().starts_with("/connect/app/start/"));
    url.path_segments().unwrap().next_back().unwrap().into()
}
async fn metadata(f: &Fixture, ctx: &str) -> AppResult<Response> {
    context_handler::get(
        State(f.state.clone()),
        ConnectInfo("127.0.0.1:5000".parse().unwrap()),
        HeaderMap::new(),
        Query(ContextQuery { ctx: ctx.into() }),
    )
    .await
}
async fn resume(f: &Fixture, ctx: &str, auth: AuthUser) -> Response {
    context_handler::resume(
        State(f.state.clone()),
        OptionalAuthUser(Some(auth)),
        ConnectInfo("127.0.0.1:5000".parse().unwrap()),
        HeaderMap::new(),
        Query(ContextQuery { ctx: ctx.into() }),
    )
    .await
    .unwrap()
}
async fn body(response: Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}
fn png() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[tokio::test]
async fn app_branding_db_login_context_is_gated_and_non_gate_login_unchanged() {
    let Some(f) = fixture("branding_gate").await else {
        return;
    };
    let p = params(&f);
    let expected = format!(
        "{}/login?return_to={}",
        f.state.config.frontend_url,
        urlencoding::encode(&build_authorize_url(&f.state.config.frontend_url, &p))
    );
    let ordinary = authorize_inner(&f.state, OptionalAuthUser(None), &p, true, None)
        .await
        .unwrap();
    assert_eq!(location(&ordinary), expected);
    assert_eq!(count(&f, CONTEXTS).await, 0);
    gated(&f, false, false).await;
    let ctx = start(&f, &p).await;
    let (record, _) = contexts::load(&f.state, &ctx).await.unwrap();
    assert_eq!(record.expires_at - record.created_at, Duration::minutes(5));
    assert_eq!(record.authorize_params.state, p.state);
    let response = metadata(&f, &ctx).await.unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let json = body(response).await;
    assert_eq!(json["client_name"], "Test app");
    assert_eq!(json["destination"], "app.example");
    assert_eq!(json["verified"], false);
    assert!(json.get("client_id").is_none());
    let mut none = p.clone();
    none.prompt = Some("none".into());
    let response = authorize_inner(&f.state, OptionalAuthUser(None), &none, true, None)
        .await
        .unwrap();
    assert!(location(&response).contains("error=login_required"));
    assert_eq!(count(&f, CONTEXTS).await, 1);
    crate::services::app_connect_rollout::set_client_capability(&f.state.db, &f.app.id, false)
        .await
        .unwrap();
    assert!(matches!(
        metadata(&f, &ctx).await,
        Err(AppError::NotFound(_))
    ));
    let dark = authorize_inner(&f.state, OptionalAuthUser(None), &p, true, None)
        .await
        .unwrap();
    assert_eq!(location(&dark), expected);
}

#[tokio::test]
async fn app_branding_db_context_tamper_expiry_activation_and_rate_limit() {
    let Some(f) = fixture("branding_context").await else {
        return;
    };
    gated(&f, false, false).await;
    let ctx = start(&f, &params(&f)).await;
    let (record, _) = contexts::load(&f.state, &ctx).await.unwrap();
    assert!(matches!(
        metadata(&f, &(ctx.clone() + "tampered")).await,
        Err(AppError::NotFound(_))
    ));
    let now = Utc::now().timestamp();
    let expired = jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256), &serde_json::json!({
        "sub": record.id, "iss": f.state.config.jwt_issuer, "aud": "nyxid/oauth-authorize-context", "token_type": "oauth_authorize_context", "iat": now - 400, "exp": now - 100
    }), &f.state.jwt_keys.encoding).unwrap();
    assert!(matches!(
        metadata(&f, &expired).await,
        Err(AppError::NotFound(_))
    ));
    f.state
        .db
        .collection::<Document>("oauth_clients")
        .update_one(
            doc! { "_id": &f.app.id },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert!(matches!(
        metadata(&f, &ctx).await,
        Err(AppError::NotFound(_))
    ));
    f.state
        .db
        .collection::<Document>("oauth_clients")
        .update_one(
            doc! { "_id": &f.app.id },
            doc! { "$set": { "is_active": true } },
        )
        .await
        .unwrap();
    f.state.db.collection::<Document>(CONTEXTS).update_one(doc! { "_id": &record.id }, doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } }).await.unwrap();
    assert!(matches!(
        metadata(&f, &ctx).await,
        Err(AppError::NotFound(_))
    ));
    for _ in 0..30 {
        let _ = metadata(&f, "invalid").await;
    }
    assert!(matches!(
        metadata(&f, &ctx).await,
        Err(AppError::RateLimited)
    ));
}

#[tokio::test]
async fn app_branding_db_par_login_resumes_stored_request_after_consumption() {
    let Some(f) = fixture("branding_par").await else {
        return;
    };
    gated(&f, false, false).await;
    let p = params(&f);
    let (request_uri, _) = par_service::create_request(
        &f.state.db,
        &f.app.id,
        "code",
        &p.redirect_uri,
        p.scope.as_deref(),
        p.state.as_deref(),
        p.code_challenge.as_deref(),
        p.code_challenge_method.as_deref(),
        p.nonce.as_deref(),
        None,
        Some("force"),
        &[],
        None,
        None,
    )
    .await
    .unwrap();
    let raw: AuthorizeQuery =
        serde_json::from_value(serde_json::json!({"client_id":f.app.id,"request_uri":request_uri}))
            .unwrap();
    let response = authorize(
        State(f.state.clone()),
        OptionalAuthUser(None),
        HeaderMap::new(),
        Ok(Query(raw)),
    )
    .await
    .unwrap();
    let url = url::Url::parse(&location(&response)).unwrap();
    assert!(url.path().starts_with("/connect/app/start/"));
    let ctx = url.path_segments().unwrap().next_back().unwrap();
    assert_eq!(count(&f, "pushed_authorization_requests").await, 0);
    let response = resume(&f, ctx, human(&f)).await;
    let url = url::Url::parse(&location(&response)).unwrap();
    let id = url.path_segments().unwrap().next_back().unwrap();
    let link = app_links::load(&f.state, id, &f.auth.user_id.to_string())
        .await
        .unwrap();
    let AppConnectOrigin::Authorize {
        authorize_params: stored,
        ..
    } = link.origin
    else {
        panic!("authorize");
    };
    assert_eq!(stored.state, p.state);
    assert_eq!(stored.nonce, p.nonce);
    assert_eq!(stored.code_challenge, p.code_challenge);
    assert_eq!(stored.redirect_uri, p.redirect_uri);
    assert_eq!(stored.nyx_connect.as_deref(), Some("force"));
}

#[tokio::test]
async fn app_branding_db_prompt_login_requires_new_human_session() {
    let Some(f) = fixture("branding_login").await else {
        return;
    };
    gated(&f, false, false).await;
    let mut p = params(&f);
    p.prompt = Some("login consent".into());
    let ctx = start(&f, &p).await;
    let (record, _) = contexts::load(&f.state, &ctx).await.unwrap();
    let session_id = uuid::Uuid::new_v4();
    let mut auth = human(&f);
    auth.session_id = Some(session_id);
    f.state.db.collection::<Document>("sessions").insert_one(doc! {
        "_id": session_id.to_string(), "user_id": auth.user_id.to_string(), "token_hash": "test", "revoked": false,
        "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::hours(1)),
        "created_at": bson::DateTime::from_chrono(record.created_at - Duration::minutes(1)), "last_active_at": bson::DateTime::now(),
    }).await.unwrap();
    assert!(location(&resume(&f, &ctx, auth.clone()).await).contains("/connect/app/start/"));
    assert_eq!(count(&f, LINKS).await, 0);
    f.state
        .db
        .collection::<Document>("sessions")
        .update_one(
            doc! { "_id": session_id.to_string() },
            doc! { "$set": { "created_at": bson::DateTime::now() } },
        )
        .await
        .unwrap();
    assert!(!location(&resume(&f, &ctx, auth).await).contains("/start/"));
    assert_eq!(count(&f, LINKS).await, 1);
    assert!(
        context_handler::resume(
            State(f.state.clone()),
            OptionalAuthUser(Some(f.auth.clone())),
            ConnectInfo("127.0.0.1:5000".parse().unwrap()),
            HeaderMap::new(),
            Query(ContextQuery { ctx })
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn app_branding_db_revisions_verification_and_asset_cleanup() {
    let Some(f) = fixture("branding_revision").await else {
        return;
    };
    let client = branding::upload_logo(&f.state.db, &f.app.id, &f.owner, png())
        .await
        .unwrap();
    assert_eq!(client.branding_revision, 1);
    assert!(branding::logo_url(&client).is_some());
    assert!(!branding::is_verified(&client));
    let first_id = client.logo_asset_id.clone().unwrap();
    let asset = branding_handler::asset(State(f.state.clone()), Path(first_id.clone()))
        .await
        .unwrap();
    assert_eq!(asset.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(
        asset.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let original = axum::body::to_bytes(asset.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(
        branding_handler::verify(
            State(f.state.clone()),
            human(&f),
            Path(f.app.id.clone()),
            Json(VerifyRequest {
                branding_revision: 1,
                verified: true
            })
        )
        .await
        .is_err()
    );
    let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("users")
        .update_one(
            doc! { "_id": f.auth.user_id.to_string() },
            doc! { "$set": { "role_ids": [roles.admin] } },
        )
        .await
        .unwrap();
    let marked = branding_handler::verify(
        State(f.state.clone()),
        human(&f),
        Path(f.app.id.clone()),
        Json(VerifyRequest {
            branding_revision: 1,
            verified: true,
        }),
    )
    .await
    .unwrap()
    .0;
    assert!(marked.verified);
    let unchanged = oauth_client_service::update_client_for_creator(
        &f.state.db,
        &f.app.id,
        &f.owner,
        Some("Test app"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(unchanged.branding_revision, 1);
    assert!(branding::is_verified(&unchanged));
    let renamed = oauth_client_service::update_client_for_creator(
        &f.state.db,
        &f.app.id,
        &f.owner,
        Some("$New name"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(renamed.client_name, "$New name");
    assert_eq!(renamed.branding_revision, 2);
    assert!(!branding::is_verified(&renamed));
    assert!(
        branding::verify(&f.state.db, &f.app.id, 1, true)
            .await
            .is_err()
    );
    oauth_client_service::update_handoff_blurb(
        &f.state.db,
        &f.app.id,
        &f.owner,
        "A helpful handoff",
    )
    .await
    .unwrap();
    let homepage = oauth_client_service::update_client_for_creator(
        &f.state.db,
        &f.app.id,
        &f.owner,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("https://branding.example.com"),
    )
    .await
    .unwrap();
    assert_eq!(homepage.branding_revision, 4);
    let replacement = branding::upload_logo(&f.state.db, &f.app.id, &f.owner, png())
        .await
        .unwrap();
    assert_eq!(replacement.branding_revision, 5);
    assert_ne!(replacement.logo_asset_id.as_ref().unwrap(), &first_id);
    assert_eq!(&original[..8], b"\x89PNG\r\n\x1a\n");
    assert!(matches!(
        branding::read_logo(&f.state.db, &first_id).await,
        Err(AppError::NotFound(_))
    ));
    assert!(
        branding::read_logo(&f.state.db, replacement.logo_asset_id.as_ref().unwrap())
            .await
            .is_ok()
    );
    assert_eq!(count(&f, "branding_assets.files").await, 1);
    let verified = branding::verify(&f.state.db, &f.app.id, 5, true)
        .await
        .unwrap();
    assert!(branding::is_verified(&verified));
    let unverified = branding::verify(&f.state.db, &f.app.id, 5, false)
        .await
        .unwrap();
    assert!(!branding::is_verified(&unverified));
    let renamed_by_admin = oauth_client_service::admin_update_client(
        &f.state.db,
        &f.app.id,
        oauth_client_service::AdminUpdateClient {
            client_name: Some("Admin reviewed name"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(renamed_by_admin.branding_revision, 6);
    let attempted_self_grant = serde_json::from_value(serde_json::json!({
        "branding_revision": 99, "branding_verified_revision": 99, "verified": true,
    }))
    .unwrap();
    let updated = crate::handlers::developer_apps::update_my_oauth_client(
        State(f.state.clone()),
        human(&f),
        Path(f.app.id.clone()),
        Json(attempted_self_grant),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(updated.branding.branding_revision, 6);
    assert!(!updated.branding.verified);
    let _ = crate::handlers::developer_apps::delete_my_oauth_client(
        State(f.state.clone()),
        human(&f),
        Path(f.app.id.clone()),
    )
    .await
    .unwrap();
    assert!(matches!(
        branding::read_logo(&f.state.db, replacement.logo_asset_id.as_ref().unwrap()).await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(count(&f, "branding_assets.files").await, 0);
}

#[tokio::test]
async fn app_branding_db_multipart_owner_acl_limits_and_rollout() {
    use axum::{
        Router,
        body::Body,
        extract::{DefaultBodyLimit, FromRequest},
        http::Request,
        routing::post,
    };
    use tower::ServiceExt;
    let Some(f) = fixture("branding_upload").await else {
        return;
    };
    let auth = human(&f);
    let router = Router::new()
        .route(
            "/logo/{id}",
            post(
                move |state: State<AppState>,
                      path: Path<String>,
                      multipart: axum::extract::Multipart| {
                    let auth = auth.clone();
                    async move { branding_handler::upload_logo(state, auth, path, multipart).await }
                },
            ),
        )
        .layer(DefaultBodyLimit::max(branding::MAX_LOGO_INPUT + 8192))
        .with_state(f.state.clone());
    let request = |bytes: Vec<u8>| {
        let mut payload = b"--test-boundary\r\nContent-Disposition: form-data; name=\"logo\"; filename=\"logo.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
        payload.extend(bytes);
        payload.extend(b"\r\n--test-boundary--\r\n");
        Request::post(format!("/logo/{}", f.app.id))
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=test-boundary",
            )
            .body(Body::from(payload))
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request(b"<svg/>".to_vec()))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(vec![0; branding::MAX_LOGO_INPUT + 1]))
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(png()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    crate::services::app_connect_rollout::set_client_capability(&f.state.db, &f.app.id, false)
        .await
        .unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(request(png()))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    // An unrelated human cannot upload against an org-owned app.
    let outsider = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
    let multipart = axum::extract::Multipart::from_request(request(png()), &())
        .await
        .unwrap();
    assert!(matches!(
        branding_handler::upload_logo(
            State(f.state.clone()),
            outsider,
            Path(f.app.id.clone()),
            multipart
        )
        .await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn app_branding_db_social_failure_preserves_context_then_resume_consumes_once() {
    let Some(f) = fixture("branding_consume").await else {
        return;
    };
    gated(&f, false, false).await;
    let ctx = start(&f, &params(&f)).await;
    let return_to = format!(
        "{}/oauth/authorize-context/resume?ctx={ctx}",
        f.state.config.frontend_url
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        format!("nyx_social_return_to={}", urlencoding::encode(&return_to))
            .parse()
            .unwrap(),
    );
    let error = crate::handlers::social_auth::callback(
        State(f.state.clone()),
        ConnectInfo("127.0.0.1:5000".parse().unwrap()),
        Path("google".into()),
        axum::extract::Query(crate::handlers::social_auth::CallbackQuery {
            code: None,
            state: None,
            error: Some("access_denied".into()),
            error_description: None,
        }),
        headers,
    )
    .await
    .unwrap_err();
    let error_url = url::Url::parse(error.1[header::LOCATION].to_str().unwrap()).unwrap();
    assert_eq!(error_url.path(), format!("/connect/app/start/{ctx}"));
    assert!(contexts::load(&f.state, &ctx).await.is_ok());
    let response = resume(&f, &ctx, human(&f)).await;
    assert!(location(&response).contains("/connect/app/"));
    assert_eq!(count(&f, LINKS).await, 1);
    assert!(matches!(
        metadata(&f, &ctx).await,
        Err(AppError::NotFound(_))
    ));
    let replay = context_handler::resume(
        State(f.state.clone()),
        OptionalAuthUser(Some(human(&f))),
        ConnectInfo("127.0.0.1:5000".parse().unwrap()),
        HeaderMap::new(),
        Query(ContextQuery { ctx }),
    )
    .await;
    assert!(matches!(replay, Err(AppError::NotFound(_))));
    assert_eq!(count(&f, LINKS).await, 1);
    assert_eq!(count(&f, CODES).await, 0);
}
