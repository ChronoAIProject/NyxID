// Included in oauth::tests to share the real handler fixtures.
struct IncrementalFixture {
    state: AppState,
    user_id: String,
    client_id: String,
    services: Vec<UserService>,
    params: AuthorizeQuery,
    binding_id: Option<String>,
}

impl IncrementalFixture {
    async fn new(binding: bool, all: bool) -> Self {
        let db = connect_test_database("incremental").await.expect("MongoDB");
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let scope = format!("openid offline_access proxy {BROKER_BINDING_SCOPE}");
        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, &client_id, &scope).await;
        let mut services = Vec::new();
        for slug in ["old-a", "old-b", "new-c", "new-d", "optional-e"] {
            services.push(insert_user_service(&db, &user_id, slug).await);
        }
        let previous = vec![services[0].id.clone(), services[1].id.clone()];
        consent_service::grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            &scope,
            (!all).then_some(previous.clone()),
        )
        .await
        .unwrap();
        let binding_id =
            if binding {
                let raw = crate::models::oauth_broker_binding::generate_binding_id();
                insert_binding_for_client(
                    &state,
                    &client_id,
                    &raw,
                    &user_id,
                    scope.split_whitespace().map(String::from).collect(),
                )
                .await;
                let stored = load_binding(&db, &raw).await;
                db.collection::<RefreshToken>(REFRESH_TOKENS).update_one(
                doc! { "jti": &stored.refresh_token_jti },
                doc! { "$set": { "allow_all_services": all, "allowed_service_ids": previous } },
            ).await.unwrap();
                Some(raw)
            } else {
                None
            };
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(b"incremental-test-verifier"));
        let params: AuthorizeQuery = serde_json::from_value(serde_json::json!({
            "response_type": "code", "client_id": client_id,
            "redirect_uri": "https://app.example/callback", "scope": scope,
            "code_challenge": challenge, "code_challenge_method": "S256",
            "service_access_mode": "incremental", "prompt": "consent",
            "requested_service_ids": [services[2].id, services[3].id, services[2].id],
            "resource": [oauth_resource_service::user_service_resource_uri(&state.config, "new-c")],
            "binding_grant_id": binding_id.as_ref().map(|id| hash_binding_id(id)),
        }))
        .unwrap();
        Self {
            state,
            user_id,
            client_id,
            services,
            params,
            binding_id,
        }
    }

    async fn request(&self) -> AppResult<String> {
        let response = authorize_inner(
            &self.state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&self.user_id))),
            &self.params,
            true,
            None,
        )
        .await?;
        let location = response.headers()[header::LOCATION].to_str().unwrap();
        let url = url::Url::parse(location).unwrap();
        assert!(location.len() < 256, "consent URL stays bounded");
        let handle = url.query_pairs()
            .find(|(key, _)| key == "consent_request_id")
            .unwrap().1.into_owned();
        oauth_consent_request_service::get(&self.state.db, &self.user_id, &handle).await
    }

    async fn decide(
        &self,
        token: &str,
        selected: &[String],
        all: bool,
        decision: &str,
    ) -> AppResult<Response> {
        // Deliberately bogus unsigned fields: authority must come from the JWT.
        let form = serde_json::from_value(serde_json::json!({
            "response_type": "code", "client_id": "tampered-client",
            "redirect_uri": "https://evil.example", "decision": decision,
            "consent_request": token, "allowed_service_ids": selected,
            "allow_all_services": all, "service_access_mode": "incremental",
        }))
        .unwrap();
        authorize_incremental_decision(
            State(self.state.clone()),
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&self.user_id))),
            TelemetryContext::default(),
            Form(form),
        )
        .await
    }

    async fn code(&self, token: &str) -> String {
        let response = self.decide(token, &[], false, "allow").await.unwrap();
        let url = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("app.example"));
        url.query_pairs()
            .find(|(key, _)| key == "code")
            .unwrap()
            .1
            .into_owned()
    }

    async fn exchange(&self, code: &str) -> AppResult<Json<TokenResponse>> {
        let request = serde_json::from_value(serde_json::json!({
            "grant_type": "authorization_code", "client_id": self.client_id,
            "redirect_uri": "https://app.example/callback", "code": code,
            "code_verifier": "incremental-test-verifier",
        }))
        .unwrap();
        token_inner(
            &self.state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            request,
        )
        .await
    }
}

fn incremental_callback_error(response: &Response) -> Option<String> {
    url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
async fn incremental_ordinary_and_binding_flows_preserve_full_refresh_grant() {
    for with_binding in [false, true] {
        let fixture = IncrementalFixture::new(with_binding, false).await;
        if with_binding {
            // Legacy bindings deserialize an absent rotation version as zero.
            fixture
                .state
                .db
                .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
                .update_one(
                    doc! { "_id": hash_binding_id(fixture.binding_id.as_ref().unwrap()) },
                    doc! { "$unset": { "grant_version": "" } },
                )
                .await
                .unwrap();
        }
        let signed = fixture.request().await.unwrap();
        let verified = verify_consent_request(&fixture.state, &signed, &fixture.user_id).unwrap();
        let snapshot = verified.incremental_consent.unwrap();
        assert_eq!(
            snapshot.required_service_ids.len(),
            2,
            "duplicate IDs deduplicate"
        );
        assert_eq!(snapshot.current_service_ids.len(), 2);
        assert!(verified.external_subject_platform.is_none());
        let code = fixture.code(&signed).await;
        let stored = fixture
            .state
            .db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .find_one(doc! { "code_hash": crate::crypto::token::hash_token(&code) })
            .await
            .unwrap()
            .unwrap();
        let expected = incremental_consent_service::union(
            &[],
            &fixture.services[..4]
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>(),
        );
        assert_eq!(stored.allowed_service_ids, expected);
        assert!(
            stored.resource_uris.is_empty(),
            "persistent IDs are separate from access resources"
        );
        let Json(tokens) = fixture.exchange(&code).await.unwrap();
        let access = jwt::verify_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &tokens.access_token,
        )
        .unwrap();
        assert_eq!(
            access.allowed_service_ids,
            Some(vec![fixture.services[2].id.clone()])
        );
        let refresh = tokens.refresh_token.unwrap();
        let claims =
            jwt::verify_token(&fixture.state.jwt_keys, &fixture.state.config, &refresh).unwrap();
        let persisted = fixture
            .state
            .db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted.allowed_service_ids, expected);
        assert!(!persisted.allow_all_services);
        let binding_id = if with_binding {
            assert_eq!(tokens.binding_updated, Some(true));
            assert!(tokens.binding_id.is_none());
            fixture.binding_id.as_ref().unwrap().clone()
        } else {
            tokens.binding_id.unwrap()
        };
        let grant = oauth_broker_service::resolve_binding_grant_for_subject(
            &fixture.state.db,
            &fixture.client_id,
            &fixture.user_id,
            None,
            &hash_binding_id(&binding_id),
        )
        .await
        .unwrap();
        assert_eq!(grant.allowed_service_ids, expected);
        // Refresh without resource returns the complete grant, not only C.
        let request = serde_json::from_value(serde_json::json!({
            "grant_type": "refresh_token", "client_id": fixture.client_id, "refresh_token": refresh,
        }))
        .unwrap();
        let Json(refreshed) = token_inner(
            &fixture.state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            request,
        )
        .await
        .unwrap();
        let claims = jwt::verify_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &refreshed.access_token,
        )
        .unwrap();
        assert_eq!(claims.allowed_service_ids, Some(expected));
    }
}

#[tokio::test]
async fn incremental_retains_allow_all_for_refresh_but_narrows_access() {
    let fixture = IncrementalFixture::new(true, true).await;
    let code = fixture.code(&fixture.request().await.unwrap()).await;
    let Json(tokens) = fixture.exchange(&code).await.unwrap();
    let access = jwt::verify_token(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &tokens.access_token,
    )
    .unwrap();
    assert_eq!(
        access.allowed_service_ids,
        Some(vec![fixture.services[2].id.clone()])
    );
    let binding = load_binding(&fixture.state.db, fixture.binding_id.as_ref().unwrap()).await;
    let refresh = fixture
        .state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "jti": &binding.refresh_token_jti })
        .await
        .unwrap()
        .unwrap();
    assert!(refresh.allow_all_services);
}

#[tokio::test]
async fn incremental_concurrent_decisions_conflict_without_losing_grants() {
    let fixture = IncrementalFixture::new(false, false).await;
    let first = fixture.request().await.unwrap();
    let second = fixture.request().await.unwrap();
    let extra = [fixture.services[4].id.clone()];
    let (left, right) = tokio::join!(
        fixture.decide(&first, &extra, false, "allow"),
        fixture.decide(&second, &extra, false, "allow")
    );
    assert_eq!(
        [left.unwrap(), right.unwrap()]
            .iter()
            .filter(|response| incremental_callback_error(response).is_some())
            .count(),
        1
    );
    let consent =
        consent_service::check_consent(&fixture.state.db, &fixture.user_id, &fixture.client_id, "")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(consent.allowed_service_ids.unwrap().len(), 5);
}

#[tokio::test]
async fn incremental_revoke_before_decision_or_exchange_never_revives_consent() {
    for after_decision in [false, true] {
        let fixture = IncrementalFixture::new(false, false).await;
        let signed = fixture.request().await.unwrap();
        let code = if after_decision {
            Some(fixture.code(&signed).await)
        } else {
            None
        };
        consent_service::revoke_consent(&fixture.state.db, &fixture.user_id, &fixture.client_id)
            .await
            .unwrap();
        if let Some(code) = code {
            assert!(fixture.exchange(&code).await.is_err());
        } else {
            assert_eq!(
                incremental_callback_error(&fixture.decide(&signed, &[], false, "allow").await.unwrap()),
                Some("interaction_required".to_string())
            );
        }
        assert!(
            consent_service::check_consent(
                &fixture.state.db,
                &fixture.user_id,
                &fixture.client_id,
                ""
            )
            .await
            .unwrap()
            .is_none()
        );
        assert_eq!(
            fixture
                .state
                .db
                .collection::<RefreshToken>(REFRESH_TOKENS)
                .count_documents(doc! { "revoked": false })
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn incremental_broker_rotation_during_review_preserves_scopes_and_binding() {
    let fixture = IncrementalFixture::new(true, false).await;
    let signed = fixture.request().await.unwrap();
    let raw_binding = fixture.binding_id.as_ref().unwrap();
    let prior = load_binding(&fixture.state.db, raw_binding).await;
    oauth_broker_service::exchange_via_binding(
        &fixture.state.db,
        fixture.state.encryption_keys.clone(),
        &fixture.state.http_client,
        &fixture.state.jwt_keys,
        &fixture.state.config,
        false,
        &fixture.client_id,
        raw_binding,
        Some("openid"),
        None,
        None,
    ).await.unwrap();
    let rotated = load_binding(&fixture.state.db, raw_binding).await;
    assert_ne!(rotated.refresh_token_jti, prior.refresh_token_jti);
    assert_eq!(rotated.grant_version, prior.grant_version);
    let code = fixture.code(&signed).await;
    let Json(tokens) = fixture.exchange(&code).await.unwrap();
    assert_eq!(tokens.binding_updated, Some(true));
    let grant = oauth_broker_service::resolve_binding_grant_for_subject(
        &fixture.state.db, &fixture.client_id, &fixture.user_id, None,
        &hash_binding_id(raw_binding),
    ).await.unwrap();
    assert!(grant.scopes.contains("offline_access"));
    assert!(grant.scopes.contains("proxy"));
}

#[tokio::test]
async fn stale_incremental_binding_update_keeps_original_refresh_usable() {
    let fixture = IncrementalFixture::new(true, false).await;
    let signed = fixture.request().await.unwrap();
    let snapshot = verify_consent_request(&fixture.state, &signed, &fixture.user_id)
        .unwrap()
        .incremental_consent
        .unwrap();
    let raw_binding = fixture.binding_id.as_ref().unwrap();
    let original = load_binding(&fixture.state.db, raw_binding).await;
    let scope = fixture.params.scope.as_deref().unwrap();
    let replacement = oauth_service::issue_oauth_refresh_token(
        &fixture.state.db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &fixture.client_id,
        &fixture.user_id,
        scope,
        &[],
        &fixture.services[..4]
            .iter()
            .map(|service| service.id.clone())
            .collect::<Vec<_>>(),
        false,
    )
    .await
    .unwrap();

    consent_service::grant_consent_with_services(
        &fixture.state.db,
        &fixture.user_id,
        &fixture.client_id,
        scope,
        Some(vec![
            fixture.services[0].id.clone(),
            fixture.services[1].id.clone(),
        ]),
    )
    .await
    .unwrap();
    let scopes = scope.split_whitespace().map(str::to_string).collect::<Vec<_>>();
    assert!(oauth_broker_service::update_binding_grant_with_version(
        &fixture.state.db,
        &fixture.state.encryption_keys,
        &fixture.client_id,
        &fixture.user_id,
        &hash_binding_id(raw_binding),
        &replacement.refresh_token,
        &replacement.refresh_token_jti,
        &scopes,
        None,
        Some(&snapshot),
    )
    .await
    .is_err());

    let current = load_binding(&fixture.state.db, raw_binding).await;
    assert_eq!(current.refresh_token_jti, original.refresh_token_jti);
    assert_eq!(current.grant_version, original.grant_version);
    let old_refresh = fixture
        .state
        .db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "jti": &original.refresh_token_jti })
        .await
        .unwrap()
        .unwrap();
    assert!(!old_refresh.revoked);
    oauth_broker_service::exchange_via_binding(
        &fixture.state.db,
        fixture.state.encryption_keys.clone(),
        &fixture.state.http_client,
        &fixture.state.jwt_keys,
        &fixture.state.config,
        false,
        &fixture.client_id,
        raw_binding,
        None,
        None,
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn google_style_incremental_scope_request_adds_only_requested_permission() {
    let mut fixture = IncrementalFixture::new(false, false).await;
    fixture.state.db.collection::<crate::models::oauth_client::OauthClient>(
        crate::models::oauth_client::COLLECTION_NAME,
    ).update_one(
        doc! { "_id": &fixture.client_id },
        doc! { "$set": { "allowed_scopes": format!("{} account:write", fixture.params.scope.as_deref().unwrap()) } },
    ).await.unwrap();
    fixture.params.service_access_mode = None;
    fixture.params.include_granted_scopes = true;
    fixture.params.scope = Some("account:write".to_string());
    fixture.params.requested_service_ids.clear();
    fixture.params.resource.clear();
    let signed = fixture.request().await.unwrap();
    let snapshot = verify_consent_request(&fixture.state, &signed, &fixture.user_id)
        .unwrap().incremental_consent.unwrap();
    assert!(snapshot.current_scopes.contains("offline_access"));
    assert!(snapshot.scopes.contains("account:write"));
    let code = fixture.code(&signed).await;
    let Json(tokens) = fixture.exchange(&code).await.unwrap();
    let scope = tokens.scope.unwrap();
    assert!(scope.contains("openid"));
    assert!(scope.contains("account:write"));
}

#[tokio::test]
async fn omitted_scope_does_not_grant_new_client_allowed_scopes() {
    let mut fixture = IncrementalFixture::new(false, false).await;
    fixture.state.db.collection::<crate::models::oauth_client::OauthClient>(
        crate::models::oauth_client::COLLECTION_NAME,
    ).update_one(
        doc! { "_id": &fixture.client_id },
        doc! { "$set": { "allowed_scopes": format!("{} account:write", fixture.params.scope.as_deref().unwrap()) } },
    ).await.unwrap();
    fixture.params.scope = None;
    let signed = fixture.request().await.unwrap();
    let snapshot = verify_consent_request(&fixture.state, &signed, &fixture.user_id)
        .unwrap().incremental_consent.unwrap();
    assert_eq!(snapshot.scopes, snapshot.current_scopes);
    assert!(!snapshot.scopes.contains("account:write"));
}

#[tokio::test]
async fn unavailable_existing_service_does_not_block_additions() {
    let fixture = IncrementalFixture::new(true, false).await;
    fixture.state.db.collection::<UserService>(USER_SERVICES)
        .update_one(doc! { "_id": &fixture.services[1].id }, doc! { "$set": { "is_active": false } })
        .await.unwrap();
    let signed = fixture.request().await.unwrap();
    let code = fixture.code(&signed).await;
    let Json(tokens) = fixture.exchange(&code).await.unwrap();
    assert_eq!(tokens.binding_updated, Some(true));
}

#[tokio::test]
async fn narrowed_consent_cannot_be_widened_from_an_older_binding() {
    let fixture = IncrementalFixture::new(true, false).await;
    consent_service::grant_consent_with_services(
        &fixture.state.db,
        &fixture.user_id,
        &fixture.client_id,
        "openid proxy",
        Some(vec![fixture.services[0].id.clone()]),
    ).await.unwrap();
    assert!(matches!(fixture.request().await, Err(AppError::Conflict(_))));
}

#[tokio::test]
async fn review_handle_is_bound_to_the_user() {
    let fixture = IncrementalFixture::new(false, false).await;
    let signed = fixture.request().await.unwrap();
    let handle = oauth_consent_request_service::create(
        &fixture.state.db, &fixture.user_id, &signed,
    ).await.unwrap();
    assert_eq!(
        oauth_consent_request_service::get(&fixture.state.db, &fixture.user_id, &handle)
            .await.unwrap(),
        signed,
    );
    assert!(oauth_consent_request_service::get(
        &fixture.state.db, &Uuid::new_v4().to_string(), &handle,
    ).await.is_err());
}

#[tokio::test]
async fn expired_review_can_return_to_client_with_state() {
    let fixture = IncrementalFixture::new(false, false).await;
    let signed = fixture.request().await.unwrap();
    let mut claims = decode::<ConsentRequestClaims>(&signed, &fixture.state.jwt_keys.decoding, &{
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[CONSENT_REQUEST_AUDIENCE]);
        validation
    }).unwrap().claims;
    claims.exp = Utc::now().timestamp() - 120;
    let expired = encode(&Header::new(Algorithm::RS256), &claims, &fixture.state.jwt_keys.encoding).unwrap();
    let response = fixture.decide(&expired, &[], false, "deny").await.unwrap();
    assert_eq!(incremental_callback_error(&response), Some("access_denied".to_string()));
}

#[tokio::test]
async fn incremental_revoked_binding_refresh_invalidates_pending_code() {
    let fixture = IncrementalFixture::new(true, false).await;
    let code = fixture.code(&fixture.request().await.unwrap()).await;
    let binding = load_binding(&fixture.state.db, fixture.binding_id.as_ref().unwrap()).await;
    oauth_service::revoke_issued_refresh(&fixture.state.db, &binding.refresh_token_jti)
        .await.unwrap();
    assert!(fixture.exchange(&code).await.is_err());
    let current = load_binding(&fixture.state.db, fixture.binding_id.as_ref().unwrap()).await;
    assert_eq!(current.refresh_token_jti, binding.refresh_token_jti);
}

#[tokio::test]
async fn incremental_rejects_tampering_wildcard_wrong_user_and_expiry() {
    let fixture = IncrementalFixture::new(false, false).await;
    let signed = fixture.request().await.unwrap();
    assert!(incremental_callback_error(&fixture.decide(&signed, &[], true, "allow").await.unwrap()).is_some());
    assert!(verify_consent_request(&fixture.state, &signed, &Uuid::new_v4().to_string()).is_err());
    let mut claims = decode::<ConsentRequestClaims>(&signed, &fixture.state.jwt_keys.decoding, &{
        let mut v = Validation::new(Algorithm::RS256);
        v.set_audience(&[CONSENT_REQUEST_AUDIENCE]);
        v
    })
    .unwrap()
    .claims;
    claims.exp = Utc::now().timestamp() - 120;
    let expired = encode(
        &Header::new(Algorithm::RS256),
        &claims,
        &fixture.state.jwt_keys.encoding,
    )
    .unwrap();
    assert_eq!(
        incremental_callback_error(&fixture.decide(&expired, &[], false, "allow").await.unwrap()),
        Some("interaction_required".to_string())
    );
    let mut parts = signed.split('.').map(String::from).collect::<Vec<_>>();
    claims.exp = Utc::now().timestamp() + 600;
    claims
        .incremental_consent
        .as_mut()
        .unwrap()
        .allow_all_services = true;
    parts[1] = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    assert!(
        fixture
            .decide(&parts.join("."), &[], true, "allow")
            .await
            .is_err()
    );
    let denied = fixture.decide(&signed, &[], false, "deny").await.unwrap();
    assert!(
        denied.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .contains("error=access_denied")
    );
    let consent =
        consent_service::check_consent(&fixture.state.db, &fixture.user_id, &fixture.client_id, "")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(consent.allowed_service_ids.unwrap().len(), 2);
}

#[tokio::test]
async fn incremental_exact_ids_reject_unknown_inactive_and_foreign_services() {
    for kind in ["unknown", "inactive", "foreign"] {
        let mut fixture = IncrementalFixture::new(false, false).await;
        let id = match kind {
            "inactive" => {
                crate::services::service_history::collection::<UserService>(
                    &fixture.state.db,
                    USER_SERVICES,
                )
                .update_one(
                    doc! { "_id": &fixture.services[2].id },
                    doc! { "$set": { "is_active": false } },
                )
                .await
                .unwrap();
                fixture.services[2].id.clone()
            }
            "foreign" => {
                insert_user_service(&fixture.state.db, &Uuid::new_v4().to_string(), "new-c")
                    .await
                    .id
            }
            _ => Uuid::new_v4().to_string(),
        };
        fixture.params.requested_service_ids = vec![id];
        fixture.params.resource.clear();
        assert!(
            fixture.request().await.is_err(),
            "{kind} ID must fail closed"
        );
    }
}

#[tokio::test]
async fn incremental_par_preserves_mode_and_exact_ids_and_ignores_unsigned_snapshot() {
    let fixture = IncrementalFixture::new(false, false).await;
    let (uri, _) = par_service::create_request_with_service_access(
        &fixture.state.db,
        &fixture.client_id,
        "code",
        &fixture.params.redirect_uri,
        fixture.params.scope.as_deref(),
        None,
        fixture.params.code_challenge.as_deref(),
        Some("S256"),
        None,
        Some("consent"),
        &[],
        None,
        None,
        Some(ServiceAccessMode::Incremental),
        &fixture.params.requested_service_ids,
    )
    .await
    .unwrap();
    let input: AuthorizeQuery = serde_json::from_value(serde_json::json!({
        "client_id": fixture.client_id, "request_uri": uri,
        "incremental_consent": {"allow_all_services":true},
    }))
    .unwrap();
    assert!(input.incremental_consent.is_none());
    let restored = resolve_pushed_authorize_params(&fixture.state, input)
        .await
        .unwrap();
    assert_eq!(
        restored.service_access_mode,
        Some(ServiceAccessMode::Incremental)
    );
    assert_eq!(
        restored.requested_service_ids,
        fixture.params.requested_service_ids
    );
    assert!(
        build_authorize_url("https://nyx.example", &restored)
            .contains("service_access_mode=incremental")
    );
}

#[tokio::test]
async fn incremental_revocation_overlapping_token_insertion_cleans_new_refresh() {
    let fixture = IncrementalFixture::new(false, false).await;
    let code = fixture.code(&fixture.request().await.unwrap()).await;
    let db = fixture.state.db.clone();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    // Hold a delete uncommitted while token issuance can still read the old
    // grant. Its post-insert write fence must wait and reject after commit.
    db.collection::<Consent>(CONSENTS)
        .delete_one(doc! { "user_id": &fixture.user_id })
        .session(&mut session)
        .await
        .unwrap();
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_many(
            doc! { "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .session(&mut session)
        .await
        .unwrap();
    let exchange = tokio::spawn(async move { fixture.exchange(&code).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if db
                .collection::<RefreshToken>(REFRESH_TOKENS)
                .count_documents(doc! {})
                .await
                .unwrap()
                > 0
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("token must be inserted before revoke commits");
    session.commit_transaction().await.unwrap();
    assert!(exchange.await.unwrap().is_err());
    assert_eq!(
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! { "revoked": false })
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn incremental_org_ids_use_live_membership_without_same_slug_substitution() {
    let mut fixture = IncrementalFixture::new(false, false).await;
    let db = &fixture.state.db;
    let org_id = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org_id, UserType::Org))
        .await
        .unwrap();
    let membership = test_membership(&org_id, &fixture.user_id, OrgRole::Member, None);
    db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    let shared = insert_user_service(db, &org_id, "new-c").await;
    fixture.params.requested_service_ids = vec![shared.id.clone()];
    fixture.params.resource.clear();
    let signed = fixture.request().await.unwrap();
    let snapshot = verify_consent_request(&fixture.state, &signed, &fixture.user_id)
        .unwrap()
        .incremental_consent
        .unwrap();
    assert_eq!(snapshot.required_service_ids, vec![shared.id]);
    // Removing membership after the page opened must invalidate the exact org
    // service; a personal service with the same slug cannot satisfy it.
    db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
        .delete_one(doc! { "_id": &membership.id })
        .await
        .unwrap();
    assert_eq!(incremental_callback_error(&fixture.decide(&signed, &[], false, "allow").await.unwrap()), Some("interaction_required".to_string()));
}

#[tokio::test]
async fn incremental_legacy_replica_change_invalidates_unversioned_snapshot() {
    let fixture = IncrementalFixture::new(false, false).await;
    let consents = fixture.state.db.collection::<Consent>(CONSENTS);
    consents
        .update_one(
            doc! { "user_id": &fixture.user_id },
            doc! { "$unset": { "revision": "" } },
        )
        .await
        .unwrap();
    let signed = fixture.request().await.unwrap();
    consents
        .update_one(
            doc! { "user_id": &fixture.user_id },
            doc! { "$set": { "allowed_service_ids": [] } },
        )
        .await
        .unwrap();
    assert_eq!(incremental_callback_error(&fixture.decide(&signed, &[], false, "allow").await.unwrap()), Some("interaction_required".to_string()));
}

#[test]
fn incremental_wire_contract_rejects_unknown_modes_and_invalid_exact_ids() {
    let input = serde_json::json!({ "response_type": "code", "client_id": "c", "service_access_mode": "merge" });
    assert!(serde_json::from_value::<AuthorizeQuery>(input).is_err());
    assert!(validate_service_access_request(None, &[Uuid::new_v4().to_string()]).is_err());
    assert!(
        validate_service_access_request(
            Some(ServiceAccessMode::Incremental),
            &["api-github".to_string()]
        )
        .is_err()
    );
    assert!(
        validate_service_access_request(
            Some(ServiceAccessMode::Incremental),
            &vec![Uuid::new_v4().to_string(); 101]
        )
        .is_err()
    );
}
