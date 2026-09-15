use super::super::app_requirements::tests::{
    Fixture, catalog, fixture, publish, requirement, service,
};
use super::*;
use crate::models::app_connect_link::COLLECTION_NAME as LINKS;

use crate::models::platform_settings::AppConnectRollout;
use crate::test_utils::{test_auth_user, test_user};
use mongodb::bson::{self, Document};
use uuid::Uuid;

fn human(f: &Fixture) -> AuthUser {
    let mut auth = f.auth.clone();
    auth.auth_method = AuthMethod::Session;
    auth.oauth_client_id = None;
    auth
}
fn peer() -> ConnectInfo<SocketAddr> {
    ConnectInfo("127.0.0.1:41000".parse().unwrap())
}
async fn started(f: &Fixture) -> (AppConnectLink, String) {
    let created = create(
        State(f.state.clone()),
        f.auth.clone(),
        Json(CreateRequest {
            callback_url: "https://app.example/callback".into(),
            state: "app-state".into(),
        }),
    )
    .await
    .unwrap()
    .0;
    let url = url::Url::parse(&created.connect_url).unwrap();
    let capability = url
        .fragment()
        .unwrap()
        .strip_prefix("t=")
        .unwrap()
        .to_string();
    let link = links::load(&f.state, &created.id, &f.auth.user_id.to_string())
        .await
        .unwrap();
    (link, capability)
}
async fn redeemed(f: &Fixture) -> AppConnectLink {
    let (link, capability) = started(f).await;
    let _ = redeem(
        State(f.state.clone()),
        human(f),
        Path(link.id.clone()),
        peer(),
        HeaderMap::new(),
        Json(RedeemRequest { capability }),
    )
    .await
    .unwrap();
    links::load(&f.state, &link.id, &link.user_id)
        .await
        .unwrap()
}
async fn empty(f: &Fixture) {
    let slug = "api-github-pat";
    catalog(f, slug, "bearer").await;
    publish(f, requirement(slug)).await;
}

#[tokio::test]
async fn app_connect_links_db_redemption_is_once_and_subject_bound() {
    let Some(f) = fixture("app_link_redeem").await else {
        return;
    };
    empty(&f).await;
    let (link, capability) = started(&f).await;
    let stranger = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&stranger, UserType::Person))
        .await
        .unwrap();
    let mut other = test_auth_user(&stranger);
    other.auth_method = AuthMethod::Session;
    assert!(matches!(
        redeem(
            State(f.state.clone()),
            other,
            Path(link.id.clone()),
            peer(),
            HeaderMap::new(),
            Json(RedeemRequest {
                capability: capability.clone()
            })
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(
        get(
            State(f.state.clone()),
            human(&f),
            Path(link.id.clone()),
            peer(),
            HeaderMap::new()
        )
        .await
        .is_err()
    );
    let redeemed = links::redeem(&f.state, &link.id, &link.user_id, &capability)
        .await
        .unwrap();
    assert!(redeemed.capability_hash.is_empty());
    assert!(redeemed.redeemed_at.is_some());
    assert!(
        links::redeem(&f.state, &link.id, &link.user_id, &capability)
            .await
            .is_err()
    );
    let _ = get(
        State(f.state.clone()),
        human(&f),
        Path(link.id),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn app_connect_links_db_app_b_cannot_read_app_a() {
    let Some(f) = fixture("app_link_client").await else {
        return;
    };
    empty(&f).await;
    let (link, _) = started(&f).await;
    let mut app_b = f.app.clone();
    app_b.id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<crate::models::oauth_client::OauthClient>("oauth_clients")
        .insert_one(&app_b)
        .await
        .unwrap();
    let mut other = f.auth.clone();
    other.oauth_client_id = Some(app_b.id);
    assert!(matches!(
        get(
            State(f.state.clone()),
            other,
            Path(link.id.clone()),
            peer(),
            HeaderMap::new()
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    let _ = get(
        State(f.state.clone()),
        f.auth.clone(),
        Path(link.id),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn app_connect_links_db_org_admin_cannot_act_on_member_session() {
    let Some(f) = fixture("app_link_org_subject").await else {
        return;
    };
    empty(&f).await;
    let member_id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&member_id, UserType::Person))
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &f.owner,
            &member_id,
            crate::models::org_membership::OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    let created = links::start_from_app(
        &f.state,
        &f.app.id,
        &member_id,
        "https://app.example/callback",
        "member-state",
    )
    .await
    .unwrap();
    assert!(matches!(
        links::load(&f.state, &created.link.id, &f.auth.user_id.to_string()).await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        cancel(
            State(f.state.clone()),
            human(&f),
            Path(created.link.id),
            peer(),
            HeaderMap::new(),
            None,
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
}

#[tokio::test]
async fn app_connect_links_db_rollout_disabled_hides_every_route() {
    let Some(f) = fixture("app_link_disabled_rollout").await else {
        return;
    };
    empty(&f).await;
    let link = redeemed(&f).await;
    let mut policy = f.state.app_connect_policy();
    policy.rollout = AppConnectRollout::Disabled;
    policy.revision += 1;
    f.state.set_app_connect_policy_if_fresh(policy);
    let who = human(&f);
    let path = || Path((link.id.clone(), "required".into()));
    assert!(matches!(
        create(
            State(f.state.clone()),
            f.auth.clone(),
            Json(CreateRequest {
                callback_url: "https://app.example/callback".into(),
                state: "x".into()
            })
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        get(
            State(f.state.clone()),
            who.clone(),
            Path(link.id.clone()),
            peer(),
            HeaderMap::new()
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        redeem(
            State(f.state.clone()),
            who.clone(),
            Path(link.id.clone()),
            peer(),
            HeaderMap::new(),
            Json(RedeemRequest {
                capability: "x".into()
            })
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        connect(
            State(f.state.clone()),
            who.clone(),
            path(),
            peer(),
            HeaderMap::new(),
            Json(ConnectRequest {
                service_slug: "api-github-pat".into()
            })
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        reauthorize(
            State(f.state.clone()),
            who.clone(),
            path(),
            peer(),
            HeaderMap::new(),
            Json(ConnectRequest {
                service_slug: "api-github-pat".into()
            })
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        select(
            State(f.state.clone()),
            who.clone(),
            path(),
            peer(),
            HeaderMap::new(),
            Json(SelectRequest {
                user_service_id: None
            })
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        validate(
            State(f.state.clone()),
            who.clone(),
            path(),
            peer(),
            HeaderMap::new()
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        ready(
            State(f.state.clone()),
            who.clone(),
            Path(link.id.clone()),
            peer(),
            HeaderMap::new()
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
    assert!(matches!(
        cancel(
            State(f.state.clone()),
            who,
            Path(link.id),
            peer(),
            HeaderMap::new(),
            None,
        )
        .await,
        Err(AppError::AppConnectLinkNotFound)
    ));
}

#[tokio::test]
async fn app_connect_links_db_hosted_actions_reject_non_session_callers() {
    let Some(f) = fixture("app_link_humans").await else {
        return;
    };
    empty(&f).await;
    let link = redeemed(&f).await;
    for method in [
        AuthMethod::AccessToken,
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::Relay,
        AuthMethod::ServiceAccount,
    ] {
        let mut auth = human(&f);
        auth.auth_method = method;
        assert!(matches!(
            cancel(
                State(f.state.clone()),
                auth,
                Path(link.id.clone()),
                peer(),
                HeaderMap::new(),
                None,
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
}

#[tokio::test]
async fn app_connect_links_db_no_key_is_included_and_repair_never_issues_tokens() {
    let Some(f) = fixture("app_link_included").await else {
        return;
    };
    catalog(&f, "included-service", "none").await;
    let mut r = requirement("included-service");
    r.allow_no_credential = true;
    publish(&f, r).await;
    let link = redeemed(&f).await;
    let view = get(
        State(f.state.clone()),
        human(&f),
        Path(link.id.clone()),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(view.items[0].readiness, "included");
    assert_eq!(view.items[0].state, ItemState::Met);
    let terminal = ready(
        State(f.state.clone()),
        human(&f),
        Path(link.id),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(terminal.status, AppConnectStatus::Completed);
    assert!(terminal.grant_update_required);
    let callback = url::Url::parse(terminal.callback_url.as_ref().unwrap()).unwrap();
    let query: std::collections::HashMap<_, _> = callback.query_pairs().collect();
    assert_eq!(query.get("state").unwrap(), "app-state");
    assert_eq!(query.get("status").unwrap(), "completed");
    assert!(!query.contains_key("code"));
    for collection in ["authorization_codes", "access_tokens", "refresh_tokens"] {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn app_connect_links_db_disabled_service_is_never_enabled() {
    let Some(f) = fixture("app_link_disabled_service").await else {
        return;
    };
    let catalog = catalog(&f, "api-github-pat", "bearer").await;
    publish(&f, requirement("api-github-pat")).await;
    let (s, _) = service(&f, &f.auth.user_id.to_string(), &catalog, "disabled").await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&s.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    let link = redeemed(&f).await;
    let view = get(
        State(f.state.clone()),
        human(&f),
        Path(link.id.clone()),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(view.items[0].readiness, "disabled");
    assert!(
        links::ready(&f.state, &link.id, &link.user_id)
            .await
            .is_err()
    );
    assert!(
        !f.state
            .db
            .collection::<Document>("user_services")
            .find_one(doc! {"_id":s.id})
            .await
            .unwrap()
            .unwrap()
            .get_bool("is_active")
            .unwrap()
    );
}

#[tokio::test]
async fn app_connect_links_db_child_return_is_internal_and_public_preview_denied() {
    let Some(f) = fixture("app_link_child").await else {
        return;
    };
    empty(&f).await;
    let link = redeemed(&f).await;
    let child = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github-pat",
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        child.link.parent_session_id.as_deref(),
        Some(link.id.as_str())
    );
    assert_eq!(
        child.link.callback_url,
        Some(links::return_url(&f.state, &link.id))
    );
    assert!(matches!(
        connect_link_service::preview(&f.state.db, &child.raw_token).await,
        Err(AppError::ConnectLinkNotFound)
    ));
    let mut app_auth = f.auth.clone();
    app_auth.auth_method = AuthMethod::AccessToken;
    assert!(guard_child(&f.state, &app_auth, &child.link).await.is_err());
    guard_child(&f.state, &human(&f), &child.link)
        .await
        .unwrap();
}

#[tokio::test]
async fn app_connect_links_db_child_completion_reconciles_after_restart() {
    let Some(f) = fixture("app_link_reconcile").await else {
        return;
    };
    empty(&f).await;
    let link = redeemed(&f).await;
    let child = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github-pat",
        false,
    )
    .await
    .unwrap();
    let catalog_id = child.link.service_id.clone();
    let (s, _) = service(&f, &link.user_id, &catalog_id, "reconciled").await;
    f.state
        .db
        .collection::<Document>("connect_links")
        .update_one(
            doc! {"_id":child.link.id},
            doc! {"$set":{"status":"completed","completed_user_service_id":&s.id}},
        )
        .await
        .unwrap();
    let (read, _) = links::refresh(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(read.items[0].user_service_id, Some(s.id));
    assert!(read.items[0].explicit_selection);
    assert_eq!(read.items[0].state, ItemState::Met);
}

#[tokio::test]
async fn app_connect_links_db_child_cancel_fences_inflight_item_and_parent_cancel_is_terminal() {
    let Some(f) = fixture("app_link_cancel_fence").await else {
        return;
    };
    empty(&f).await;
    let link = redeemed(&f).await;
    let child = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github-pat",
        false,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>(LINKS)
        .update_one(
            doc! {"_id":&link.id},
            doc! {"$set":{"items.0.state":"validating","items.0.attempt_id":"old-probe"}},
        )
        .await
        .unwrap();
    connect_link_service::cancel(&f.state.db, &link.user_id, &child.link.id)
        .await
        .unwrap();
    let settled = links::load(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(settled.items[0].attempt_id, None);
    let late = f
        .state
        .db
        .collection::<Document>(LINKS)
        .update_one(
            doc! {"_id":&link.id,"status":"in_progress","items.0.attempt_id":"old-probe"},
            doc! {"$set":{"items.0.state":"met"}},
        )
        .await
        .unwrap();
    assert_eq!(late.matched_count, 0);
    links::cancel(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert!(matches!(
        links::ready(&f.state, &link.id, &link.user_id).await,
        Err(AppError::AppConnectLinkCancelled)
    ));
}

#[tokio::test]
async fn app_connect_links_db_callback_requires_registered_destination_and_state() {
    let Some(f) = fixture("app_link_callback").await else {
        return;
    };
    empty(&f).await;
    assert!(
        links::start_from_app(
            &f.state,
            &f.app.id,
            &f.auth.user_id.to_string(),
            "https://evil.example/callback",
            "state"
        )
        .await
        .is_err()
    );
    assert!(
        links::start_from_app(
            &f.state,
            &f.app.id,
            &f.auth.user_id.to_string(),
            "https://app.example/callback",
            ""
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn app_connect_links_db_expiry_and_ttl_extensions_are_bounded() {
    let Some(f) = fixture("app_link_expiry").await else {
        return;
    };
    catalog(&f, "included-service", "none").await;
    let mut r = requirement("included-service");
    r.allow_no_credential = true;
    publish(&f, r).await;
    let link = redeemed(&f).await;
    let (once, _) = links::refresh(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    let (twice, _) = links::refresh(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(once.expires_at, twice.expires_at);
    assert!(twice.expires_at <= twice.created_at + chrono::Duration::hours(2));
    f.state.db.collection::<Document>(LINKS).update_one(doc! {"_id":&link.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    links::expire_sessions(&f.state.db).await.unwrap();
    assert_eq!(
        links::load(&f.state, &link.id, &link.user_id)
            .await
            .unwrap()
            .status,
        AppConnectStatus::Expired
    );
}

#[tokio::test]
async fn app_connect_links_db_scope_repair_reuses_connection_and_provider_scope_allowlist() {
    use crate::models::provider_config::ProviderConfig;
    let Some(f) = fixture("app_link_scope_repair").await else {
        return;
    };
    let catalog_id = catalog(&f, "api-github", "bearer").await;
    let provider_id = Uuid::new_v4().to_string();
    let mut provider:ProviderConfig=bson::from_document(doc! {
        "_id":&provider_id,"slug":"github","name":"GitHub","provider_type":"oauth2",
        "authorization_url":"https://github.com/login/oauth/authorize","token_url":"https://github.com/login/oauth/access_token",
        "default_scopes":["read:user"],"is_active":true,"created_by":"system","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).unwrap();
    provider.client_id_encrypted = Some(
        f.state
            .encryption_keys
            .encrypt(b"test-client")
            .await
            .unwrap(),
    );
    provider.client_secret_encrypted = Some(
        f.state
            .encryption_keys
            .encrypt(b"test-secret")
            .await
            .unwrap(),
    );
    f.state
        .db
        .collection::<ProviderConfig>("provider_configs")
        .insert_one(provider)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalog_id},
            doc! {"$set":{"provider_config_id":&provider_id}},
        )
        .await
        .unwrap();
    let mut r = requirement("api-github");
    r.accepted_credential_types = vec!["oauth2".into()];
    r.required_downstream_scopes = vec!["repo".into()];
    publish(&f, r).await;
    let (service, key) = service(
        &f,
        &f.auth.user_id.to_string(),
        &catalog_id,
        "personal-github",
    )
    .await;
    let connection = Uuid::new_v4().to_string();
    f.state.db.collection::<Document>("user_api_keys").update_one(doc! {"_id":&key.id},doc! {"$set":{"provider_config_id":&provider_id,"connection_id":&connection,"token_scopes":"read:user"}}).await.unwrap();
    let link = redeemed(&f).await;
    let view = get(
        State(f.state.clone()),
        human(&f),
        Path(link.id.clone()),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(view.items[0].readiness, "needs_reauth");
    let child = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github",
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        child.link.reauthorize_user_service_id.as_ref(),
        Some(&service.id)
    );
    assert_eq!(child.link.required_scopes, vec!["repo"]);
    let authorization = super::super::connect_links::complete_connect_link(
        State(f.state.clone()),
        human(&f),
        peer(),
        HeaderMap::new(),
        Json(super::super::connect_links::CompleteConnectLinkRequest {
            token: child.raw_token,
            credential: None,
            endpoint_url: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            device_state: None,
        }),
    )
    .await
    .unwrap()
    .0;
    let url = url::Url::parse(authorization.authorization_url.as_ref().unwrap()).unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert!(query["scope"].split_whitespace().any(|s| s == "repo"));
    let oauth_state = f
        .state
        .db
        .collection::<crate::models::oauth_state::OAuthState>("oauth_states")
        .find_one(doc! {"connect_link_id":&child.link.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(oauth_state.connection_id, Some(connection.clone()));
    assert_eq!(
        oauth_state.redirect_path,
        Some(format!("/connect/app/{}", link.id))
    );
    // A successful provider callback stores the expanded scopes on this same key.
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .update_one(
            doc! {"_id":&key.id},
            doc! {"$set":{"status":"active","token_scopes":"read:user repo"}},
        )
        .await
        .unwrap();
    connect_link_service::complete_oauth_callback(
        &f.state.db,
        &child.link.id,
        &link.user_id,
        &connection,
    )
    .await
    .unwrap();
    let (repaired, report) = links::refresh(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(
        report.requirements[0].state,
        crate::services::app_requirements_service::RequirementState::Met
    );
    assert_eq!(repaired.items[0].user_service_id, Some(service.id));
    assert_eq!(
        f.state
            .db
            .collection::<Document>("user_api_keys")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(matches!(
        crate::services::user_token_service::initiate_oauth_connect(
            &f.state.db,
            &f.state.encryption_keys,
            &f.state.config.base_url,
            &link.user_id,
            &provider_id,
            None,
            None,
            &["delete_repo".into()],
            None,
            Some(&connection),
            None,
            None
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
}

#[tokio::test]
async fn app_connect_links_db_real_late_probe_cannot_restore_cancelled_child() {
    use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};
    let Some((f, node_id, mut rx, service)) = probe_fixture("app_link_real_probe").await else {
        return;
    };
    let link = redeemed(&f).await;
    let child = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github",
        false,
    )
    .await
    .unwrap();
    // Model a completed connection waiting for a check, while leaving the child
    // Pending so cancellation remains possible during that provider request.
    f.state.db.collection::<Document>(LINKS).update_one(doc! {"_id":&link.id},doc! {"$set":{"items.0.state":"unknown","items.0.user_service_id":&service.id,"items.0.explicit_selection":true}}).await.unwrap();
    let state = f.state.clone();
    let id = link.id.clone();
    let subject = link.user_id.clone();
    let running =
        tokio::spawn(async move { links::validate_item(&state, &id, &subject, "required").await });
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let NodeOutboundMessage::Text(frame) = frame else {
        panic!("expected proxy request")
    };
    let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
    connect_link_service::cancel(&f.state.db, &link.user_id, &child.link.id)
        .await
        .unwrap();
    f.state.node_ws_manager.deliver_proxy_response(
        &node_id,
        NodeProxyResponse {
            request_id: frame["request_id"].as_str().unwrap().into(),
            status: 200,
            headers: vec![],
            body: br#"{"login":"fixture"}"#.to_vec(),
        },
    );
    running.await.unwrap().unwrap();
    let (final_link, _) = links::refresh(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(final_link.items[0].state, ItemState::Failed);
    assert_eq!(
        final_link.items[0].reason_code.as_deref(),
        Some("child_cancelled")
    );
    let record = f
        .state
        .db
        .collection::<Document>("service_validation_records")
        .find_one(doc! {"user_service_id":service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record
            .get_document("caller_context")
            .unwrap()
            .get_str("kind")
            .unwrap(),
        "app"
    );
}

#[tokio::test]
async fn app_connect_links_db_frozen_manifest_and_no_provider_io_on_read() {
    let Some(f) = fixture("app_link_frozen").await else {
        return;
    };
    let id = catalog(&f, "api-github", "bearer").await;
    let mut r = requirement("api-github");
    r.validator = crate::models::app_requirement_manifest::ValidatorSelection::Profile {
        id: "github_user_v1".into(),
    };
    publish(&f, r).await;
    service(&f, &f.auth.user_id.to_string(), &id, "unverified").await;
    let link = redeemed(&f).await;
    catalog(&f, "another-service", "bearer").await;
    publish(&f, requirement("another-service")).await;
    let view = get(
        State(f.state.clone()),
        human(&f),
        Path(link.id),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(view.requirements_version, 1);
    assert_eq!(view.items[0].catalog_slugs, vec!["api-github"]);
    assert_eq!(view.items[0].state, ItemState::Unknown);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("service_validation_records")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn app_connect_links_db_selection_requires_manifest_membership_and_resource_acl() {
    let Some(f) = fixture("app_link_selection_acl").await else {
        return;
    };
    let catalog_id = catalog(&f, "api-github-pat", "bearer").await;
    publish(&f, requirement("api-github-pat")).await;
    let stranger = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&stranger, UserType::Person))
        .await
        .unwrap();
    let (foreign, _) = service(&f, &stranger, &catalog_id, "foreign").await;
    let link = redeemed(&f).await;
    assert!(matches!(
        links::select_item(
            &f.state,
            &link.id,
            &link.user_id,
            "required",
            Some(&foreign.id)
        )
        .await,
        Err(AppError::RequirementNotSatisfiable)
    ));
    assert!(matches!(
        links::connect_item(
            &f.state,
            &link.id,
            &link.user_id,
            "required",
            "unlisted-slug",
            false
        )
        .await,
        Err(AppError::RequirementNotSatisfiable)
    ));
    let view = get(
        State(f.state.clone()),
        human(&f),
        Path(link.id),
        peer(),
        HeaderMap::new(),
    )
    .await
    .unwrap()
    .0;
    assert!(view.items[0].choices.is_empty());
    assert!(view.items[0].user_service_id.is_none());
}

#[tokio::test]
async fn app_connect_links_db_capability_debug_and_bson_dates_are_safe() {
    let Some(f) = fixture("app_link_model").await else {
        return;
    };
    empty(&f).await;
    crate::db::ensure_app_connect_link_indexes(&f.state.db)
        .await
        .unwrap();
    let (link, capability) = started(&f).await;
    let document = bson::to_document(&link).unwrap();
    assert!(document.get_datetime("created_at").is_ok());
    assert!(document.get_datetime("expires_at").is_ok());
    assert_eq!(document.get_str("_id").unwrap(), link.id);
    let debug = format!("{link:?}");
    assert!(!debug.contains(&capability));
    assert!(!debug.contains(&link.capability_hash));
    assert!(!debug.contains("app-state"));
    assert!(!debug.contains("app.example"));
    let mut terminal = link;
    terminal.status = AppConnectStatus::Failed;
    terminal.origin=AppConnectOrigin::App{callback_url:"https://app.example/callback?status=forged&state=wrong&app_connect_link_id=wrong&grant_update_required=false&keep=1".into(),state:"bound-state".into()};
    let url = url::Url::parse(&links::terminal_callback_url(&terminal).unwrap().unwrap()).unwrap();
    assert_eq!(url.query_pairs().filter(|(k, _)| k == "state").count(), 1);
    assert_eq!(
        url.query_pairs().find(|(k, _)| k == "state").unwrap().1,
        "bound-state"
    );
    assert_eq!(
        url.query_pairs().find(|(k, _)| k == "status").unwrap().1,
        "failed"
    );
}

#[tokio::test]
async fn app_connect_links_db_choices_snapshot_each_service_once() {
    use crate::handlers::app_requirements::publish_manifest;
    use crate::services::app_requirement_manifest_service::PublishManifest;
    let Some(f) = fixture("app_link_choices").await else {
        return;
    };
    let catalog_id = catalog(&f, "api-github-pat", "bearer").await;
    let requirements = (0..3)
        .map(|i| {
            let mut r = requirement("api-github-pat");
            r.id = format!("requirement-{i}");
            r
        })
        .collect();
    let _ = publish_manifest(
        State(f.state.clone()),
        f.auth.clone(),
        Path(f.app.id.clone()),
        Json(PublishManifest {
            enforcement: crate::models::app_requirement_manifest::Enforcement::Advise,
            requirements,
        }),
    )
    .await
    .unwrap();
    for i in 0..4 {
        service(
            &f,
            &f.auth.user_id.to_string(),
            &catalog_id,
            &format!("choice-{i}"),
        )
        .await;
    }
    let link = redeemed(&f).await;
    let response = response(&f.state, &human(&f), &link.id).await.unwrap();
    assert_eq!(response.authority_snapshots, 4);
    assert_eq!(response.items.len(), 3);
    for item in response.items {
        assert_eq!(item.choices.len(), 4);
        assert!(
            item.choices
                .iter()
                .all(|c| c.catalog_slug == "api-github-pat")
        );
    }
}

#[tokio::test]
async fn app_connect_links_db_child_failure_reasons_survive_reads_until_action() {
    let Some(f) = fixture("app_link_failure_reasons").await else {
        return;
    };
    empty(&f).await;
    let link = redeemed(&f).await;
    let child = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github-pat",
        false,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>("connect_links")
        .update_one(
            doc! { "_id": &child.link.id },
            doc! { "$set": { "last_error": "provider_denied" } },
        )
        .await
        .unwrap();
    for _ in 0..2 {
        let read = response(&f.state, &human(&f), &link.id).await.unwrap();
        assert_eq!(read.items[0].state, ItemState::Failed);
        assert_eq!(
            read.items[0].reason_code.as_deref(),
            Some("provider_authorization_failed")
        );
    }
    let _ = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github-pat",
        false,
    )
    .await
    .unwrap();
    let read = response(&f.state, &human(&f), &link.id).await.unwrap();
    assert_eq!(read.items[0].state, ItemState::Connecting);
    assert_eq!(read.items[0].reason_code, None);
    f.state
        .db
        .collection::<Document>(LINKS)
        .update_one(
            doc! { "_id": &link.id },
            doc! { "$set": { "items.0.connect_link_id": null,
            "items.0.state": "validating", "items.0.attempt_started_at": null } },
        )
        .await
        .unwrap();
    for _ in 0..2 {
        let read = response(&f.state, &human(&f), &link.id).await.unwrap();
        assert_eq!(read.items[0].state, ItemState::Unknown);
        assert_eq!(
            read.items[0].reason_code.as_deref(),
            Some("attempt_interrupted")
        );
    }
}

#[tokio::test]
async fn app_connect_links_db_parent_cancel_and_expiry_cancel_pending_children() {
    let Some(f) = fixture("app_link_children_cleanup").await else {
        return;
    };
    empty(&f).await;
    for expire in [false, true] {
        let link = redeemed(&f).await;
        let child = links::connect_item(
            &f.state,
            &link.id,
            &link.user_id,
            "required",
            "api-github-pat",
            false,
        )
        .await
        .unwrap();
        if expire {
            f.state
                .db
                .collection::<Document>(LINKS)
                .update_one(
                    doc! { "_id": &link.id },
                    doc! { "$set": { "expires_at": bson::DateTime::from_millis(0) } },
                )
                .await
                .unwrap();
            links::expire_sessions(&f.state.db).await.unwrap();
        } else {
            links::cancel(&f.state, &link.id, &link.user_id)
                .await
                .unwrap();
        }
        let child = f
            .state
            .db
            .collection::<crate::models::connect_link::ConnectLink>("connect_links")
            .find_one(doc! { "_id": child.link.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            child.status,
            crate::models::connect_link::ConnectLinkStatus::Cancelled
        );
        assert!(child.completed_at.is_some());
    }
}

async fn probe_fixture(
    name: &str,
) -> Option<(
    Fixture,
    String,
    tokio::sync::mpsc::Receiver<crate::services::node_ws_manager::NodeOutboundMessage>,
    crate::models::user_service::UserService,
)> {
    use crate::models::node::Node;
    use crate::services::node_ws_manager::NodeCapabilitiesMsg;
    let mut f = fixture(name).await?;
    crate::services::coordination_service::ensure_indexes(&f.state.db)
        .await
        .unwrap();
    crate::db::ensure_service_validation_indexes(&f.state.db)
        .await
        .unwrap();
    f.state.config.node_hmac_signing_enabled = false;
    let catalog_id = catalog(&f, "api-github", "bearer").await;
    let mut r = requirement("api-github");
    r.validator = crate::models::app_requirement_manifest::ValidatorSelection::Profile {
        id: "github_user_v1".into(),
    };
    publish(&f, r).await;
    let (service, key) = service(
        &f,
        &f.auth.user_id.to_string(),
        &catalog_id,
        "validation-github",
    )
    .await;
    let node_id = Uuid::new_v4().to_string();
    let node:Node=bson::from_document(doc! {"_id":&node_id,"user_id":f.auth.user_id.to_string(),"name":"Node","status":"online","auth_token_hash":"test","signing_secret_hash":"test","is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
    f.state
        .db
        .collection::<Node>("nodes")
        .insert_one(node)
        .await
        .unwrap();
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(b"fixture-token")
        .await
        .unwrap();
    f.state.db.collection::<Document>("user_api_keys").update_one(doc! {"_id":&key.id},doc! {"$set":{"access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&service.id}, doc! {"$set":{"node_id":&node_id}})
        .await
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    crate::test_utils::register_test_node_connection(&f.state, &node_id, tx).await;
    f.state.node_ws_manager.record_capabilities(
        &node_id,
        &NodeCapabilitiesMsg {
            no_redirect_proxy: true,
            ..Default::default()
        },
    );
    Some((f, node_id, rx, service))
}

#[tokio::test]
async fn app_connect_links_db_probe_cooldown_preserves_prior_item() {
    use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};
    let Some((f, node_id, mut rx, _)) = probe_fixture("app_link_probe_cooldown").await else {
        return;
    };
    let link = redeemed(&f).await;
    let state = f.state.clone();
    let id = link.id.clone();
    let subject = link.user_id.clone();
    let running =
        tokio::spawn(async move { links::validate_item(&state, &id, &subject, "required").await });
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let NodeOutboundMessage::Text(frame) = frame else {
        panic!("expected request");
    };
    let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
    f.state.node_ws_manager.deliver_proxy_response(
        &node_id,
        NodeProxyResponse {
            request_id: frame["request_id"].as_str().unwrap().into(),
            status: 200,
            headers: vec![],
            body: br#"{"login":"fixture"}"#.to_vec(),
        },
    );
    running.await.unwrap().unwrap();
    let (before, _) = links::refresh(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(before.items[0].state, ItemState::Met);
    assert!(matches!(
        links::validate_item(&f.state, &link.id, &link.user_id, "required").await,
        Err(AppError::ServiceValidationRateLimited)
    ));
    let after = links::load(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(after.items, before.items);
    assert!(
        rx.try_recv().is_err(),
        "cooldown must not dispatch another probe"
    );
}

#[tokio::test]
async fn app_connect_links_db_start_over_cancels_only_replaced_child() {
    use crate::models::connect_link::{ConnectLink, ConnectLinkStatus};
    let Some(f) = fixture("app_link_start_over").await else {
        return;
    };
    empty(&f).await;
    let link = redeemed(&f).await;
    let first = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github-pat",
        false,
    )
    .await
    .unwrap();
    let second = links::connect_item(
        &f.state,
        &link.id,
        &link.user_id,
        "required",
        "api-github-pat",
        false,
    )
    .await
    .unwrap();
    let children = f.state.db.collection::<ConnectLink>("connect_links");
    assert_eq!(
        children
            .find_one(doc! { "_id": &first.link.id })
            .await
            .unwrap()
            .unwrap()
            .status,
        ConnectLinkStatus::Cancelled
    );
    assert_eq!(
        children
            .find_one(doc! { "_id": &second.link.id })
            .await
            .unwrap()
            .unwrap()
            .status,
        ConnectLinkStatus::Pending
    );
    let parent = links::load(&f.state, &link.id, &link.user_id)
        .await
        .unwrap();
    assert_eq!(
        parent.items[0].connect_link_id.as_deref(),
        Some(second.link.id.as_str())
    );
    assert!(
        links::ensure_child_subject(&f.state.db, &first.link, &link.user_id)
            .await
            .is_err()
    );
}
