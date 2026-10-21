use super::*;
use crate::{
    crypto::jwt,
    handlers::oauth,
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        user::{COLLECTION_NAME as USERS, UserType},
    },
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};
use axum::{extract::State, http::StatusCode};
use axum_extra::extract::Form;
use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};

const OWNER: &str = "12345678-1234-4123-8123-1234567890ab";
const SECRET: &str = "creator-introspection-fixture";

fn client() -> OauthClient {
    bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "client_name": "Gateway",
        "client_secret_hash": hex::encode(Sha256::digest(SECRET.as_bytes())),
        "redirect_uris": [], "allowed_scopes": "openid", "grant_types": "authorization_code",
        "client_type": "confidential", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    })
    .unwrap()
}

fn mint(state: &AppState) -> (Zeroizing<String>, Claims) {
    let token = creator_bearer(state, OWNER).unwrap();
    let claims = jwt::verify_token(&state.jwt_keys, &state.config, &token).unwrap();
    (token, claims)
}

async fn fixture() -> (AppState, OauthClient, DownstreamService) {
    let state = test_app_state(connect_transaction_test_database("creator_grant").await);
    let client = client();
    state
        .db
        .collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .insert_one(&client)
        .await
        .unwrap();
    let mut user = test_user(OWNER, UserType::Person);
    let role = Uuid::new_v4().to_string();
    let group = Uuid::new_v4().to_string();
    user.role_ids = vec![role.clone()];
    user.group_ids = vec![group.clone()];
    state.db.collection(USERS).insert_one(user).await.unwrap();
    state.db.collection::<Document>(crate::models::role::COLLECTION_NAME).insert_one(doc! {
        "_id": role, "slug": "baseline", "name": "Baseline", "permissions": ["nyxid:profile:read"],
        "is_default": false, "is_system": false,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    state
        .db
        .collection::<Document>(crate::models::group::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": group, "slug": "members", "name": "Members", "role_ids": [],
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = GATEWAY_SLUG.into();
    service.requires_user_credential = false;
    service.introspection_client_ids = Some(vec![client.id.clone()]);
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    (state, client, service)
}

async fn introspect(
    state: &AppState,
    client: &OauthClient,
    secret: &str,
    token: &str,
) -> (StatusCode, Value) {
    let request = serde_json::from_value::<oauth::IntrospectRequest>(json!({
        "token": token, "client_id": client.id, "client_secret": secret,
    }))
    .unwrap();
    let response = oauth::introspect(State(state.clone()), Form(request)).await;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn creator_mint_introspection_adds_only_gateway_permissions_for_registered_caller() {
    let (state, client, _) = fixture().await;
    let (token, claims) = mint(&state);
    assert!(is_creator_claims(&claims));
    assert_eq!(claims.act.as_ref().unwrap().sub, CREATOR_ACTOR);
    assert_eq!(claims.exp - claims.iat, CREATOR_TOKEN_TTL_SECS);
    let baseline = super::super::rbac_helpers::resolve_user_rbac(&state.db, OWNER)
        .await
        .unwrap();
    let (status, body) = introspect(&state, &client, SECRET, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], true);
    assert_eq!(
        body["permissions"],
        json!([
            "nyxid:profile:read",
            "cma-eg:channels:read",
            "cma-eg:channels:create",
            "cma-eg:channels:update",
            "cma-eg:channels:delete"
        ])
    );
    assert_eq!(body["roles"], json!(baseline.role_slugs));
    assert_eq!(body["groups"], json!(baseline.group_slugs));
    assert!(body.get("act").is_none());
    let ordinary = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(OWNER).unwrap(),
        crate::mw::auth::ACCOUNT_READ_SCOPE,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, body) = introspect(&state, &client, SECRET, &ordinary).await;
    assert_eq!(body["permissions"], json!(baseline.permissions));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn creator_predicate_rejects_every_non_creator_marker_and_invalid_lifetime() {
    let state = crate::test_utils::test_app_state_no_db().await;
    let (_, claims) = mint(&state);
    let original = serde_json::to_value(&claims).unwrap();
    let mut exclusions = vec![
        ("token_type", json!("refresh")),
        ("token_type", json!("id")),
        ("delegated", Value::Null),
        ("delegated", json!(false)),
        ("delegation_origin", Value::Null),
        ("delegation_origin", json!("oauth")),
        ("act", Value::Null),
        ("act", json!({"sub":"other"})),
        ("client_id", json!("")),
        ("sid", json!("")),
        ("sgen", json!(0)),
        (
            "protected_consent",
            json!({"consent_id":"c", "consent_revision":"r"}),
        ),
        (
            "oauth_lineage",
            json!({"consents":[], "source_session_id":null}),
        ),
        ("cnf", json!({})),
        ("relay_api_key_id", json!("")),
        ("relay_api_key_name", json!("")),
    ];
    for value in [
        "",
        " account:read",
        "account:read ",
        "account:read proxy",
        "account:read account:read",
        "proxy",
    ] {
        exclusions.push(("scope", json!(value)));
    }
    for field in [
        "sa",
        "relay",
        "assistant_forward",
        "relay_allow_all_services",
        "relay_allow_all_nodes",
        "allow_all_services",
        "allow_all_nodes",
    ] {
        for value in [false, true] {
            exclusions.push((field, json!(value)));
        }
    }
    for field in [
        "roles",
        "groups",
        "permissions",
        "resources",
        "allowed_service_ids",
        "allowed_node_ids",
        "relay_allowed_service_ids",
        "relay_allowed_node_ids",
    ] {
        exclusions.push((field, json!([])));
        exclusions.push((field, json!(["value"])));
    }
    for (field, value) in exclusions {
        let mut changed = original.clone();
        changed[field] = value;
        let changed: Claims = serde_json::from_value(changed).unwrap();
        assert!(!is_creator_claims(&changed), "accepted marker {field}");
    }
    for (iat, exp) in [
        (claims.exp, claims.exp),
        (claims.exp + 1, claims.exp),
        (i64::MIN, i64::MAX),
        (claims.iat, claims.iat + 121),
        (Utc::now().timestamp() - 120, Utc::now().timestamp()),
    ] {
        let mut changed = claims.clone();
        changed.iat = iat;
        changed.exp = exp;
        assert!(
            !is_creator_claims(&changed),
            "accepted lifetime {iat}..{exp}"
        );
    }
    let mut shortest = claims.clone();
    shortest.iat = shortest.exp - 1;
    assert!(is_creator_claims(&shortest));
}

#[tokio::test]
async fn other_real_service_mints_and_ineligible_clients_do_not_read_catalog() {
    let state = crate::test_utils::test_app_state_no_db().await;
    let (_, creator) = mint(&state);
    let owner = Uuid::parse_str(OWNER).unwrap();
    let mut aevatar = crate::models::downstream_service::test_helpers::dummy_service();
    aevatar.slug = "aevatar".into();
    aevatar.delegation_token_scope = "proxy".into();
    let header = crate::handlers::assistant::build_forward_authorization(
        &state,
        &crate::test_utils::test_auth_user(OWNER),
        &aevatar,
    )
    .unwrap();
    let assistant = header.to_str().unwrap().strip_prefix("Bearer ").unwrap();
    let proxy = super::super::identity_service::generate_proxy_delegation_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        crate::mw::auth::ACCOUNT_READ_SCOPE,
        "nyxbot",
        None,
        None,
    )
    .unwrap();
    let mut proxy_claims = jwt::verify_token(&state.jwt_keys, &state.config, &proxy).unwrap();
    assert_eq!(proxy_claims.act.as_ref().unwrap().sub, "nyxbot");
    proxy_claims.exp = proxy_claims.iat + CREATOR_TOKEN_TTL_SECS;
    assert!(!is_creator_claims(&proxy_claims));
    assert!(
        creator_gateway_permissions(&state.db, &proxy_claims, &client())
            .await
            .unwrap()
            .is_empty()
    );
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "proxy",
        "service",
        jwt::MCP_DELEGATION_TOKEN_TTL_SECS,
        None,
    )
    .unwrap();
    // This state has no reachable MongoDB. Any accidental catalog read fails.
    for token in [assistant, proxy.as_str(), delegated.as_str()] {
        let claims = jwt::verify_token(&state.jwt_keys, &state.config, token).unwrap();
        assert!(!is_creator_claims(&claims));
        assert!(
            creator_gateway_permissions(&state.db, &claims, &client())
                .await
                .unwrap()
                .is_empty()
        );
    }
    for (active, kind) in [(false, "confidential"), (true, "public")] {
        let mut caller = client();
        caller.is_active = active;
        caller.client_type = kind.into();
        assert!(
            creator_gateway_permissions(&state.db, &creator, &caller)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn reserved_creator_actor_requires_dedicated_mint_and_is_not_a_valid_slug() {
    let state = crate::test_utils::test_app_state_no_db().await;
    let owner = Uuid::parse_str(OWNER).unwrap();
    let (_, claims) = mint(&state);
    assert!(is_creator_claims(&claims));
    assert_eq!(claims.act.as_ref().unwrap().sub, CREATOR_ACTOR);
    assert!(crate::handlers::admin_helpers::validate_slug(CREATOR_ACTOR).is_err());
    assert!(super::super::user_service_service::validate_slug(CREATOR_ACTOR).is_err());
    assert!(crate::handlers::admin_helpers::validate_slug("nyxbot").is_ok());
    assert!(super::super::user_service_service::validate_slug("nyxbot").is_ok());
    for ttl in [CREATOR_TOKEN_TTL_SECS, jwt::MCP_DELEGATION_TOKEN_TTL_SECS] {
        assert!(matches!(
            jwt::generate_delegated_access_token(
                &state.jwt_keys,
                &state.config,
                &owner,
                crate::mw::auth::ACCOUNT_READ_SCOPE,
                CREATOR_ACTOR,
                ttl,
                None,
            ),
            Err(AppError::ValidationError(_))
        ));
    }
    assert!(matches!(
        jwt::generate_delegated_access_token_for_client(
            &state.jwt_keys,
            &state.config,
            &owner,
            crate::mw::auth::ACCOUNT_READ_SCOPE,
            CREATOR_ACTOR,
            None,
            CREATOR_TOKEN_TTL_SECS,
            None,
        ),
        Err(AppError::ValidationError(_))
    ));
}

#[tokio::test]
async fn creator_grant_requires_authenticated_active_associated_confidential_client() {
    let (state, mut caller, service) = fixture().await;
    let (token, claims) = mint(&state);
    let (_, body) = introspect(&state, &caller, "wrong", &token).await;
    assert_eq!(body, json!({"active":false}));
    for (kind, active) in [
        ("public", true),
        ("confidential", false),
        ("confidential", true),
    ] {
        caller.client_type = kind.into();
        caller.is_active = active;
        state
            .db
            .collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
            .replace_one(doc! {"_id":&caller.id}, &caller)
            .await
            .unwrap();
        if kind == "confidential" && active {
            state
                .db
                .collection::<Document>(SERVICES)
                .update_one(
                    doc! {"_id":&service.id},
                    doc! {"$set":{"introspection_client_ids":[Uuid::new_v4().to_string()]}},
                )
                .await
                .unwrap();
        }
        let (status, body) = introspect(&state, &caller, SECRET, &token).await;
        assert_eq!(status, StatusCode::OK);
        if active {
            assert_eq!(body["active"], true);
            assert_eq!(body["permissions"], json!(["nyxid:profile:read"]));
        } else {
            assert_eq!(body, json!({"active":false}));
        }
        assert!(
            creator_gateway_permissions(&state.db, &claims, &caller)
                .await
                .unwrap()
                .is_empty()
        );
    }
    state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&service.id},
            doc! {"$unset":{"introspection_client_ids":""}},
        )
        .await
        .unwrap();
    let (_, body) = introspect(&state, &caller, SECRET, &token).await;
    assert_eq!(body["permissions"], json!(["nyxid:profile:read"]));
    let unrelated = client();
    state
        .db
        .collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .insert_one(&unrelated)
        .await
        .unwrap();
    let (status, body) = introspect(&state, &unrelated, SECRET, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], true);
    assert_eq!(body["permissions"], json!(["nyxid:profile:read"]));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn creator_catalog_missing_inactive_misconfigured_or_read_failure_is_unavailable() {
    let (state, caller, service) = fixture().await;
    let (token, _) = mint(&state);
    for patch in [
        doc! {"is_active":false},
        doc! {"is_active":true,"requires_user_credential":true},
    ] {
        state
            .db
            .collection::<Document>(SERVICES)
            .update_one(doc! {"_id":&service.id}, doc! {"$set":patch})
            .await
            .unwrap();
        let (status, body) = introspect(&state, &caller, SECRET, &token).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "oauth_authority_unavailable");
        assert!(body.get("active").is_none());
    }
    state
        .db
        .collection::<Document>(SERVICES)
        .drop()
        .await
        .unwrap();
    assert_eq!(
        introspect(&state, &caller, SECRET, &token).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    // A failing catalog view injects a real, collection-local MongoDB read
    // error without global failpoints that could disrupt concurrent tests.
    state
        .db
        .collection::<Document>("catalog_source")
        .insert_one(doc! {"slug":"cmaeg", "is_active":true})
        .await
        .unwrap();
    state
        .db
        .create_collection(SERVICES)
        .view_on("catalog_source")
        .pipeline(vec![doc! {"$set":{"broken":{"$divide":[1,0]}}}])
        .await
        .unwrap();
    let (status, body) = introspect(&state, &caller, SECRET, &token).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "oauth_authority_unavailable");
    // Non-creator introspection still succeeds against the broken catalog.
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(OWNER).unwrap(),
        "openid",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        introspect(&state, &caller, SECRET, &token).await.0,
        StatusCode::OK
    );
    state.db.drop().await.unwrap();
}
