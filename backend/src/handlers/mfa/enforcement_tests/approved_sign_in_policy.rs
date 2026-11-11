use super::*;
use crate::{
    models::{
        auth_device_code::{
            AuthDeviceCode, COLLECTION_NAME as LEGACY_CODES, V2_COLLECTION_NAME as DEVICE_CODES,
        },
        session::MfaExemption,
    },
    services::{
        approved_sign_in_policy as policy, auth_agent_key_login_service as agent,
        auth_device_service as device, login_code_service as code,
        session_authority::DerivedMfaAuthority,
    },
};

async fn parent(state: &AppState, id: &str, verified: bool) -> token_service::IssuedTokens {
    let tokens = token_service::prepare_session_tokens_with_assurance(
        &state.db,
        &state.config,
        &state.jwt_keys,
        id,
        None,
        None,
        verified,
    )
    .await
    .unwrap();
    let mut tx = state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    token_service::insert_prepared_session(&state.db, &tokens, &mut tx)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    if !verified {
        state
            .db
            .collection::<bson::Document>(SESSIONS)
            .update_one(
                doc! {"_id": &tokens.tokens.session_id},
                doc! {"$set": {"mfa_exemption": "mobile_rollout"}},
            )
            .await
            .unwrap();
    }
    tokens.tokens
}

fn authority(tokens: &token_service::IssuedTokens, verified: bool) -> DerivedMfaAuthority {
    let mut authority = DerivedMfaAuthority::new(true, verified);
    authority.session_id = Some(uuid::Uuid::parse_str(&tokens.session_id).unwrap());
    authority
}

async fn change(
    state: &AppState,
    id: &str,
    verified: &token_service::IssuedTokens,
    enabled: bool,
) -> String {
    let saved = policy::save(
        &state.db,
        id,
        enabled,
        authority(verified, true),
        &crate::services::audit_service::AuditActor {
            user_id: id.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
    )
    .await
    .unwrap();
    saved.revision.unwrap_or_default()
}

async fn enable_fixture_policy(
    state: &AppState,
    id: &str,
    verified: &token_service::IssuedTokens,
    default_on: bool,
) -> String {
    if default_on {
        state
            .db
            .collection::<bson::Document>(USERS)
            .update_one(
                doc! {"_id": id},
                doc! {"$unset": {"profile_config.security": ""}},
            )
            .await
            .unwrap();
        policy::DEFAULT_REVISION.into()
    } else {
        change(state, id, verified, true).await
    }
}

#[test]
fn qr_policy_missing_model_shapes_default_on_and_explicit_false_is_preserved() {
    assert!(
        crate::models::user::UserProfileConfig::default()
            .security
            .skip_mfa_for_approved_sign_ins
    );
    for shape in ["profile", "security", "field", "nonce", "revision"] {
        let mut row = bson::to_document(&test_user(
            &uuid::Uuid::new_v4().to_string(),
            UserType::Person,
        ))
        .unwrap();
        match shape {
            "profile" => {
                row.remove("profile_config");
            }
            "security" => {
                row.get_document_mut("profile_config")
                    .unwrap()
                    .remove("security");
            }
            "field" => {
                row.get_document_mut("profile_config")
                    .unwrap()
                    .insert("security", doc! {});
            }
            "nonce" => {
                row.get_document_mut("profile_config")
                    .unwrap()
                    .insert("security", doc! {"use_nonce": "used"});
            }
            _ => {
                row.get_document_mut("profile_config").unwrap().insert(
                    "security",
                    doc! {"revision": uuid::Uuid::new_v4().to_string()},
                );
            }
        }
        let user: User = bson::from_document(row).unwrap();
        assert!(
            user.profile_config.security.skip_mfa_for_approved_sign_ins,
            "{shape}"
        );
    }
    let policy: crate::models::user::SecurityPreferences =
        bson::from_document(doc! {"skip_mfa_for_approved_sign_ins": false}).unwrap();
    assert!(!policy.skip_mfa_for_approved_sign_ins);
}

async fn mfa_fixture(
    name: &str,
) -> (
    AppState,
    String,
    token_service::IssuedTokens,
    token_service::IssuedTokens,
) {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let (state, id) = fixture(name).await;
    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! {"_id": &id}, doc! {"$set": {"mfa_enabled": true}})
        .await
        .unwrap();
    let verified = parent(&state, &id, true).await;
    let phone = parent(&state, &id, false).await;
    (state, id, verified, phone)
}

fn input(id: &str, request: &device::InitiateOutput) -> device::ApproveInput {
    device::ApproveInput {
        user_id: id.into(),
        user_code: request.user_code.clone(),
        approver_ip: None,
        approver_user_agent: None,
    }
}

fn selection() -> agent::Selection {
    serde_json::from_value(json!({"kind":"new", "name":"Policy fixture", "scopes":"proxy"}))
        .unwrap()
}

async fn assert_revoked_credential(state: &AppState, id: &str) {
    use crate::models::api_key_credential::{
        ApiKeyCredential, COLLECTION_NAME, CredentialRevokedReason,
    };
    let credential = state
        .db
        .collection::<ApiKeyCredential>(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap();
    assert!(!credential.is_active);
    assert!(credential.revoked_at.is_some());
    assert_eq!(
        credential.revoked_reason,
        Some(CredentialRevokedReason::UndeliveredExpired)
    );
}

#[tokio::test]
async fn qr_policy_setting_is_mounted_persisted_and_requires_verified_enable_only() {
    let (state, id, verified, phone) = mfa_fixture("qr_setting_http").await;
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(USERS)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"profile_config.security.future_setting": "preserve"}},
        )
        .await
        .unwrap();
    let (_, router) = crate::routes::build_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let app = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router
                .with_state(app)
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::new();
    let endpoint = format!("{base}/users/me/security");
    let refused = client
        .put(&endpoint)
        .bearer_auth(&phone.access_token)
        .json(&json!({"skip_mfa_for_approved_sign_ins": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(refused.json::<Value>().await.unwrap()["error_code"], 2003);
    let unknown = client
        .put(&endpoint)
        .bearer_auth(&verified.access_token)
        .json(
            &json!({"skip_mfa_for_approved_sign_ins": true, "revision": policy::DEFAULT_REVISION}),
        )
        .send()
        .await
        .unwrap();
    assert!(unknown.status().is_client_error());
    let saved = client
        .put(&endpoint)
        .bearer_auth(&verified.access_token)
        .json(&json!({"skip_mfa_for_approved_sign_ins": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        saved.status(),
        StatusCode::OK,
        "{}",
        saved.text().await.unwrap()
    );
    let response: Value = client
        .get(format!("{base}/users/me"))
        .bearer_auth(&phone.access_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        response["profile_config"]["security"],
        json!({"skip_mfa_for_approved_sign_ins": true})
    );
    let row = state
        .db
        .collection::<bson::Document>(USERS)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    let config = row
        .get_document("profile_config")
        .unwrap()
        .get_document("security")
        .unwrap();
    let revision = config.get_str("revision").unwrap().to_owned();
    assert_eq!(config.get_str("future_setting").unwrap(), "preserve");
    assert!(config.get_datetime("changed_at").is_ok());
    let disabled = client
        .put(&endpoint)
        .bearer_auth(&phone.access_token)
        .json(&json!({"skip_mfa_for_approved_sign_ins": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(disabled.status(), StatusCode::OK);
    let row = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.profile_config.security.skip_mfa_for_approved_sign_ins);
    assert_ne!(
        row.profile_config.security.revision.as_deref(),
        Some(revision.as_str())
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"user_id": &id, "event_type": "security_setting_changed"})
            .await
            .unwrap(),
        2
    );
    for method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::Delegated,
        crate::mw::auth::AuthMethod::ServiceAccount,
        crate::mw::auth::AuthMethod::Relay,
        crate::mw::auth::AuthMethod::AccessToken,
    ] {
        let mut auth = test_auth_user(&id);
        auth.auth_method = method.clone();
        if method == crate::mw::auth::AuthMethod::AccessToken {
            auth.oauth_client_id = Some("third-party".into());
        }
        assert!(
            crate::handlers::users::save_security_preferences(
                State(state.clone()),
                auth,
                Json(crate::handlers::users::SaveSecurityPreferencesRequest {
                    skip_mfa_for_approved_sign_ins: true
                })
            )
            .await
            .is_err()
        );
    }
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qr_policy_legacy_and_v2_deliver_honest_browser_and_cli_authority() {
    exercise_account_policy(false).await;
}

#[tokio::test]
async fn qr_policy_default_legacy_and_v2_need_no_mfa() {
    exercise_account_policy(true).await;
}

async fn exercise_account_policy(default_on: bool) {
    let (state, id, verified, phone) = mfa_fixture("qr_policy_delivery").await;
    let (base, server) = serve(state.clone()).await;
    let client = reqwest::Client::new();
    let phone_authority = authority(&phone, false);
    for legacy_parent in [false, true] {
        if legacy_parent {
            state
                .db
                .collection::<bson::Document>(SESSIONS)
                .update_one(
                    doc! {"_id": &phone.session_id},
                    doc! {"$unset": {"mfa_assured": "", "mfa_exemption": ""}},
                )
                .await
                .unwrap();
        }
        for v2 in [false, true] {
            for browser in [false, true] {
                if !default_on {
                    change(&state, &id, &verified, false).await;
                }
                if default_on {
                    enable_fixture_policy(&state, &id, &verified, true).await;
                }
                let request = if v2 {
                    device::initiate_v2(
                        &state.db,
                        state.auth_device_hmac_key.as_slice(),
                        Default::default(),
                        true,
                    )
                    .await
                    .unwrap()
                } else {
                    device::initiate(
                        &state.db,
                        state.auth_device_hmac_key.as_slice(),
                        Default::default(),
                    )
                    .await
                    .unwrap()
                };
                let approve = || {
                    device::approve(
                        &state.db,
                        &state.config,
                        &state.jwt_keys,
                        &state.encryption_keys,
                        state.auth_device_hmac_key.as_slice(),
                        input(&id, &request),
                        phone_authority,
                    )
                };
                let revision = if default_on {
                    let saved = policy::save(
                        &state.db,
                        &id,
                        true,
                        phone_authority,
                        &crate::services::audit_service::AuditActor {
                            user_id: id.clone(),
                            ip_address: None,
                            user_agent: None,
                            api_key_id: None,
                            api_key_name: None,
                        },
                    )
                    .await
                    .unwrap();
                    assert!(
                        saved.revision.is_none(),
                        "default true save must be a no-op without MFA"
                    );
                    policy::DEFAULT_REVISION.to_owned()
                } else {
                    assert!(matches!(approve().await, Err(AppError::MfaStepUpRequired)));
                    change(&state, &id, &verified, true).await
                };
                let mut sidless = phone_authority;
                sidless.session_id = None;
                assert!(matches!(
                    device::approve(
                        &state.db,
                        &state.config,
                        &state.jwt_keys,
                        &state.encryption_keys,
                        state.auth_device_hmac_key.as_slice(),
                        input(&id, &request),
                        sidless
                    )
                    .await,
                    Err(AppError::MfaStepUpRequired)
                ));
                approve().await.unwrap();
                let delivery = if browser {
                    device::poll_for_browser(
                        &state.db,
                        &state.encryption_keys,
                        state.auth_device_hmac_key.as_slice(),
                        &request.device_code,
                        &hash_token("policy-browser"),
                        "127.0.0.1",
                        None,
                    )
                    .await
                    .unwrap()
                } else {
                    device::poll_and_prepare(
                        &state.db,
                        &state.encryption_keys,
                        state.auth_device_hmac_key.as_slice(),
                        &request.device_code,
                    )
                    .await
                    .unwrap()
                };
                let device::PollClaim::Account(delivery) = delivery else {
                    panic!("account delivery expected")
                };
                let session = state
                    .db
                    .collection::<Session>(SESSIONS)
                    .find_one(doc! {"_id": &delivery.session_id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(session.mfa_assured, Some(false));
                assert_eq!(
                    session.mfa_exemption,
                    Some(MfaExemption::ApprovedSignInPolicy)
                );
                let response = if browser {
                    client
                        .get(format!("{base}/identity"))
                        .header(header::COOKIE, "nyx_session=policy-browser")
                } else {
                    client
                        .get(format!("{base}/identity"))
                        .bearer_auth(&delivery.access_token)
                }
                .send()
                .await
                .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                if !browser {
                    let refreshed = token_service::refresh_tokens(
                        &state.db,
                        &state.config,
                        &state.jwt_keys,
                        &delivery.refresh_token,
                        None,
                        None,
                    )
                    .await
                    .unwrap();
                    assert_eq!(refreshed.session_id, session.id);
                }
                // Exempt authority authenticates but cannot use ordinary credential derivation.
                assert!(matches!(
                    code::mint(
                        &state.db,
                        state.auth_device_hmac_key.as_slice(),
                        &id,
                        None,
                        None,
                        phone_authority
                    )
                    .await,
                    Err(AppError::MfaStepUpRequired)
                ));
                assert!(matches!(
                    token_service::create_derived_session_tokens(
                        &state.db,
                        &state.config,
                        &state.jwt_keys,
                        &id,
                        None,
                        None,
                        phone_authority
                    )
                    .await,
                    Err(AppError::MfaStepUpRequired)
                ));
                let rows = state.db.collection::<AuthDeviceCode>(if v2 {
                    DEVICE_CODES
                } else {
                    LEGACY_CODES
                });
                let row = rows.find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), request.device_code.as_bytes())}).await.unwrap().unwrap();
                if default_on {
                    let user = state
                        .db
                        .collection::<User>(USERS)
                        .find_one(doc! {"_id": &id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(user.profile_config.security.skip_mfa_for_approved_sign_ins);
                    assert!(user.profile_config.security.revision.is_none());
                }
                assert!(!row.mfa_assured);
                assert_eq!(row.mfa_policy_revision.as_deref(), Some(revision.as_str()));
            }
        }
    }
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qr_policy_revocation_blocks_pending_delivery_and_off_on_never_revives_it() {
    let (state, id, verified, phone) = mfa_fixture("qr_policy_revoke").await;
    for browser in [false, true] {
        enable_fixture_policy(&state, &id, &verified, true).await;
        let request = device::initiate_v2(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            Default::default(),
            true,
        )
        .await
        .unwrap();
        device::approve(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            input(&id, &request),
            authority(&phone, false),
        )
        .await
        .unwrap();
        let row = state.db.collection::<AuthDeviceCode>(DEVICE_CODES).find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), request.device_code.as_bytes())}).await.unwrap().unwrap();
        change(&state, &id, &verified, false).await;
        change(&state, &id, &verified, true).await;
        let denied = if browser {
            device::poll_for_browser(
                &state.db,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                &request.device_code,
                &hash_token("never-issued"),
                "127.0.0.1",
                None,
            )
            .await
        } else {
            device::poll_and_prepare(
                &state.db,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                &request.device_code,
            )
            .await
        };
        assert!(matches!(denied, Err(AppError::AuthDeviceCodeDenied)));
        assert!(
            state
                .db
                .collection::<Session>(SESSIONS)
                .find_one(doc! {"_id": &row.approved_session_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                .count_documents(doc! {"session_id": &row.approved_session_id, "revoked": false})
                .await
                .unwrap(),
            0
        );
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qr_policy_agent_key_protocols_and_bound_browser_handoffs_obey_live_revision() {
    exercise_agent_policy(false).await;
}

#[tokio::test]
async fn qr_policy_default_agent_key_protocols_and_handoffs_need_no_mfa() {
    exercise_agent_policy(true).await;
}

async fn exercise_agent_policy(default_on: bool) {
    let (state, id, verified, phone) = mfa_fixture("qr_policy_agent").await;
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    for rule15 in [false, true] {
        for revoke in [false, true] {
            for handoff in [false, true] {
                if rule15 && handoff {
                    continue;
                }
                let revision = enable_fixture_policy(&state, &id, &verified, default_on).await;
                if rule15 {
                    let request = agent::request(
                        &state.db,
                        state.auth_device_hmac_key.as_slice(),
                        Default::default(),
                        None,
                    )
                    .await
                    .unwrap();
                    agent::approve(
                        &state.db,
                        &state.encryption_keys,
                        state.auth_device_hmac_key.as_slice(),
                        &id,
                        &request.user_code,
                        selection(),
                        None,
                        None,
                        None,
                        authority(&phone, false),
                    )
                    .await
                    .unwrap();
                    let row = state.db.collection::<bson::Document>(crate::models::agent_key_login_request::COLLECTION_NAME)
                        .find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), format!("agent-key-login:{}", request.device_code).as_bytes())}).await.unwrap().unwrap();
                    assert_eq!(row.get_str("mfa_policy_revision").unwrap(), revision);
                    if revoke {
                        change(&state, &id, &verified, false).await;
                    }
                    let delivery = agent::poll(
                        &state.db,
                        &state.encryption_keys,
                        state.auth_device_hmac_key.as_slice(),
                        &request.device_code,
                        None,
                        None,
                    )
                    .await;
                    if revoke {
                        assert!(matches!(delivery, Err(AppError::AgentKeyLoginDenied)));
                        let row = state.db.collection::<bson::Document>(crate::models::agent_key_login_request::COLLECTION_NAME)
                            .find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), format!("agent-key-login:{}", request.device_code).as_bytes())})
                            .await.unwrap().unwrap();
                        assert_eq!(row.get_str("status").unwrap(), "denied");
                        assert!(row.get_datetime("purge_at").is_ok());
                        assert!(!row.contains_key("delivery_credential_encrypted"));
                        assert_revoked_credential(&state, row.get_str("credential_id").unwrap())
                            .await;
                    } else {
                        assert!(delivery.is_ok());
                    }
                } else {
                    let request = device::initiate_v2(
                        &state.db,
                        state.auth_device_hmac_key.as_slice(),
                        Default::default(),
                        true,
                    )
                    .await
                    .unwrap();
                    device::approve_with_agent_key(
                        &state.db,
                        &state.encryption_keys,
                        state.auth_device_hmac_key.as_slice(),
                        input(&id, &request),
                        selection(),
                        None,
                        authority(&phone, false),
                    )
                    .await
                    .unwrap();
                    let approved = state.db.collection::<AuthDeviceCode>(DEVICE_CODES)
                        .find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), request.device_code.as_bytes())}).await.unwrap().unwrap();
                    assert_eq!(
                        approved.mfa_policy_revision.as_deref(),
                        Some(revision.as_str())
                    );
                    let browser_code = if handoff {
                        Some(
                            code::browser_handoff(
                                &state.db,
                                state.auth_device_hmac_key.as_slice(),
                                &request.device_code,
                            )
                            .await
                            .unwrap(),
                        )
                    } else {
                        None
                    };
                    if revoke {
                        change(&state, &id, &verified, false).await;
                    }
                    if let Some(minted) = browser_code {
                        let bound = state
                            .db
                            .collection::<crate::models::login_code::LoginCode>(
                                crate::models::login_code::COLLECTION_NAME,
                            )
                            .find_one(doc! {"_id": &minted.id})
                            .await
                            .unwrap()
                            .unwrap();
                        assert_eq!(
                            bound.mfa_policy_revision.as_deref(),
                            Some(revision.as_str())
                        );
                        let result = code::redeem(
                            &state.db,
                            &state.config,
                            &state.jwt_keys,
                            &state.encryption_keys,
                            state.auth_device_hmac_key.as_slice(),
                            &minted.code,
                            device::InitiateInput {
                                client_ip: Some("127.0.0.1".into()),
                                ..Default::default()
                            },
                            None,
                        )
                        .await;
                        if revoke {
                            assert!(matches!(result, Err(AppError::LoginCodeInvalid)));
                            let row = state
                                .db
                                .collection::<crate::models::login_code::LoginCode>(
                                    crate::models::login_code::COLLECTION_NAME,
                                )
                                .find_one(doc! {"_id": &minted.id})
                                .await
                                .unwrap()
                                .unwrap();
                            assert_eq!(
                                row.status,
                                crate::models::login_code::LoginCodeStatus::Cancelled
                            );
                        } else {
                            assert!(matches!(result, Ok(code::Delivery::AgentKey(_))));
                        }
                    } else {
                        let result = device::poll_agent_key(
                            &state.db,
                            &state.encryption_keys,
                            state.auth_device_hmac_key.as_slice(),
                            &request.device_code,
                        )
                        .await;
                        if revoke {
                            assert!(matches!(result, Err(AppError::AuthDeviceCodeDenied)));
                        } else {
                            assert!(result.is_ok());
                        }
                    }
                    if revoke {
                        let row = state.db.collection::<AuthDeviceCode>(DEVICE_CODES)
                            .find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), request.device_code.as_bytes())})
                            .await.unwrap().unwrap();
                        assert_eq!(
                            row.status,
                            crate::models::auth_device_code::AuthDeviceCodeStatus::Denied
                        );
                        assert!(row.purge_at.is_some());
                        let grant = row.agent_key_grant.unwrap();
                        assert!(grant.delivery_credential_encrypted.is_none());
                        assert_revoked_credential(&state, &grant.credential_id).await;
                    }
                }
            }
        }
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qr_policy_replacing_a_handoff_preserves_the_grant_and_new_code() {
    let (state, id, verified, phone) = mfa_fixture("qr_policy_handoff_replace").await;
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    change(&state, &id, &verified, true).await;
    let request = device::initiate_v2(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        Default::default(),
        true,
    )
    .await
    .unwrap();
    device::approve_with_agent_key(
        &state.db,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        input(&id, &request),
        selection(),
        None,
        authority(&phone, false),
    )
    .await
    .unwrap();
    let old = code::browser_handoff(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &request.device_code,
    )
    .await
    .unwrap();
    let new = code::browser_handoff(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &request.device_code,
    )
    .await
    .unwrap();
    let redeem = async |code: &str| {
        code::redeem(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            code,
            device::InitiateInput {
                client_ip: Some("127.0.0.1".into()),
                ..Default::default()
            },
            None,
        )
        .await
    };
    assert!(matches!(
        redeem(&old.code).await,
        Err(AppError::LoginCodeInvalid)
    ));
    let row = state.db.collection::<AuthDeviceCode>(DEVICE_CODES)
        .find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), request.device_code.as_bytes())}).await.unwrap().unwrap();
    assert_eq!(
        row.status,
        crate::models::auth_device_code::AuthDeviceCodeStatus::Approved
    );
    let credential = state
        .db
        .collection::<crate::models::api_key_credential::ApiKeyCredential>(
            crate::models::api_key_credential::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &row.agent_key_grant.unwrap().credential_id})
        .await
        .unwrap()
        .unwrap();
    assert!(credential.is_active);
    assert!(credential.revoked_at.is_none());
    assert!(matches!(
        redeem(&new.code).await,
        Ok(code::Delivery::AgentKey(_))
    ));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qr_policy_marker_cannot_exempt_an_unbound_login_code() {
    let (state, id, verified, _) = mfa_fixture("qr_policy_unbound_code").await;
    let revision = enable_fixture_policy(&state, &id, &verified, true).await;
    let minted = code::mint(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &id,
        None,
        None,
        authority(&verified, true),
    )
    .await
    .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::login_code::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &minted.id},
            doc! {"$set": {
                "mfa_assured": false, "mfa_policy_revision": revision,
            }},
        )
        .await
        .unwrap();
    let sessions = state
        .db
        .collection::<Session>(SESSIONS)
        .count_documents(doc! {})
        .await
        .unwrap();
    assert!(matches!(
        code::redeem(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            &minted.code,
            device::InitiateInput {
                client_ip: Some("127.0.0.1".into()),
                ..Default::default()
            },
            None,
        )
        .await,
        Err(AppError::LoginCodeInvalid)
    ));
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        sessions
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qr_policy_app_identity_preserves_request_only_and_persistent_provenance() {
    exercise_app_policy(false).await;
}

#[tokio::test]
async fn qr_policy_default_app_identity_needs_no_mfa() {
    exercise_app_policy(true).await;
}

async fn exercise_app_policy(default_on: bool) {
    use crate::{
        handlers::login_approval as handler,
        models::login_approval::{COLLECTION_NAME as APPROVALS, LoginApproval, LoginFlow},
        services::login_approval_service as proof,
    };
    let (state, id, verified, phone) = mfa_fixture("qr_policy_app_identity").await;
    for keep in [false, true] {
        for revoke_at in ["never", "before_identity", "before_decision"] {
            let revision = enable_fixture_policy(&state, &id, &verified, default_on).await;
            let request = device::initiate_v2(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                Default::default(),
                true,
            )
            .await
            .unwrap();
            let (row, secret) = proof::begin(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                LoginFlow::Device,
                &request.user_code,
                keep,
            )
            .await
            .unwrap();
            let app = proof::begin_app(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                &row,
                Default::default(),
            )
            .await
            .unwrap();
            device::approve(
                &state.db,
                &state.config,
                &state.jwt_keys,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                input(&id, &app),
                authority(&phone, false),
            )
            .await
            .unwrap();
            if revoke_at == "before_identity" {
                change(&state, &id, &verified, false).await;
            }
            let mut headers = HeaderMap::new();
            headers.insert(
                header::ORIGIN,
                url::Url::parse(&state.config.frontend_url)
                    .unwrap()
                    .origin()
                    .ascii_serialization()
                    .parse()
                    .unwrap(),
            );
            headers.insert(
                header::COOKIE,
                format!(
                    "nyx_approval_{}={}",
                    row.id.replace('-', ""),
                    secret.as_str()
                )
                .parse()
                .unwrap(),
            );
            let result = handler::app_poll(
                State(state.clone()),
                axum::extract::Path(row.id.clone()),
                axum::extract::ConnectInfo("127.0.0.1:1234".parse().unwrap()),
                headers.clone(),
                Json(crate::handlers::auth_device::AuthDevicePollBody {
                    device_code: app.device_code.clone(),
                }),
            )
            .await;
            if revoke_at == "before_identity" {
                assert!(matches!(result, Err(AppError::AuthDeviceCodeDenied)));
                continue;
            }
            let (response_headers, response) = result.unwrap();
            let value = serde_json::to_value(response.0).unwrap();
            assert_eq!(value["verified"], true);
            assert_eq!(value["mfa_required"], false);
            assert_eq!(response_headers.contains_key(header::SET_COOKIE), keep);
            let proof_row = state
                .db
                .collection::<LoginApproval>(APPROVALS)
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .unwrap();
            assert!(proof_row.verified);
            assert!(!proof_row.mfa_verified);
            assert_eq!(
                proof_row.mfa_policy_revision.as_deref(),
                Some(revision.as_str())
            );
            if keep {
                let cookie = response_headers
                    .get_all(header::SET_COOKIE)
                    .iter()
                    .find_map(|v| {
                        v.to_str()
                            .unwrap()
                            .strip_prefix("nyx_session=")
                            .map(|v| v.split(';').next().unwrap())
                    })
                    .unwrap();
                let browser = state
                    .db
                    .collection::<Session>(SESSIONS)
                    .find_one(doc! {"token_hash": hash_token(cookie)})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(browser.mfa_assured, Some(false));
                assert_eq!(
                    browser.mfa_exemption,
                    Some(MfaExemption::ApprovedSignInPolicy)
                );
            }
            if revoke_at == "before_decision" {
                change(&state, &id, &verified, false).await;
                change(&state, &id, &verified, true).await;
            }
            let result = handler::approve(
                State(state.clone()),
                axum::extract::Path(row.id.clone()),
                axum::extract::ConnectInfo("127.0.0.1:1234".parse().unwrap()),
                headers,
                Json(serde_json::from_value(json!({})).unwrap()),
            )
            .await;
            if revoke_at == "before_decision" {
                assert!(result.is_err());
            } else {
                let _ = result.unwrap();
                let delivered = device::poll_and_prepare(
                    &state.db,
                    &state.encryption_keys,
                    state.auth_device_hmac_key.as_slice(),
                    &request.device_code,
                )
                .await
                .unwrap();
                assert!(matches!(delivered, device::PollClaim::Account(_)));
            }
        }
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qr_policy_toggle_without_mfa_and_mfa_disable_preserve_choice_and_revoke_proofs() {
    for keep_enabled in [false, true] {
        let (state, id) = fixture("qr_policy_mfa_lifecycle").await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let tokens = parent(&state, &id, false).await;
        enable_fixture_policy(&state, &id, &tokens, true).await;
        let audit = crate::services::audit_service::AuditActor {
            user_id: id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        let off = policy::save(&state.db, &id, false, authority(&tokens, false), &audit)
            .await
            .unwrap();
        let on = policy::save(&state.db, &id, true, authority(&tokens, false), &audit)
            .await
            .unwrap();
        assert!(!off.skip_mfa_for_approved_sign_ins);
        assert!(on.skip_mfa_for_approved_sign_ins);
        assert_ne!(off.revision, on.revision);
        assert_ne!(on.revision.as_deref(), Some(policy::DEFAULT_REVISION));
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"user_id": &id, "event_type": "security_setting_changed"})
                .await
                .unwrap(),
            2
        );
        state
            .db
            .collection::<Session>(SESSIONS)
            .delete_many(doc! {"user_id": &id})
            .await
            .unwrap();
        let recovery = enroll(&state, &id).await;
        let verified = parent(&state, &id, true).await;
        let phone = parent(&state, &id, false).await;
        let revision = if keep_enabled {
            enable_fixture_policy(&state, &id, &verified, true).await
        } else {
            change(&state, &id, &verified, false).await
        };
        let pending = if keep_enabled {
            let request = device::initiate_v2(
                &state.db,
                state.auth_device_hmac_key.as_slice(),
                Default::default(),
                true,
            )
            .await
            .unwrap();
            device::approve(
                &state.db,
                &state.config,
                &state.jwt_keys,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                input(&id, &request),
                authority(&phone, false),
            )
            .await
            .unwrap();
            Some(request)
        } else {
            None
        };
        mfa_service::disable_totp_with_code(
            &state.db,
            &state.encryption_keys,
            &id,
            &recovery[0],
            None,
            Some(&audit),
        )
        .await
        .unwrap();
        let user = state
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &id})
            .await
            .unwrap()
            .unwrap();
        assert!(!user.mfa_enabled);
        assert_eq!(
            user.profile_config.security.skip_mfa_for_approved_sign_ins,
            keep_enabled
        );
        assert_ne!(
            user.profile_config.security.revision.as_deref(),
            Some(revision.as_str())
        );
        if let Some(request) = pending {
            assert!(matches!(
                device::poll_and_prepare(
                    &state.db,
                    &state.encryption_keys,
                    state.auth_device_hmac_key.as_slice(),
                    &request.device_code
                )
                .await,
                Err(AppError::AuthDeviceCodeDenied)
            ));
            let denied = state.db.collection::<AuthDeviceCode>(DEVICE_CODES)
                .find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), request.device_code.as_bytes())})
                .await.unwrap().unwrap();
            assert!(denied.purge_at.is_some());
            assert_eq!(
                denied.status,
                crate::models::auth_device_code::AuthDeviceCodeStatus::Denied
            );
            assert!(denied.delivery_access_token_encrypted.is_none());
            assert!(denied.delivery_refresh_token_encrypted.is_none());
            assert!(
                state
                    .db
                    .collection::<Session>(SESSIONS)
                    .find_one(doc! {"_id": &denied.approved_session_id})
                    .await
                    .unwrap()
                    .unwrap()
                    .revoked
            );
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                    .count_documents(
                        doc! {"session_id": &denied.approved_session_id, "revoked": false}
                    )
                    .await
                    .unwrap(),
                0
            );
        }
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn qr_policy_concurrent_disable_retries_a_stale_delivery_snapshot_and_denies() {
    exercise_policy_collision(false).await;
}

#[tokio::test]
async fn qr_policy_default_concurrent_disable_denies_stale_snapshot() {
    exercise_policy_collision(true).await;
}

async fn exercise_policy_collision(default_on: bool) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (state, id, verified, phone) = mfa_fixture("qr_policy_collision").await;
    let revision = enable_fixture_policy(&state, &id, &verified, default_on).await;
    let request = device::initiate_v2(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        Default::default(),
        true,
    )
    .await
    .unwrap();
    device::approve(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        input(&id, &request),
        authority(&phone, false),
    )
    .await
    .unwrap();
    let rows = state.db.collection::<AuthDeviceCode>(DEVICE_CODES);
    let row = rows.find_one(doc! {"device_code_hmac": device::hmac_hex(state.auth_device_hmac_key.as_slice(), request.device_code.as_bytes())}).await.unwrap().unwrap();
    let snapshot_read = Arc::new(tokio::sync::Barrier::new(2));
    let disabled = Arc::new(tokio::sync::Barrier::new(2));
    let attempts = Arc::new(AtomicUsize::new(0));
    let context = (
        state.db.clone(),
        id.clone(),
        revision,
        row.id.clone(),
        snapshot_read.clone(),
        disabled.clone(),
        attempts.clone(),
    );
    let delivery = tokio::spawn(async move {
        let db = context.0.clone();
        crate::services::transaction_service::run_db_only(&db, &context, |tx, c| {
            Box::pin(async move {
                let (db, id, revision, request_id, snapshot_read, disabled, attempts) = c;
                db.collection::<User>(USERS)
                    .find_one(doc! {"_id": id})
                    .session(&mut *tx)
                    .await?
                    .unwrap();
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    snapshot_read.wait().await;
                    disabled.wait().await;
                }
                policy::fence(db, id, Some(revision), tx).await?;
                db.collection::<AuthDeviceCode>(DEVICE_CODES)
                    .update_one(
                        doc! {"_id": request_id, "status": "approved"},
                        doc! {"$set": {"status": "delivered"}},
                    )
                    .session(&mut *tx)
                    .await?;
                Ok(())
            })
        })
        .await
    });
    snapshot_read.wait().await;
    change(&state, &id, &verified, false).await;
    disabled.wait().await;
    assert!(matches!(
        delivery.await.unwrap(),
        Err(AppError::AuthDeviceCodeDenied)
    ));
    assert!(
        attempts.load(Ordering::SeqCst) >= 2,
        "driver must retry the transient write conflict"
    );
    assert_eq!(
        rows.find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap()
            .status,
        crate::models::auth_device_code::AuthDeviceCodeStatus::Approved
    );
    assert!(matches!(
        device::poll_and_prepare(
            &state.db,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            &request.device_code
        )
        .await,
        Err(AppError::AuthDeviceCodeDenied)
    ));
    let row = rows.find_one(doc! {"_id": &row.id}).await.unwrap().unwrap();
    assert!(row.purge_at.is_some());
    assert_eq!(
        row.status,
        crate::models::auth_device_code::AuthDeviceCodeStatus::Denied
    );
    state.db.drop().await.unwrap();
}
