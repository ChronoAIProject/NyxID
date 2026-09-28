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
        Ok(url
            .query_pairs()
            .find(|(key, _)| key == "consent_request")
            .unwrap()
            .1
            .into_owned())
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
            "allow_all_services": all,
        }))
        .unwrap();
        authorize_decision(
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
                    doc! { "$unset": { "rotation_version": "" } },
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
    assert!(matches!(
        (&left, &right),
        (Ok(_), Err(AppError::Conflict(_))) | (Err(AppError::Conflict(_)), Ok(_))
    ));
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
            assert!(matches!(
                fixture.decide(&signed, &[], false, "allow").await,
                Err(AppError::Conflict(_))
            ));
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
async fn incremental_binding_rotation_or_refresh_revocation_invalidates_pending_code() {
    for revoke_refresh in [false, true] {
        let fixture = IncrementalFixture::new(true, false).await;
        let code = fixture.code(&fixture.request().await.unwrap()).await;
        let binding = load_binding(&fixture.state.db, fixture.binding_id.as_ref().unwrap()).await;
        if revoke_refresh {
            oauth_service::revoke_issued_refresh(&fixture.state.db, &binding.refresh_token_jti)
                .await
                .unwrap();
        } else {
            fixture
                .state
                .db
                .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
                .update_one(
                    doc! { "_id": &binding.id },
                    doc! { "$inc": { "rotation_version": 1 } },
                )
                .await
                .unwrap();
        }
        assert!(fixture.exchange(&code).await.is_err());
        let current = load_binding(&fixture.state.db, fixture.binding_id.as_ref().unwrap()).await;
        assert_eq!(current.refresh_token_jti, binding.refresh_token_jti);
    }
}

#[tokio::test]
async fn incremental_rejects_tampering_wildcard_wrong_user_and_expiry() {
    let fixture = IncrementalFixture::new(false, false).await;
    let signed = fixture.request().await.unwrap();
    assert!(fixture.decide(&signed, &[], true, "allow").await.is_err());
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
    assert!(fixture.decide(&expired, &[], false, "allow").await.is_err());
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
    assert!(fixture.decide(&signed, &[], false, "allow").await.is_err());
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
    assert!(matches!(
        fixture.decide(&signed, &[], false, "allow").await,
        Err(AppError::Conflict(_))
    ));
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
