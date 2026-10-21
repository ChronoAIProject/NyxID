use super::*;
use crate::models::{
    downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
    provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig},
    service_provider_requirement::{COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement},
    user_provider_token::{COLLECTION_NAME as TOKENS, UserProviderToken},
    user_service::{AUTO_PROVISION_SOURCE, COLLECTION_NAME as CONNECTIONS, UserService},
};
use crate::services::credential_record_service as records;
use crate::services::delegation_service::{
    materialize_optional_auto_provision_snapshot, read_optional_auto_provision_snapshot,
};
use futures::TryStreamExt;
use std::sync::Mutex;

type RefreshMutation = (&'static str, bson::Document, bson::Document);
struct OptionalFixture {
    state: AppState,
    caller: ExactServiceApprovalCaller,
    catalog: DownstreamService,
    connection: UserService,
    requirement: ServiceProviderRequirement,
    endpoint_id: String,
    effects: Arc<Mutex<Vec<Option<String>>>>,
    refreshes: Arc<AtomicUsize>,
    refresh_mutations: Arc<Mutex<Vec<RefreshMutation>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for OptionalFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn optional_fixture() -> OptionalFixture {
    let db = crate::test_utils::connect_test_database("exact_optional_provider")
        .await
        .unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let effects = Arc::new(Mutex::new(Vec::new()));
    let refreshes = Arc::new(AtomicUsize::new(0));
    let refresh_mutations: Arc<Mutex<Vec<RefreshMutation>>> = Arc::new(Mutex::new(Vec::new()));
    let effect_spy = effects.clone();
    let refresh_spy = refreshes.clone();
    let mutations = refresh_mutations.clone();
    let refresh_db = db.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/effect", axum::routing::get(move |headers: axum::http::HeaderMap| {
                let spy = effect_spy.clone();
                async move {
                    spy.lock().unwrap().push(headers.get("X-GitHub-Token").map(|v| v.to_str().unwrap().to_owned()));
                    axum::Json(serde_json::json!({"ok":true}))
                }
            }))
            .route("/refresh", axum::routing::post(move || {
                let spy = refresh_spy.clone();
                let mutations = mutations.clone();
                let db = refresh_db.clone();
                async move {
                    spy.fetch_add(1, Ordering::SeqCst);
                    let changes = std::mem::take(&mut *mutations.lock().unwrap());
                    for (collection, filter, update) in changes {
                        db.collection::<bson::Document>(collection).update_one(filter, update).await.unwrap();
                    }
                    axum::Json(serde_json::json!({"access_token":"refreshed-owner-token","refresh_token":"rotated-refresh","expires_in":3600}))
                }
            })))
            .await.unwrap();
    });
    let now = bson::DateTime::now();
    let provider: ProviderConfig = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "slug":"github", "name":"GitHub",
        "provider_type":"oauth2", "is_active":true, "created_by":"test",
        "token_url":format!("{base_url}/refresh"), "created_at":now, "updated_at":now,
    })
    .unwrap();
    let mut provider = provider;
    for (field, value) in [
        ("client_id_encrypted", b"test-client".as_slice()),
        ("client_secret_encrypted", b"test-secret".as_slice()),
    ] {
        let encrypted = records::encrypt(
            &state.encryption_keys,
            PROVIDERS,
            &provider.id,
            records::PLATFORM_OWNER,
            field,
            value,
        )
        .await
        .unwrap();
        records::mark_version(
            &mut provider.secret_context_versions,
            &state.encryption_keys,
            field,
        );
        match field {
            "client_id_encrypted" => provider.client_id_encrypted = Some(encrypted),
            _ => provider.client_secret_encrypted = Some(encrypted),
        }
    }
    db.collection::<ProviderConfig>(PROVIDERS)
        .insert_one(&provider)
        .await
        .unwrap();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.base_url = base_url;
    catalog.service_category = "internal".into();
    catalog.slug = format!("exact-optional-{}", uuid::Uuid::new_v4());
    db.collection::<DownstreamService>(CATALOG)
        .insert_one(&catalog)
        .await
        .unwrap();
    let endpoint =
        crate::test_utils::test_service_endpoint(&catalog.id, "effect", "GET", "/effect");
    let endpoint_id = endpoint.id.clone();
    db.collection(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(endpoint)
        .await
        .unwrap();
    let requirement = ServiceProviderRequirement {
        id: uuid::Uuid::new_v4().to_string(),
        service_id: catalog.id.clone(),
        provider_config_id: provider.id,
        required: false,
        scopes: None,
        injection_method: "header".into(),
        injection_key: Some("X-GitHub-Token".into()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .insert_one(&requirement)
        .await
        .unwrap();
    let listed = Box::pin(crate::handlers::keys::list_keys(
        axum::extract::State(state.clone()),
        crate::test_utils::test_auth_user(&owner),
    ))
    .await
    .unwrap();
    let connection = db
        .collection::<UserService>(CONNECTIONS)
        .find_one(doc! {
            "user_id":&owner, "catalog_service_id":&catalog.id, "source":AUTO_PROVISION_SOURCE,
        })
        .await
        .unwrap()
        .unwrap();
    assert!(listed.0.keys.iter().any(|key| {
        key.id == connection.id && key.catalog_service_id.as_deref() == Some(catalog.id.as_str())
    }));
    OptionalFixture {
        state,
        caller: caller_for(&owner),
        catalog,
        connection,
        requirement,
        endpoint_id,
        effects,
        refreshes,
        refresh_mutations,
        server,
    }
}

impl OptionalFixture {
    async fn operation(&self) -> ExactCatalogResolution<'static> {
        Box::pin(resolve_exact_catalog(
            &self.state,
            &self.caller,
            &self.connection.id,
            &self.endpoint_id,
            &serde_json::json!({}),
        ))
        .await
        .unwrap()
    }
    async fn authority(&self) -> ResolvedExecution {
        let operation = self.operation().await;
        Box::pin(resolve_execution_authority(
            &self.state,
            &self.caller,
            &self.connection.id,
            &self.connection.slug,
            ExecutionResolutionMode::ReadOnlySnapshot,
            &operation,
            &serde_json::json!({}),
        ))
        .await
        .unwrap()
    }
    async fn token(&self, refresh: bool) -> UserProviderToken {
        let id = uuid::Uuid::new_v4().to_string();
        let (versions, access, refresh_token) = records::oauth_token_fields(
            &self.state.encryption_keys,
            &id,
            &self.caller.proxy_resolution_user_id,
            None,
            "owner-token",
            refresh.then_some("refresh"),
        )
        .await
        .unwrap();
        let token = UserProviderToken {
            secret_context_versions: versions,
            id,
            user_id: self.caller.proxy_resolution_user_id.clone(),
            provider_config_id: self.requirement.provider_config_id.clone(),
            connection_id: None,
            credential_user_id: None,
            token_type: "oauth2".into(),
            access_token_encrypted: Some(access),
            refresh_token_encrypted: refresh_token,
            token_scopes: None,
            expires_at: Some(
                Utc::now()
                    + if refresh {
                        Duration::minutes(-1)
                    } else {
                        Duration::hours(1)
                    },
            ),
            api_key_encrypted: None,
            status: "active".into(),
            state_version: 1,
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            gateway_url: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.state
            .db
            .collection::<UserProviderToken>(TOKENS)
            .insert_one(&token)
            .await
            .unwrap();
        token
    }
    async fn bound(&self) -> ExactServiceApprovalBinding {
        let operation = self.operation().await;
        let authority = self.authority().await;
        let mut value = binding();
        value.actor_user_id = self.caller.actor_user_id.clone();
        value.user_service_id = self.connection.id.clone();
        value.endpoint_id = self.endpoint_id.clone();
        value.operation_id = self.endpoint_id.clone();
        value.catalog_digest = operation.catalog_digest;
        value.exact_view_digest = Some(operation.legacy_exact_view_digest);
        value.exact_view_digest_binding = Some(operation.exact_view_digest);
        value.endpoint_contract_digest = operation.endpoint_contract_digest;
        value.operation_digest = operation.operation_digest;
        value.operation_generation = operation.operation_generation;
        value.producer_generation_bound = operation.producer_generation_bound;
        value.arguments = serde_json::json!({});
        value.execution_authority_digest = Some(authority.legacy_digest);
        value.execution_authority_binding = Some(ExactServiceExecutionAuthorityBinding {
            projection_version: execution_authority::CONTRACT_VERSION.into(),
            digest: authority.digest,
        });
        value.optional_provider_authority_digest = authority.optional_provider_authority_digest;
        value
    }
    async fn assert_drift(&self, stored: &ExactServiceApprovalBinding) {
        let live = self.authority().await;
        assert_eq!(
            stored.execution_authority_digest.as_deref(),
            Some(live.legacy_digest.as_str()),
            "real v1 stays unchanged: this was the unfenced authority gap"
        );
        assert_eq!(
            stored.execution_authority_binding.as_ref().unwrap().digest,
            live.digest,
            "real v2 stays unchanged"
        );
        assert!(
            matches!(evaluate_authority(stored,live), ExecutionAuthorityEvaluation::Terminal(t)
            if t.state == ExactServiceApprovalState::Drifted && t.failure_code == "execution_authority_drift")
        );
        assert!(self.effects.lock().unwrap().is_empty());
        assert_eq!(self.refreshes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn optional_authority_read_only_binds_policy_addition_and_token_presence() {
    let f = optional_fixture().await;
    let without_token = f.bound().await;
    let token = f.token(true).await;
    f.assert_drift(&without_token).await;
    let stored = f.bound().await;
    let before = f
        .state
        .db
        .collection::<bson::Document>(TOKENS)
        .find_one(doc! {"_id":&token.id})
        .await
        .unwrap()
        .unwrap();
    f.authority().await;
    assert_eq!(
        before,
        f.state
            .db
            .collection::<bson::Document>(TOKENS)
            .find_one(doc! {"_id":&token.id})
            .await
            .unwrap()
            .unwrap(),
        "read-only snapshot must not decrypt/refresh/touch usage"
    );
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! {"_id":&f.requirement.id},
            doc! {"$set":{"injection_key":"X-Changed"}},
        )
        .await
        .unwrap();
    f.assert_drift(&stored).await;
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! {"_id":&f.requirement.id},
            doc! {"$set":{"injection_key":"X-GitHub-Token"}},
        )
        .await
        .unwrap();
    let mut added = f.requirement.clone();
    added.id = uuid::Uuid::new_v4().to_string();
    added.provider_config_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .insert_one(added)
        .await
        .unwrap();
    f.assert_drift(&stored).await;
}

#[tokio::test]
async fn optional_authority_binds_replacement_type_status_and_missing_legacy_fence() {
    let f = optional_fixture().await;
    let token = f.token(false).await;
    let stored = f.bound().await;
    let mut legacy = stored.clone();
    legacy.optional_provider_authority_digest = None;
    f.assert_drift(&legacy).await;
    for update in [
        doc! {"$inc":{"state_version":1_i64}},
        doc! {"$set":{"token_type":"api_key"}},
        doc! {"$set":{"status":"revoked"}},
        doc! {"$set":{"access_token_encrypted":bson::Bson::Null}},
    ] {
        f.state
            .db
            .collection::<UserProviderToken>(TOKENS)
            .replace_one(doc! {"_id": &token.id}, &token)
            .await
            .unwrap();
        f.state
            .db
            .collection::<UserProviderToken>(TOKENS)
            .update_one(doc! {"_id":&token.id}, update)
            .await
            .unwrap();
        f.assert_drift(&stored).await;
    }
    // Reconnect can replace the entire row as well as advancing its lineage.
    f.state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .delete_one(doc! {"_id":&token.id})
        .await
        .unwrap();
    let replacement = f.token(false).await;
    assert_ne!(replacement.id, token.id);
    f.assert_drift(&stored).await;
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .delete_many(doc! {"service_id":&f.catalog.id})
        .await
        .unwrap();
    let mut no_requirements = f.bound().await;
    no_requirements.optional_provider_authority_digest = None;
    assert!(
        matches!(
            evaluate_authority(&no_requirements, f.authority().await),
            ExecutionAuthorityEvaluation::Matched(_)
        ),
        "historical no-provider routes retain existing compatibility"
    );
}

#[tokio::test]
async fn optional_authority_is_ordered_and_normalizes_telegram_policy() {
    let f = optional_fixture().await;
    let mut second = f.requirement.clone();
    second.id = uuid::Uuid::new_v4().to_string();
    second.provider_config_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .insert_one(&second)
        .await
        .unwrap();
    let first = f.authority().await.optional_provider_authority_digest;
    let rows: Vec<ServiceProviderRequirement> = f
        .state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .find(doc! {"service_id":&f.catalog.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .delete_many(doc! {"service_id":&f.catalog.id})
        .await
        .unwrap();
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .insert_many(rows.into_iter().rev())
        .await
        .unwrap();
    assert_eq!(
        first,
        f.authority().await.optional_provider_authority_digest
    );
    f.state
        .db
        .collection::<ProviderConfig>(PROVIDERS)
        .update_one(
            doc! {"_id":&f.requirement.provider_config_id},
            doc! {"$set":{"slug":"telegram-bot"}},
        )
        .await
        .unwrap();
    assert_ne!(
        first,
        f.authority().await.optional_provider_authority_digest
    );
    let telegram = f.authority().await.optional_provider_authority_digest;
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! {"_id":&f.requirement.id},
            doc! {"$set":{"injection_method":"query","injection_key":"ignored"}},
        )
        .await
        .unwrap();
    assert_eq!(
        telegram,
        f.authority().await.optional_provider_authority_digest,
        "bind normalized injection actually used by Telegram"
    );
    f.state
        .db
        .collection::<ProviderConfig>(PROVIDERS)
        .update_one(
            doc! {"_id":&f.requirement.provider_config_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    assert_ne!(
        telegram,
        f.authority().await.optional_provider_authority_digest
    );
}

#[tokio::test]
async fn optional_authority_pins_policy_through_refresh_and_rejects_token_replacement() {
    let f = optional_fixture().await;
    let token = f.token(true).await;
    let authority = f.authority().await;
    let target = &authority.resolution.target;
    let snapshot = read_optional_auto_provision_snapshot(
        &f.state.db,
        &f.caller.proxy_resolution_user_id,
        &f.connection.id,
        &f.catalog.id,
        target,
    )
    .await
    .unwrap();
    // Deterministic A -> B -> A during materialization. The refresh endpoint
    // restores A; only the policy captured in the snapshot may form the vector.
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! {"_id":&f.requirement.id},
            doc! {"$set":{"injection_key":"X-Unapproved"}},
        )
        .await
        .unwrap();
    f.refresh_mutations.lock().unwrap().push((
        REQUIREMENTS,
        doc! {"_id":&f.requirement.id},
        doc! {"$set":{"injection_key":"X-GitHub-Token"}},
    ));
    let credentials = Box::pin(materialize_optional_auto_provision_snapshot(
        &f.state.db,
        &f.state.encryption_keys,
        target,
        &snapshot,
        None,
    ))
    .await
    .unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].injection_key, "X-GitHub-Token");
    assert_eq!(credentials[0].credential, "refreshed-owner-token");
    assert_eq!(
        snapshot.digest,
        f.authority().await.optional_provider_authority_digest,
        "same lineage refresh is authority neutral"
    );
    let refreshed = f
        .state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .find_one(doc! {"_id":&token.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.state_version, token.state_version);
    assert!(refreshed.last_refreshed_at.is_some());
    assert_eq!(f.refreshes.load(Ordering::SeqCst), 1);
    assert!(f.effects.lock().unwrap().is_empty());
    // A replacement while the refresh is in flight must not release its token.
    f.state.db.collection::<UserProviderToken>(TOKENS).update_one(doc! {"_id":&token.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(1))}}).await.unwrap();
    let snapshot = read_optional_auto_provision_snapshot(
        &f.state.db,
        &f.caller.proxy_resolution_user_id,
        &f.connection.id,
        &f.catalog.id,
        target,
    )
    .await
    .unwrap();
    f.refresh_mutations.lock().unwrap().push((
        TOKENS,
        doc! {"_id":&token.id},
        doc! {"$inc":{"state_version":1_i64}},
    ));
    assert!(
        Box::pin(materialize_optional_auto_provision_snapshot(
            &f.state.db,
            &f.state.encryption_keys,
            target,
            &snapshot,
            None
        ))
        .await
        .is_err()
    );
    assert!(f.effects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn optional_authority_exact_redemption_forwards_frozen_header_once() {
    let f = optional_fixture().await;
    let mut token = f.token(false).await;
    // A pending approval created before storage migration keeps its lineage.
    token.secret_context_versions.clear();
    token.access_token_encrypted = Some(
        f.state
            .encryption_keys
            .encrypt(b"owner-token")
            .await
            .unwrap(),
    );
    f.state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .replace_one(doc! {"_id": &token.id}, &token)
        .await
        .unwrap();
    // Exercise the actual approval producer, including persisted additive
    // authority, read-only observation, decision, redemption and replay.
    let operation = f.operation().await;
    let approval_target = approval_target(&f.state, &f.caller, operation.service())
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: f.caller.approval_owner_user_id.clone(),
                service_id: approval_target.service_id,
                service_name: f.catalog.name.clone(),
                approval_required: true,
                approval_mode: ApprovalMode::PerRequest,
                rules: Vec::new(),
                default_effect: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    let created = Box::pin(create_request(
        &f.state,
        &f.caller,
        ExactServiceApprovalCreate {
            user_service_id: f.connection.id.clone(),
            endpoint_id: f.endpoint_id.clone(),
            catalog_digest: operation.catalog_digest,
            exact_view_digest: Some(operation.exact_view_digest),
            endpoint_contract_digest: operation.endpoint_contract_digest,
            operation_digest: operation.operation_digest,
            operation_id: f.endpoint_id.clone(),
            operation_generation: Some(operation.operation_generation),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            arguments: serde_json::json!({}),
        },
    ))
    .await
    .unwrap();
    assert_eq!(created.state, ExactServiceApprovalState::Pending);
    let request_id = created.request_id;
    let persisted = f
        .state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":&request_id})
        .await
        .unwrap()
        .unwrap();
    let stored = persisted.exact_service.unwrap();
    assert!(stored.optional_provider_authority_digest.is_some());
    assert_eq!(
        stored.optional_provider_authority_digest,
        f.authority().await.optional_provider_authority_digest
    );
    let source = f
        .state
        .db
        .collection::<bson::Document>(TOKENS)
        .find_one(doc! {"_id": &token.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        records::migrate_snapshot(&f.state.db, &f.state.encryption_keys, TOKENS, &source)
            .await
            .unwrap()
    );
    let migrated = f
        .state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .find_one(doc! {"_id": &token.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        migrated
            .secret_context_versions
            .get("access_token_encrypted"),
        Some(&3)
    );
    assert_eq!(migrated.state_version, token.state_version);
    assert_ne!(
        migrated.access_token_encrypted,
        token.access_token_encrypted
    );
    assert_eq!(
        stored.optional_provider_authority_digest,
        f.authority().await.optional_provider_authority_digest,
        "storage-only migration must preserve the pending approval's optional authority"
    );
    let pending_before = f
        .state
        .db
        .collection::<bson::Document>(TOKENS)
        .find_one(doc! {"_id": &token.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Box::pin(observe_request(&f.state, &f.caller, &request_id))
            .await
            .unwrap()
            .state,
        ExactServiceApprovalState::Pending
    );
    assert_eq!(
        Some(pending_before),
        f.state
            .db
            .collection::<bson::Document>(TOKENS)
            .find_one(doc! {"_id": &token.id})
            .await
            .unwrap(),
        "observe stays metadata-only after migration"
    );
    Box::pin(approval_service::process_decision(
        &f.state.db,
        &f.state.config,
        &f.state.http_client,
        None,
        None,
        &request_id,
        true,
        None,
        None,
        "web",
    ))
    .await
    .unwrap();
    let before = f
        .state
        .db
        .collection::<bson::Document>(TOKENS)
        .find_one(doc! {"user_id":&f.caller.proxy_resolution_user_id})
        .await
        .unwrap()
        .unwrap();
    let observed = Box::pin(observe_request(&f.state, &f.caller, &request_id))
        .await
        .unwrap();
    assert_eq!(observed.state, ExactServiceApprovalState::Approved);
    assert_eq!(
        before,
        f.state
            .db
            .collection::<bson::Document>(TOKENS)
            .find_one(doc! {"user_id":&f.caller.proxy_resolution_user_id})
            .await
            .unwrap()
            .unwrap()
    );
    assert!(f.effects.lock().unwrap().is_empty());
    for _ in 0..2 {
        let result = Box::pin(redeem_request(
            &f.state,
            &f.caller,
            &request_id,
            ExactServiceApprovalFence {
                catalog_digest: stored.catalog_digest.clone(),
                exact_view_digest: stored.exact_view_digest_binding.clone(),
                operation_digest: stored.operation_digest.clone(),
                operation_id: stored.operation_id.clone(),
                operation_generation: stored.operation_generation,
                idempotency_key: stored.effect_idempotency_key.clone(),
            },
            crate::services::billing::route_inventory::enforce_billing_egress_classification(
                Some(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::route_inventory::BillingIngress::Mcp,
                    ),
                ),
                crate::services::billing::route_inventory::BillingIngress::Mcp,
            )
            .unwrap(),
        ))
        .await
        .unwrap();
        assert_eq!(
            result.state,
            ExactServiceApprovalState::Redeemed,
            "{:?}",
            result.failure_code
        );
    }
    assert_eq!(*f.effects.lock().unwrap(), vec![Some("owner-token".into())]);
    assert_eq!(f.refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn optional_authority_pinned_contextual_token_rejects_cross_record_ciphertext() {
    let f = optional_fixture().await;
    let token = f.token(false).await;
    assert_eq!(
        token.secret_context_versions.get("access_token_encrypted"),
        Some(&3)
    );
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_allow_legacy_record_reads(false);
    let authority = f.authority().await;
    let snapshot = read_optional_auto_provision_snapshot(
        &f.state.db,
        &f.caller.proxy_resolution_user_id,
        &f.connection.id,
        &f.catalog.id,
        &authority.resolution.target,
    )
    .await
    .unwrap();
    let credentials = Box::pin(materialize_optional_auto_provision_snapshot(
        &f.state.db,
        &keys,
        &authority.resolution.target,
        &snapshot,
        None,
    ))
    .await
    .unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].credential, "owner-token");
    let foreign_id = uuid::Uuid::new_v4().to_string();
    let foreign_ciphertext = records::encrypt(
        &keys,
        TOKENS,
        &foreign_id,
        &token.user_id,
        "access_token_encrypted",
        b"foreign-token",
    )
    .await
    .unwrap();
    let mut substituted = token.clone();
    substituted.access_token_encrypted = Some(foreign_ciphertext);
    f.state
        .db
        .collection::<UserProviderToken>(TOKENS)
        .replace_one(doc! {"_id": &token.id}, &substituted)
        .await
        .unwrap();
    assert!(
        crate::services::user_token_service::get_active_token_at_revision(
            &f.state.db,
            &keys,
            &token,
            None,
        )
        .await
        .is_err(),
        "the pinned reader must authenticate the persisted record identity"
    );
    assert_eq!(
        snapshot.digest,
        f.authority().await.optional_provider_authority_digest,
        "storage bytes are not approval authority"
    );
    let credentials = Box::pin(materialize_optional_auto_provision_snapshot(
        &f.state.db,
        &keys,
        &authority.resolution.target,
        &snapshot,
        None,
    ))
    .await
    .unwrap();
    assert!(
        credentials.is_empty(),
        "unavailable optional material must never inject foreign plaintext"
    );
    assert!(f.effects.lock().unwrap().is_empty());
    assert_eq!(f.refreshes.load(Ordering::SeqCst), 0);
}
