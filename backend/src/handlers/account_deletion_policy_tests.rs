//! Mounted owner deletion ingress: accepted disclosure is durable authority,
//! while account absence, stale policy, and delegated credentials are not.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use bson::{Bson, Document, doc};
use chrono::Utc;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    AppState,
    models::{
        assistant_action_receipt::COLLECTION_NAME as RECEIPTS,
        remote_machine::{
            AuthorizationState, COLLECTION_NAME as MACHINES, ManagedMachine, ResourceState,
        },
        remote_machine_operation::COLLECTION_NAME as OPERATIONS,
        remote_machine_outbox::COLLECTION_NAME as OUTBOX,
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{key_service, remote_machine_service as lifecycle},
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};

const DIRECT: &str = "/api/v1/users/me";
const ASSISTANT: &str = "/api/v1/assistant/actions/org/account/delete";
const POLICY: &str = crate::services::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION;
const POLICY_HEADER: &str = "X-NyxID-Account-Deletion-Policy";

struct Fixture {
    state: AppState,
    app: Router,
    owner: String,
    email: String,
    machine: String,
    human_token: String,
}

impl Fixture {
    async fn new(label: &str) -> Self {
        // This helper requires a live transaction-capable database; unavailable
        // Mongo is a test failure, never a silently passing skipped test.
        let db = connect_transaction_test_database(label).await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        crate::services::personal_erasure_service::ensure_indexes(&db)
            .await
            .unwrap();
        lifecycle::ensure_indexes(&db)
            .await
            .expect("lifecycle indexes");
        // The mounted profile read resolves the seeded platform role IDs even
        // for an ordinary owner. Match production startup before using that
        // read to prove each credential is valid.
        crate::services::role_service::seed_system_roles(&db)
            .await
            .expect("seed platform roles for authenticated profile read");
        let owner = Uuid::new_v4().to_string();
        let user = test_user(&owner, UserType::Person);
        let email = user.email.clone();
        db.collection::<User>(USERS)
            .insert_one(user)
            .await
            .expect("insert human owner");
        let machine = Uuid::new_v4().to_string();
        let mut retained = lifecycle::CreateMachineInput::fixed(
            &owner,
            true,
            "registry.test/node@sha256:0123456789abcdef",
        )
        .preview(&machine, Utc::now());
        retained.resource_state = ResourceState::Paused;
        retained.authorization_state = AuthorizationState::Revoked;
        retained.sandbox_uid = Some(Uuid::new_v4().to_string());
        retained.pvc_uid = Some(Uuid::new_v4().to_string());
        retained.pvc_name = Some("retained-workspace".into());
        db.collection::<ManagedMachine>(MACHINES)
            .insert_one(retained)
            .await
            .expect("insert paused retained workspace");
        let state = test_app_state(db);
        let human_token = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(&owner).expect("owner UUID"),
            "account:read",
            None,
            None,
            None,
            None,
            None,
        )
        .expect("sign first-party human token");
        let (_, private) = crate::routes::build_router_with_state(state.clone());
        // /users/me intentionally rejects API keys before its extractor. This
        // fixture-only read proves the same live AuthUser authentication and
        // exact owner identity without weakening the production deletion routes.
        let app = private
            .route(
                "/api/v1/account-deletion-tests/identity",
                axum::routing::get(|auth: crate::mw::auth::AuthUser| async move {
                    axum::Json(json!({"id":auth.user_id.to_string()}))
                }),
            )
            .with_state(state.clone());
        Self {
            state,
            app,
            owner,
            email,
            machine,
            human_token,
        }
    }

    async fn request(
        &self,
        token: &str,
        api_key: bool,
        method: &str,
        path: &str,
        policy_header: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        request = if api_key {
            request.header("x-api-key", token)
        } else {
            request.header("authorization", format!("Bearer {token}"))
        };
        if let Some(policy) = policy_header {
            request = request.header(POLICY_HEADER, policy);
        }
        let response = self
            .app
            .clone()
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                    .expect("request"),
            )
            .await
            .expect("mounted router response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("bounded response body");
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"body":String::from_utf8_lossy(&bytes)}));
        (status, body)
    }

    fn assistant_body(&self, id: &str, policy: Option<&str>) -> Value {
        let mut body = json!({"actionRequestId":id,"confirmEmail":self.email});
        if let Some(policy) = policy {
            body["deletionPolicyVersion"] = json!(policy);
        }
        body
    }

    async fn assert_authenticated_read(&self, token: &str, api_key: bool) {
        let path = if api_key {
            "/api/v1/account-deletion-tests/identity"
        } else {
            DIRECT
        };
        let response = self.request(token, api_key, "GET", path, None, None).await;
        assert_eq!(response.0, StatusCode::OK, "valid credential: {response:?}");
        assert_eq!(response.1["id"], self.owner);
    }

    async fn assert_no_deletion_effect(&self) {
        let owner = self
            .state
            .db
            .collection::<Document>(USERS)
            .find_one(doc! {"_id":&self.owner})
            .await
            .expect("read surviving owner")
            .expect("denial must preserve owner");
        assert_eq!(owner.get_bool("is_active"), Ok(true));
        assert!(matches!(
            owner.get("deletion_requested_at"),
            None | Some(Bson::Null)
        ));
        let machine = self.machine_document().await;
        assert!(matches!(
            machine.get("deletion_basis"),
            None | Some(Bson::Null)
        ));
        assert_eq!(machine.get_str("resource_state"), Ok("paused"));
        for collection in [RECEIPTS, OPERATIONS, OUTBOX] {
            assert_eq!(
                self.state
                    .db
                    .collection::<Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .expect("count effects"),
                0,
                "denied input must not reserve or mutate {collection}"
            );
        }
    }

    async fn machine_document(&self) -> Document {
        self.state
            .db
            .collection::<Document>(MACHINES)
            .find_one(doc! {"_id":&self.machine})
            .await
            .expect("read retained machine")
            .expect("retained machine exists")
    }

    async fn third_party_tokens(&self) -> [(String, bool); 2] {
        let client_id = "account-delete-untrusted-client";
        let now = Utc::now();
        self.state
            .db
            .collection::<crate::models::oauth_client::OauthClient>(
                crate::models::oauth_client::COLLECTION_NAME,
            )
            .insert_one(crate::models::oauth_client::OauthClient {
                id: client_id.into(),
                client_name: "Account deletion authority fixture".into(),
                client_secret_hash: String::new(),
                redirect_uris: vec![],
                allowed_scopes: "account:read".into(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code".into(),
                client_type: "public".into(),
                is_active: true,
                delegation_scopes: "account:read".into(),
                default_service_catalog_slugs: vec![],
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let consent = crate::services::consent_service::grant_consent(
            &self.state.db,
            &self.owner,
            client_id,
            "account:read",
        )
        .await
        .unwrap();
        let reference =
            crate::services::consent_service::ensure_issuance_reference(&self.state.db, &consent)
                .await
                .unwrap();
        let oauth = crate::crypto::jwt::generate_oauth_access_token_with_consent(
            &self.state.jwt_keys,
            &self.state.config,
            &Uuid::parse_str(&self.owner).unwrap(),
            "account:read",
            None,
            None,
            None,
            None,
            None,
            client_id,
            Some(&reference),
        )
        .expect("sign consent-bound third-party OAuth token");
        let key = key_service::create_api_key(
            &self.state.db,
            &self.owner,
            "Account deletion denied fixture",
            // API keys use the public key scope registry, not OAuth delegation
            // scopes. Include write so the deletion denial proves human-only
            // authority rather than a generic missing-write-scope restriction.
            "read write",
            None,
            None,
            None,
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
        .expect("create active API key");
        [(oauth, false), (key.full_key, true)]
    }

    async fn assert_durable_owner_basis_and_idempotent_cleanup(&self, started: bson::DateTime) {
        assert!(
            self.state
                .db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&self.owner})
                .await
                .expect("read fenced owner")
                .is_some_and(|owner| owner.get_bool("is_active") == Ok(false))
        );
        let job = crate::services::personal_erasure_service::status(&self.state.db, &self.owner)
            .await
            .unwrap()
            .unwrap();
        assert!(job.completed_at.is_none());
        assert_ne!(
            job.status,
            crate::models::personal_erasure::ErasureStatus::Complete
        );
        let replay = crate::services::personal_erasure_service::accept(
            &self.state.db,
            &self.owner,
            &self.owner,
            Some(POLICY),
        )
        .await
        .unwrap();
        assert_eq!(replay.id, job.id);
        let before = self.machine_document().await;
        let basis = before
            .get_document("deletion_basis")
            .expect("durable consent basis");
        assert_eq!(basis.get_str("owner_id"), Ok(self.owner.as_str()));
        assert_eq!(basis.get_str("source"), Ok("account_owner"));
        assert_eq!(basis.get_str("policy_version"), Ok(POLICY));
        let accepted = basis
            .get_datetime("accepted_at")
            .expect("BSON consent timestamp");
        assert!(*accepted >= started && *accepted <= bson::DateTime::now());
        let basis = basis.clone();
        let pvc = before
            .get_str("pvc_uid")
            .expect("retained PVC UID")
            .to_owned();
        assert_eq!(
            Box::pin(lifecycle::suspend_ineligible(&self.state.db))
                .await
                .expect("owner deletion recovery sweep"),
            1
        );
        assert_eq!(
            Box::pin(lifecycle::suspend_ineligible(&self.state.db))
                .await
                .expect("replayed recovery sweep"),
            0
        );
        // Exercise the same durable worker used in production. Allocated
        // runtime/volume evidence is still absent: completion must stay pending.
        for _ in 0..24 {
            self.state
                .db
                .collection::<Document>(crate::models::personal_erasure::COLLECTION_NAME)
                .update_one(
                    doc! {"user_id":&self.owner},
                    vec![doc! {"$set":{"retry_at":"$$NOW"}}],
                )
                .await
                .unwrap();
            crate::services::personal_erasure_service::process_one(&self.state)
                .await
                .unwrap();
            let current =
                crate::services::personal_erasure_service::status(&self.state.db, &self.owner)
                    .await
                    .unwrap()
                    .unwrap();
            if current
                .domains
                .iter()
                .any(|domain| domain.domain == "machine" && domain.reason.is_some())
            {
                break;
            }
        }
        let pending =
            crate::services::personal_erasure_service::status(&self.state.db, &self.owner)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(pending.id, job.id);
        assert!(pending.completed_at.is_none());
        assert!(
            pending
                .domains
                .iter()
                .any(|domain| domain.domain == "machine"
                    && domain.status != crate::models::personal_erasure::DomainStatus::Complete)
        );
        // The account row is removed before Machine cleanup completes; the
        // durable owner barrier still refuses writes and the Machine record
        // keeps its own deletion basis below.
        assert!(
            self.state
                .db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&self.owner})
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            crate::services::erasure_admission::begin_write(&self.state.db, &self.owner).await,
            Err(crate::errors::AppError::Conflict(_))
        ));
        let after = self.machine_document().await;
        assert_eq!(after.get_document("deletion_basis"), Ok(&basis));
        assert_eq!(after.get_str("resource_state"), Ok("deleting"));
        assert_eq!(after.get_str("authorization_state"), Ok("revoked"));
        assert_eq!(after.get_bool("writer_admitted"), Ok(false));
        assert_eq!(after.get_str("pvc_uid"), Ok(pvc.as_str()));
        assert_eq!(
            self.state
                .db
                .collection::<Document>(OPERATIONS)
                .count_documents(doc! {"machine_id":&self.machine,"action":"delete"})
                .await
                .expect("cleanup operation count"),
            1
        );
        assert_eq!(
            self.state
                .db
                .collection::<Document>(OUTBOX)
                .count_documents(doc! {"machine_id":&self.machine,"kind":"delete_sandbox"})
                .await
                .expect("cleanup outbox count"),
            1
        );
        assert!(
            lifecycle::finalize_unallocated_delete(&self.state.db, &self.machine)
                .await
                .expect("cleanup still requires physical evidence")
                .is_none()
        );
    }
}

#[tokio::test]
async fn direct_account_delete_requires_current_disclosure_before_any_effect() {
    let f = Fixture::new("direct_account_policy_denial").await;
    f.assert_authenticated_read(&f.human_token, false).await;
    for policy in [
        None,
        Some("machine-data"),
        Some("machine-data-v0"),
        Some("machine-data-v1"),
    ] {
        let response = f
            .request(&f.human_token, false, "DELETE", DIRECT, policy, None)
            .await;
        assert!(response.0.is_client_error(), "policy denial: {response:?}");
        f.assert_no_deletion_effect().await;
    }
    f.state.db.drop().await.expect("drop fixture");
}

#[tokio::test]
async fn assistant_account_delete_requires_current_disclosure_before_receipt_reservation() {
    let f = Fixture::new("assistant_account_policy_denial").await;
    f.assert_authenticated_read(&f.human_token, false).await;
    for (id, policy) in [
        ("missing-policy", None),
        ("old-policy", Some("machine-data-v0")),
        ("previous-policy", Some("machine-data")),
        ("obsolete-development-policy", Some("machine-data-v1")),
    ] {
        let response = f
            .request(
                &f.human_token,
                false,
                "POST",
                ASSISTANT,
                None,
                Some(f.assistant_body(id, policy)),
            )
            .await;
        assert!(response.0.is_client_error(), "policy denial: {response:?}");
        f.assert_no_deletion_effect().await;
    }
    f.state.db.drop().await.expect("drop fixture");
}

#[tokio::test]
async fn direct_account_delete_rejects_valid_oauth_and_api_key_even_with_current_policy() {
    let f = Fixture::new("direct_account_human_only").await;
    for (token, api_key) in f.third_party_tokens().await {
        f.assert_authenticated_read(&token, api_key).await;
        let response = f
            .request(&token, api_key, "DELETE", DIRECT, Some(POLICY), None)
            .await;
        assert_eq!(
            response.0,
            StatusCode::FORBIDDEN,
            "human-only denial: {response:?}"
        );
        f.assert_no_deletion_effect().await;
    }
    f.state.db.drop().await.expect("drop fixture");
}

#[tokio::test]
async fn assistant_account_delete_rejects_valid_oauth_and_api_key_before_receipt_reservation() {
    let f = Fixture::new("assistant_account_human_only").await;
    for (token, api_key) in f.third_party_tokens().await {
        f.assert_authenticated_read(&token, api_key).await;
        let response = f
            .request(
                &token,
                api_key,
                "POST",
                ASSISTANT,
                None,
                Some(f.assistant_body("third-party-delete", Some(POLICY))),
            )
            .await;
        assert_eq!(
            response.0,
            StatusCode::FORBIDDEN,
            "human-only denial: {response:?}"
        );
        f.assert_no_deletion_effect().await;
    }
    f.state.db.drop().await.expect("drop fixture");
}

#[tokio::test]
async fn direct_owner_account_delete_retains_policy_basis_while_physical_cleanup_is_pending_and_replayed()
 {
    let f = Fixture::new("direct_account_durable_basis").await;
    let started = bson::DateTime::now();
    let response = f
        .request(&f.human_token, false, "DELETE", DIRECT, Some(POLICY), None)
        .await;
    assert_eq!(response.0, StatusCode::OK, "owner delete: {response:?}");
    assert_eq!(response.1["status"], "accepted");
    assert!(response.1["completed_at"].is_null());
    f.assert_durable_owner_basis_and_idempotent_cleanup(started)
        .await;
    f.state.db.drop().await.expect("drop fixture");
}

#[tokio::test]
async fn assistant_owner_account_delete_retains_policy_basis_while_physical_cleanup_is_pending_and_replayed()
 {
    let f = Fixture::new("assistant_account_durable_basis").await;
    let started = bson::DateTime::now();
    let response = f
        .request(
            &f.human_token,
            false,
            "POST",
            ASSISTANT,
            None,
            Some(f.assistant_body("owner-confirmed-delete", Some(POLICY))),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK, "owner delete: {response:?}");
    assert_eq!(response.1["erasure"]["status"], "accepted");
    assert!(response.1["erasure"]["completed_at"].is_null());
    let receipt = f
        .state
        .db
        .collection::<Document>(RECEIPTS)
        .find_one(doc! {"user_id":&f.owner,"action_request_id":"owner-confirmed-delete"})
        .await
        .expect("read completed action receipt")
        .expect("receipt exists");
    assert_eq!(receipt.get_str("status"), Ok("completed"));
    f.assert_durable_owner_basis_and_idempotent_cleanup(started)
        .await;
    f.state.db.drop().await.expect("drop fixture");
}
