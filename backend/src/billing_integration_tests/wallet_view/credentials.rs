use super::*;
use crate::crypto::{jwt, token::hash_token};
use crate::models::session::{Session, SessionPurpose};
use crate::mw::auth::AuthUser;
use crate::services::{
    consent_service, key_service, oauth_client_service, service_account_service,
};

async fn session_credentials(state: &crate::AppState, owner: &str) -> (String, String) {
    let sid = Uuid::new_v4().to_string();
    let cookie = Uuid::new_v4().to_string();
    state
        .db
        .collection::<Session>("sessions")
        .insert_one(Session {
            id: sid.clone(),
            user_id: owner.into(),
            token_hash: hash_token(&cookie),
            purpose: Some(SessionPurpose::Authenticated),
            mfa_method: None,
            mfa_assured: Some(true),
            mfa_exemption: None,
            ip_address: None,
            user_agent: None,
            expires_at: Utc::now() + Duration::hours(1),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        })
        .await
        .unwrap();
    let rbac = jwt::RbacClaimData {
        roles: None,
        groups: None,
        permissions: None,
        sid: Some(sid),
    };
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(owner).unwrap(),
        "openid account:read",
        Some(&rbac),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    (token, format!("nyx_session={cookie}"))
}

// This narrowly mounted probe uses the real verified extractor and an allowed
// management-read path. It proves negative credentials are live, independently
// of the production router's early credential rejection layers.
fn extractor_probe(state: &crate::AppState) -> Router {
    Router::new().route("/api/v1/orgs", get(|auth: AuthUser| async move {
        Json(json!({"method": format!("{:?}", auth.auth_method), "user_id": auth.user_id.to_string()}))
    })).with_state(state.clone())
}

async fn oauth_token(state: &crate::AppState, owner: &str) -> String {
    let (client, _) = oauth_client_service::create_client(
        &state.db,
        "Wallet view openid-only negative",
        &["https://client.example/callback".into()],
        "public",
        owner,
        "",
        "openid",
        crate::models::oauth_client::ScopeProvenance::Explicit,
        false,
        None,
        None,
        &[],
    )
    .await
    .unwrap();
    let consent = consent_service::grant_consent(&state.db, owner, &client.id, "openid")
        .await
        .unwrap();
    let reference = consent_service::protected_reference(&consent).unwrap();
    jwt::generate_oauth_access_token_with_consent(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(owner).unwrap(),
        "openid",
        None,
        None,
        None,
        None,
        None,
        &client.id,
        Some(&reference),
    )
    .unwrap()
}

#[tokio::test]
async fn billing_wallet_view_genuinely_minted_credential_matrix_and_human_controls() {
    let db = connect_test_database("wallet_view_live_credentials")
        .await
        .unwrap();
    let owner = insert_owner(&db).await;
    let lago = Arc::new(NoLago::default());
    let state = view_state(db.clone(), lago.clone());
    insert_wallet(&db, &owner).await;
    let (first_party, cookie) = session_credentials(&state, &owner).await;
    let key = key_service::create_api_key(
        &db,
        &owner,
        "Wallet relay parent",
        "proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        Some("codex"),
        None,
    )
    .await
    .unwrap();
    let relay = jwt::generate_relay_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "proxy",
        None,
        &jwt::RelayAgentScope {
            api_key_id: key.id,
            api_key_name: key.name,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: false,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    let (sa, secret) = service_account_service::create_service_account(
        &db,
        "Wallet view General SA",
        None,
        "account:read",
        &[],
        None,
        &owner,
    )
    .await
    .unwrap();
    assert_eq!(
        sa.purpose,
        crate::models::service_account::ServiceAccountPurpose::General
    );
    let sa_token = service_account_service::authenticate_client_credentials(
        &db,
        &state.config,
        &state.jwt_keys,
        &sa.client_id,
        &secret,
        Some("account:read"),
    )
    .await
    .unwrap()
    .access_token;
    let oauth = oauth_token(&state, &owner).await;
    let app = mounted_app(&state);
    let probe = extractor_probe(&state);
    for (name, token, method) in [
        ("relay", &relay, "Relay"),
        ("service account", &sa_token, "ServiceAccount"),
        ("OAuth openid-only", &oauth, "AccessToken"),
    ] {
        for scheme in ["Bearer", "DPoP"] {
            let header = format!("{scheme} {token}");
            let (status, identity) =
                read_headers(&probe, "/api/v1/orgs", &[("authorization", &header)]).await;
            assert_eq!(status, StatusCode::OK, "{name}/{scheme}: {identity}");
            assert_eq!(identity["method"], method);
            let (status, body) = read_headers(&app, VIEW, &[("authorization", &header)]).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{name}/{scheme}: {body}");
        }
    }
    for headers in [
        vec![("authorization", format!("Bearer {first_party}"))],
        vec![("cookie", cookie)],
    ] {
        let borrowed: Vec<_> = headers
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let (status, body) = read_headers(&app, VIEW, &borrowed).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_balances(&body);
    }
    lago.assert_unused();
}

#[tokio::test]
async fn billing_wallet_view_rejects_non_person_bearer_and_cookie_subjects() {
    let db = connect_test_database("wallet_view_org_subject")
        .await
        .unwrap();
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    insert_wallet(&db, &org).await;
    let lago = Arc::new(NoLago::default());
    let state = view_state(db.clone(), lago.clone());
    let (token, cookie) = session_credentials(&state, &org).await;
    let app = mounted_app(&state);
    let before = financial_state(&db).await;
    for headers in [
        vec![("authorization", format!("Bearer {token}"))],
        vec![("cookie", cookie)],
    ] {
        let borrowed: Vec<_> = headers
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let (status, body) = read_headers(&app, VIEW, &borrowed).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }
    assert_eq!(financial_state(&db).await, before);
    lago.assert_unused();
}

#[tokio::test]
async fn billing_wallet_view_live_service_delegation_authenticates_allowed_route_then_is_denied() {
    let db = connect_test_database("wallet_view_live_delegation")
        .await
        .unwrap();
    let owner = insert_owner(&db).await;
    let lago = Arc::new(NoLago::default());
    let state = view_state(db, lago.clone());
    let token = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "account:read",
        "wallet-test-service",
        300,
        None,
    )
    .unwrap();
    let claims = jwt::verify_token(&state.jwt_keys, &state.config, &token).unwrap();
    assert_eq!(
        claims.delegation_origin,
        Some(jwt::DelegationOrigin::Service)
    );
    crate::services::oauth_token_authority_service::validate_claims(&state.db, &claims)
        .await
        .unwrap();
    let app = mounted_app(&state);
    for scheme in ["Bearer", "DPoP"] {
        let header = format!("{scheme} {token}");
        let headers = [("authorization", header.as_str())];
        let (status, body) = read_headers(&app, "/api/v1/orgs", &headers).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "valid service delegation on allowed route: {body}"
        );
        let (status, body) = read_headers(&app, VIEW, &headers).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{scheme}: {body}");
    }
    lago.assert_unused();
}
