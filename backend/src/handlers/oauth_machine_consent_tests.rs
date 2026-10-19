// Included in oauth::tests to exercise the real initial/incremental OAuth handlers.
struct MachineConsentFixture {
    state: AppState,
    user: String,
    client: String,
    node: String,
    params: AuthorizeQuery,
}
impl MachineConsentFixture {
    async fn new() -> Self {
        let db = crate::test_utils::connect_transaction_test_database("machine_oauth").await;
        crate::services::machine_external_grant_service::create_indexes(&db)
            .await
            .unwrap();
        db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"user_id":1,"client_id":1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let user = Uuid::new_v4().to_string();
        let client = Uuid::new_v4().to_string();
        let node = Uuid::new_v4().to_string();
        insert_person_user(&db, &user).await;
        insert_public_client(&db, &client, "openid proxy machine offline_access").await;
        let profile = nyxid_machine::MachineProfile {
            version: nyxid_machine::PROTOCOL_VERSION,
            runtime_id: Uuid::new_v4().to_string(),
            shell: true,
            files: true,
            roots: vec!["/workspace".into()],
            ..Default::default()
        };
        db.collection::<mongodb::bson::Document>("nodes").insert_one(doc! {
            "_id": &node, "user_id": &user, "name": "Machine test", "status": "online",
            "auth_token_hash":"test-only", "is_active":true,
            "machine": mongodb::bson::to_bson(&profile).unwrap(),
            "created_at":mongodb::bson::DateTime::now(), "updated_at":mongodb::bson::DateTime::now(),
        }).await.unwrap();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(b"machine-test-verifier"));
        let params = serde_json::from_value(serde_json::json!({
            "response_type":"code","client_id":client,"redirect_uri":"https://app.example/callback",
            "scope":"openid proxy machine offline_access", "code_challenge":challenge,
            "code_challenge_method":"S256","prompt":"consent"
        }))
        .unwrap();
        Self {
            state,
            user,
            client,
            node,
            params,
        }
    }
    fn selection(&self) -> crate::services::machine_external_grant_service::GrantSelection {
        serde_json::from_value(serde_json::json!({"assignments":{self.node.clone():{
            "capabilities":{"shell":true,"files":true},"file_roots":["/workspace"],"service_ids":[]
        }}}))
        .unwrap()
    }
    async fn prepare(&self) -> String {
        self.prepare_params(&self.params).await
    }
    async fn prepare_params(&self, params: &AuthorizeQuery) -> String {
        let signed = sign_consent_request(
            &self.state,
            &self.user,
            params,
            params.scope.as_deref().unwrap(),
        )
        .unwrap();
        prepare_machine_consent(
            State(self.state.clone()),
            crate::test_utils::test_auth_user(&self.user),
            Json(PrepareMachineConsentRequest {
                consent_request: signed,
                selection: self.selection(),
            }),
        )
        .await
        .unwrap()
        .0
        .token
    }
    async fn decide(&self, token: &str) -> AppResult<Response> {
        let form = serde_json::from_value(serde_json::json!({
            "response_type":"code","client_id":"forged-client","redirect_uri":"https://evil.invalid",
            "scope":"proxy", "decision":"allow","consent_request":token,
            "allow_all_services":false,"allowed_service_ids":[]
        })).unwrap();
        authorize_decision(
            State(self.state.clone()),
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&self.user))),
            TelemetryContext::default(),
            Form(form),
        )
        .await
    }
    async fn code(&self, token: &str) -> String {
        let response = self.decide(token).await.unwrap();
        let url = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("app.example"));
        url.query_pairs()
            .find(|(k, _)| k == "code")
            .unwrap()
            .1
            .into_owned()
    }
    async fn exchange(&self, code: &str) -> oauth_service::ExchangedTokens {
        oauth_service::exchange_authorization_code(
            &self.state.db,
            &self.state.config,
            &self.state.jwt_keys,
            false,
            code,
            &self.client,
            "https://app.example/callback",
            Some("machine-test-verifier"),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn machine_oauth_signed_selection_is_one_time_and_atomic_under_race() {
    let f = MachineConsentFixture::new().await;
    let signed = f.prepare().await;
    let (a, b) = tokio::join!(f.decide(&signed), f.decide(&signed));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let live = consent_service::check_consent(&f.state.db, &f.user, &f.client, "machine")
        .await
        .unwrap()
        .unwrap();
    let grant = f
        .state
        .db
        .collection::<mongodb::bson::Document>("machine_external_grants")
        .find_one(doc! {"user_id":&f.user,"status":"active"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        grant
            .get_document("binding")
            .unwrap()
            .get_str("consent_revision")
            .unwrap(),
        live.revision.as_deref().unwrap()
    );
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_external_grants")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(f.decide(&signed).await.is_err());
}

#[tokio::test]
async fn machine_oauth_code_access_refresh_keep_exact_version_and_never_upgrade() {
    let f = MachineConsentFixture::new().await;
    let signed = f.prepare().await;
    let code = f.code(&signed).await;
    let stored = f
        .state
        .db
        .collection::<AuthorizationCode>(AUTH_CODES)
        .find_one(doc! {"code_hash":crate::crypto::token::hash_token(&code)})
        .await
        .unwrap()
        .unwrap();
    let reference = stored.protected_consent.unwrap();
    let issued = f.exchange(&code).await;
    let access =
        jwt::verify_token(&f.state.jwt_keys, &f.state.config, &issued.access_token).unwrap();
    assert_eq!(access.protected_consent, Some(reference.clone()));
    let row = f
        .state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! {"jti":&issued.refresh_token_jti})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.protected_consent, Some(reference.clone()));
    let refreshed = crate::services::token_service::refresh_tokens(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &issued.refresh_token,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        jwt::verify_token(&f.state.jwt_keys, &f.state.config, &refreshed.access_token)
            .unwrap()
            .protected_consent,
        Some(reference.clone())
    );
    let old_grant = f
        .state
        .db
        .collection::<mongodb::bson::Document>("machine_external_grants")
        .find_one(doc! {"status":"active"})
        .await
        .unwrap()
        .unwrap();
    let again = f.prepare().await;
    f.code(&again).await;
    assert!(
        crate::services::token_service::refresh_tokens(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &refreshed.refresh_token,
            None,
            None
        )
        .await
        .is_err()
    );
    assert!(
        consent_service::validate_protected_reference(
            &f.state.db,
            &f.user,
            &f.client,
            "machine",
            Some(&reference)
        )
        .await
        .is_err()
    );
    let active = f
        .state
        .db
        .collection::<mongodb::bson::Document>("machine_external_grants")
        .find_one(doc! {"status":"active"})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        old_grant.get_str("_id").unwrap(),
        active.get_str("_id").unwrap()
    );
    // Every issuance entrypoint rejects missing and stale protected versions.
    for reference in [None, Some(&reference)] {
        assert!(
            oauth_service::issue_oauth_refresh_token(
                &f.state.db,
                &f.state.config,
                &f.state.jwt_keys,
                &f.client,
                &f.user,
                "machine proxy",
                &[],
                &[],
                false,
                reference,
            )
            .await
            .is_err()
        );
    }
    // A legacy refresh row claiming machine cannot silently acquire current consent.
    let legacy = oauth_service::issue_oauth_refresh_token(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &f.client,
        &f.user,
        "proxy",
        &[],
        &[],
        false,
        None,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! {"jti":&legacy.refresh_token_jti},
            doc! {"$set":{"scope":"machine proxy"}},
        )
        .await
        .unwrap();
    assert!(
        crate::services::token_service::refresh_tokens(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &legacy.refresh_token,
            None,
            None
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn machine_oauth_rejects_unsigned_or_wrong_owner_selection_and_rolls_back_stale_approval() {
    let f = MachineConsentFixture::new().await;
    let signed = f.prepare().await;
    let other = Uuid::new_v4().to_string();
    assert!(verify_consent_request(&f.state, &signed, &other).is_err());
    let mut altered = signed.clone().into_bytes();
    altered[10] = if altered[10] == b'a' { b'b' } else { b'a' };
    assert!(
        verify_consent_request(&f.state, std::str::from_utf8(&altered).unwrap(), &f.user).is_err()
    );
    let mut forged = f.selection();
    forged.assignments.values_mut().next().unwrap().file_roots = vec!["/etc".into()];
    assert!(
        crate::services::machine_external_grant_service::validate_selection(
            &f.state.db,
            &f.user,
            &forged
        )
        .await
        .is_err()
    );
    let replacement = consent_service::grant_consent_with_services(
        &f.state.db,
        &f.user,
        &f.client,
        "proxy",
        Some(vec![]),
    )
    .await
    .unwrap();
    assert!(f.decide(&signed).await.is_err());
    let current = consent_service::check_consent(&f.state.db, &f.user, &f.client, "")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.revision, replacement.revision);
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_external_grants")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let request = f
        .state
        .db
        .collection::<mongodb::bson::Document>("machine_oauth_consent_requests")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        request.get("consumed_at"),
        Some(mongodb::bson::Bson::Null)
    ));
}

#[tokio::test]
async fn machine_oauth_incremental_approval_binds_new_version_and_service_edit_revokes() {
    let f = MachineConsentFixture::new().await;
    let first = f.prepare().await;
    f.code(&first).await;
    let old = consent_service::check_consent(&f.state.db, &f.user, &f.client, "machine")
        .await
        .unwrap()
        .unwrap();
    let mut params = f.params.clone();
    params.service_access_mode = Some(ServiceAccessMode::Incremental);
    params.incremental_consent = Some(
        incremental_consent_service::prepare(
            &f.state.db,
            &f.user,
            &f.client,
            "test",
            Some("machine proxy"),
            &[],
            &[],
            None,
        )
        .await
        .unwrap(),
    );
    let request = f.prepare_params(&params).await;
    let code = f.code(&request).await;
    let issued = f.exchange(&code).await;
    assert_ne!(
        issued.protected_consent.unwrap().consent_revision,
        old.revision.unwrap()
    );
    consent_service::grant_consent_with_services(
        &f.state.db,
        &f.user,
        &f.client,
        "machine proxy",
        Some(vec![]),
    )
    .await
    .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_external_grants")
            .count_documents(doc! {"status":"active"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn machine_oauth_client_security_mutation_fences_but_label_edit_does_not() {
    let f = MachineConsentFixture::new().await;
    let signed = f.prepare().await;
    f.code(&signed).await;
    oauth_client_service::admin_update_client(
        &f.state.db,
        &f.client,
        oauth_client_service::AdminUpdateClient {
            client_name: Some("Renamed only"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_external_grants")
            .count_documents(doc! {"status":"active"})
            .await
            .unwrap(),
        1
    );
    oauth_client_service::admin_update_client(
        &f.state.db,
        &f.client,
        oauth_client_service::AdminUpdateClient {
            is_active: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    oauth_client_service::admin_update_client(
        &f.state.db,
        &f.client,
        oauth_client_service::AdminUpdateClient {
            is_active: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let client = f
        .state
        .db
        .collection::<mongodb::bson::Document>(OAUTH_CLIENTS)
        .find_one(doc! {"_id":&f.client})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        client
            .get_i64("machine_external_security_revision")
            .unwrap(),
        2
    );
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_external_grants")
            .count_documents(doc! {"status":"active"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_external_grants")
            .count_documents(doc! {"status":"revoked"})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn machine_oauth_consent_revoke_reconsent_never_adopts_old_tokens() {
    let f = MachineConsentFixture::new().await;
    let signed = f.prepare().await;
    let code = f.code(&signed).await;
    let original = f.exchange(&code).await;
    let old_reference = original.protected_consent.unwrap();
    let result = consent_service::revoke_consent(&f.state.db, &f.user, &f.client)
        .await
        .unwrap();
    assert_eq!(result.revoked_refresh_tokens, 1);
    let request = f.prepare().await;
    let code = f.code(&request).await;
    let current = f.exchange(&code).await;
    let new_reference = current.protected_consent.unwrap();
    assert_ne!(old_reference.consent_id, new_reference.consent_id);
    assert!(
        consent_service::validate_protected_reference(
            &f.state.db,
            &f.user,
            &f.client,
            "machine proxy",
            Some(&old_reference)
        )
        .await
        .is_err()
    );
    assert!(
        crate::services::token_service::refresh_tokens(
            &f.state.db,
            &f.state.config,
            &f.state.jwt_keys,
            &original.refresh_token,
            None,
            None
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn machine_oauth_pending_initial_request_cannot_survive_consent_revoke_aba() {
    let f = MachineConsentFixture::new().await;
    // Both selections observed no Consent. Deleting the Consent created by A
    // must not make the older pending B authoritative again.
    let selected_a = f.prepare().await;
    let selected_b = f.prepare().await;
    assert_ne!(selected_a, selected_b);
    f.code(&selected_a).await;
    consent_service::revoke_consent(&f.state.db, &f.user, &f.client)
        .await
        .unwrap();
    assert!(f.decide(&selected_b).await.is_err());
    assert!(
        consent_service::check_consent(&f.state.db, &f.user, &f.client, "")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_external_grants")
            .count_documents(doc! {"status":"active"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>("machine_oauth_consent_requests")
            .count_documents(doc! {"consumed_at":mongodb::bson::Bson::Null})
            .await
            .unwrap(),
        0
    );
    // A new owner selection is still usable after revocation.
    f.code(&f.prepare().await).await;
}

#[tokio::test]
async fn machine_oauth_pending_selection_cannot_cross_source_disable_reenable() {
    for change_client in [true, false] {
        let f = MachineConsentFixture::new().await;
        let selected = f.prepare().await;
        for active in [false, true] {
            if change_client {
                oauth_client_service::admin_update_client(
                    &f.state.db,
                    &f.client,
                    oauth_client_service::AdminUpdateClient {
                        is_active: Some(active),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            } else {
                // Exercise the explicit integration contract: source mutation
                // and its monotonic Machine epoch belong to one transaction.
                let mut session = f.state.db.client().start_session().await.unwrap();
                session.start_transaction().await.unwrap();
                f.state
                    .db
                    .collection::<mongodb::bson::Document>("users")
                    .update_one(doc! {"_id":&f.user}, doc! {"$set":{"is_active":active}})
                    .session(&mut session)
                    .await
                    .unwrap();
                crate::services::machine_external_grant_service::user_security_changed_in_session(
                    &f.state.db,
                    &f.user,
                    "test_source_security_change",
                    &mut session,
                )
                .await
                .unwrap();
                session.commit_transaction().await.unwrap();
            }
        }
        assert!(f.decide(&selected).await.is_err());
        assert!(
            consent_service::check_consent(&f.state.db, &f.user, &f.client, "")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            f.state
                .db
                .collection::<mongodb::bson::Document>("machine_external_grants")
                .count_documents(doc! {"status":"active"})
                .await
                .unwrap(),
            0
        );
        f.code(&f.prepare().await).await;
    }
}

#[tokio::test]
async fn machine_oauth_requires_human_saved_selection_and_rejects_expired_request() {
    let f = MachineConsentFixture::new().await;
    let signed = sign_consent_request(
        &f.state,
        &f.user,
        &f.params,
        "openid proxy machine offline_access",
    )
    .unwrap();
    assert!(f.decide(&signed).await.is_err());
    let mut caller = crate::test_utils::test_auth_user(&f.user);
    caller.oauth_client_id = Some(f.client.clone());
    assert!(
        prepare_machine_consent(
            State(f.state.clone()),
            caller,
            Json(PrepareMachineConsentRequest {
                consent_request: signed.clone(),
                selection: f.selection()
            })
        )
        .await
        .is_err()
    );
    let mut ordinary = f.params.clone();
    ordinary.scope = Some("proxy".into());
    let ordinary = sign_consent_request(&f.state, &f.user, &ordinary, "proxy").unwrap();
    assert!(
        prepare_machine_consent(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.user),
            Json(PrepareMachineConsentRequest {
                consent_request: ordinary,
                selection: f.selection()
            })
        )
        .await
        .is_err()
    );
    let selected = f.prepare().await;
    f.state.db.collection::<mongodb::bson::Document>("machine_oauth_consent_requests")
        .update_many(doc! {},doc! {"$set":{"expires_at":mongodb::bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(f.decide(&selected).await.is_err());
    assert!(
        consent_service::check_consent(&f.state.db, &f.user, &f.client, "")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn machine_oauth_silent_authorization_does_not_launder_other_credentials() {
    let f = MachineConsentFixture::new().await;
    f.code(&f.prepare().await).await;
    let mut params = f.params.clone();
    params.prompt = Some("none".into());
    let before = f
        .state
        .db
        .collection::<AuthorizationCode>(AUTH_CODES)
        .count_documents(doc! {})
        .await
        .unwrap();
    for is_browser in [true, false] {
        let mut key = crate::test_utils::test_auth_user(&f.user);
        key.auth_method = AuthMethod::ApiKey;
        key.api_key_id = Some(Uuid::new_v4().to_string());
        assert!(
            authorize_inner(
                &f.state,
                OptionalAuthUser(Some(key)),
                &params,
                is_browser,
                None
            )
            .await
            .is_err()
        );
        let mut oauth = crate::test_utils::test_auth_user(&f.user);
        oauth.oauth_client_id = Some(Uuid::new_v4().to_string());
        assert!(
            authorize_inner(
                &f.state,
                OptionalAuthUser(Some(oauth)),
                &params,
                is_browser,
                None
            )
            .await
            .is_err()
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    // Existing human consent can still use the standard silent browser flow.
    let response = authorize_inner(
        &f.state,
        OptionalAuthUser(Some(crate::test_utils::test_auth_user(&f.user))),
        &params,
        true,
        None,
    )
    .await
    .unwrap();
    let url = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    assert!(url.query_pairs().any(|(name, _)| name == "code"));
}

#[tokio::test]
async fn machine_oauth_ordinary_broker_token_retains_client_and_cannot_approve() {
    use axum::extract::FromRequestParts;
    let f = MachineConsentFixture::new().await;
    let original = f.prepare().await;
    f.code(&original).await;
    let issued = oauth_service::issue_oauth_refresh_token(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &f.client,
        &f.user,
        "openid proxy",
        &[],
        &[],
        false,
        None,
    )
    .await
    .unwrap();
    let (binding, _) = oauth_broker_service::create_binding(
        &f.state.db,
        &f.state.encryption_keys,
        &f.client,
        &f.user,
        &issued.refresh_token,
        &issued.refresh_token_jti,
        &["openid".into(), "proxy".into()],
        None,
        None,
        false,
    )
    .await
    .unwrap();
    let exchanged = oauth_broker_service::exchange_via_binding(
        &f.state.db,
        f.state.encryption_keys.clone(),
        &f.state.http_client,
        &f.state.jwt_keys,
        &f.state.config,
        false,
        &f.client,
        &binding,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let claims =
        jwt::verify_token(&f.state.jwt_keys, &f.state.config, &exchanged.access_token).unwrap();
    assert_eq!(claims.client_id.as_deref(), Some(f.client.as_str()));
    assert!(claims.protected_consent.is_none());
    let request = axum::http::Request::builder()
        .uri("/oauth/authorize")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", exchanged.access_token),
        )
        .body(())
        .unwrap();
    let (mut parts, _) = request.into_parts();
    let caller = AuthUser::from_request_parts(&mut parts, &f.state)
        .await
        .unwrap();
    assert_eq!(caller.oauth_client_id.as_deref(), Some(f.client.as_str()));
    assert!(
        prepare_machine_consent(
            State(f.state.clone()),
            caller,
            Json(PrepareMachineConsentRequest {
                consent_request: original,
                selection: f.selection(),
            }),
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn machine_oauth_pending_source_invalidation_forces_reconsent_review() {
    let f = MachineConsentFixture::new().await;
    let signed = f.prepare().await;
    f.code(&signed).await;
    let consent = consent_service::check_consent(&f.state.db, &f.user, &f.client, "machine")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !crate::services::machine_oauth_consent_service::needs_review(
            &f.state.db,
            &f.user,
            &f.client,
            "machine proxy",
            Some(&consent)
        )
        .await
        .unwrap()
    );
    // Simulate a durable source fence with a later grant outside the first
    // bounded invalidation batch. The row remains active but is no authority.
    f.state
        .db
        .collection::<mongodb::bson::Document>(OAUTH_CLIENTS)
        .update_one(
            doc! {"_id":&f.client},
            doc! {"$inc":{"machine_external_security_revision":1_i64}},
        )
        .await
        .unwrap();
    assert!(
        crate::services::machine_oauth_consent_service::needs_review(
            &f.state.db,
            &f.user,
            &f.client,
            "machine proxy",
            Some(&consent)
        )
        .await
        .unwrap()
    );
    assert!(
        !crate::services::machine_oauth_consent_service::needs_review(
            &f.state.db,
            &f.user,
            &f.client,
            "proxy",
            Some(&consent)
        )
        .await
        .unwrap()
    );
}
