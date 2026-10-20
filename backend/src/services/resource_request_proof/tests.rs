use super::*;
use crate::{
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        downstream_service::test_helpers::dummy_service,
        user::{COLLECTION_NAME as USERS, User, UserType},
        user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint},
    },
    services::{assistant_agent_credential_service::ASSISTANT_PLATFORM, key_service},
    test_utils::{
        connect_test_database, test_app_config, test_app_state_with_config, test_user,
        test_user_endpoint, test_user_service,
    },
};
use mongodb::bson::{Document, from_document};

fn request(method: &str, path: &str) -> reqwest::Request {
    reqwest::Client::new()
        .request(
            method.parse().unwrap(),
            format!("https://cma.invalid{path}"),
        )
        .build()
        .unwrap()
}

#[test]
fn descriptor_binds_exact_encoded_path_query_and_empty_body() {
    let first =
        ReadDescriptor::from_request(&request("GET", "/api/v1/agents?cursor=a%2Fb&limit=1"))
            .unwrap();
    assert_eq!(first.path_and_query, "/api/v1/agents?cursor=a%2Fb&limit=1");
    for path in [
        "/api/v1/agents?cursor=a/b&limit=1",
        "/api/v1/agents?limit=1&cursor=a%2Fb",
        "/api/v1/agents/",
    ] {
        assert_ne!(
            first,
            ReadDescriptor::from_request(&request("GET", path)).unwrap()
        );
    }
    assert!(ReadDescriptor::from_request(&request("HEAD", "/api/v1/agents")).is_ok());
    let form: IntrospectionInput = serde_urlencoded::from_str(
        &serde_urlencoded::to_string([
            ("request_method", "GET"),
            ("request_path_and_query", first.path_and_query.as_str()),
            ("request_body_sha256", descriptor::EMPTY_BODY_SHA256),
        ])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(form.descriptor().unwrap(), first);
}

#[test]
fn descriptor_rejects_mutations_bodies_overrides_and_ambiguous_headers() {
    for method in ["POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
        assert!(ReadDescriptor::from_request(&request(method, "/api/v1/agents")).is_err());
    }
    for (header, value) in [
        ("idempotency-key", "read"),
        ("x-http-method-override", "DELETE"),
        ("content-encoding", "gzip"),
    ] {
        let mut changed = request("GET", "/api/v1/agents");
        changed.headers_mut().insert(
            axum::http::HeaderName::from_bytes(header.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        assert!(ReadDescriptor::from_request(&changed).is_err());
    }
    let mut changed = request("GET", "/api/v1/agents");
    *changed.body_mut() = Some("payload".into());
    assert!(ReadDescriptor::from_request(&changed).is_err());
    let mut changed = request("GET", "/api/v1/agents");
    changed
        .headers_mut()
        .append("content-type", "application/json".parse().unwrap());
    changed
        .headers_mut()
        .append("content-type", "text/plain".parse().unwrap());
    assert!(ReadDescriptor::from_request(&changed).is_err());
    assert!(
        ReadDescriptor::from_request(&request("GET", "/api/v1/agents?_method=DELETE")).is_err()
    );
}

struct Fixture {
    state: AppState,
    subject: String,
    client: OauthClient,
    catalog: String,
    connection: String,
    key: key_service::CreatedApiKey,
    conversation: String,
}

impl Fixture {
    async fn new() -> Self {
        let db = connect_test_database("resource_read_proof").await.unwrap();
        let subject = Uuid::new_v4().to_string();
        let now = mongodb::bson::DateTime::now();
        db.collection::<User>(USERS)
            .insert_one(test_user(&subject, UserType::Person))
            .await
            .unwrap();
        let client: OauthClient = from_document(doc! {
            "_id": Uuid::new_v4().to_string(), "client_name":"CMA",
            "client_secret_hash":crate::crypto::token::hash_token("fixture-secret"),
            "redirect_uris":[], "allowed_scopes":"openid", "grant_types":"authorization_code",
            "client_type":"confidential", "is_active":true, "delegation_scopes":"",
            "created_by":&subject, "created_at":now, "updated_at":now,
        })
        .unwrap();
        db.collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
            .insert_one(&client)
            .await
            .unwrap();
        let mut catalog = dummy_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.base_url = "https://cma.invalid".into();
        catalog.identity_propagation_mode = "jwt".into();
        catalog.introspection_client_ids = Some(vec![client.id.clone()]);
        db.collection::<DownstreamService>(SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();
        let mut operation = crate::test_utils::test_service_endpoint(
            &catalog.id,
            "list_agents",
            "GET",
            "/api/v1/agents",
        );
        operation.parameters = Some(serde_json::json!([
            {"name":"limit","in":"query","schema":{"type":"integer"}},
        ]));
        db.collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_one(operation)
        .await
        .unwrap();
        let connection = Uuid::new_v4().to_string();
        let endpoint = Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint,
                &subject,
                "CMA",
                &catalog.base_url,
                None,
                Some(&catalog.id),
            ))
            .await
            .unwrap();
        let mut service = test_user_service(
            &connection,
            &subject,
            "cma",
            &endpoint,
            Some(&catalog.id),
            None,
        );
        service.source = Some("auto_provision".into());
        service.identity_propagation_mode = "jwt".into();
        db.collection::<UserService>(CONNECTIONS)
            .insert_one(service)
            .await
            .unwrap();
        let key = key_service::create_api_key(
            &db,
            &subject,
            "NyxBot",
            "proxy",
            None,
            None,
            Some(std::slice::from_ref(&connection)),
            None,
            Some(false),
            Some(true),
            Some(false),
            None,
            None,
            Some(ASSISTANT_PLATFORM),
            None,
        )
        .await
        .unwrap();
        let agent = crate::services::assistant_team_service::ensure_nyxbot(&db, &subject)
            .await
            .unwrap();
        let conversation = format!("nyxa-{}", Uuid::new_v4().simple());
        let row: AssistantConversation = from_document(doc! {
            "_id":&conversation,"user_id":&subject,"agent_id":&agent.id,"title":"read proof",
            "model":&agent.model,"credential_api_key_id":&key.id,"message_count":0_i64,
            "created_at":now,"updated_at":now,
            "active_turn":{"turn_id":Uuid::new_v4().to_string(),"started_at":now,"heartbeat_at":now},
        }).unwrap();
        db.collection::<AssistantConversation>(CONVERSATIONS)
            .insert_one(row)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
            .insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&subject,"api_key_id":&key.id,"conversation_id":&conversation})
            .await.unwrap();
        let mut config = test_app_config();
        config.resource_request_proof_enabled = true;
        let state = test_app_state_with_config(db, config);
        Self {
            state,
            subject,
            client,
            catalog: catalog.id,
            connection,
            key,
            conversation,
        }
    }

    async fn issuer(&self) -> ReadProofIssuer<'_> {
        self.issuer_for(None).await
    }

    async fn issuer_for(&self, credential_id: Option<&str>) -> ReadProofIssuer<'_> {
        let target = self.target().await;
        prepare_admitted(
            &self.state.db,
            &self.state.jwt_keys,
            &self.state.config,
            &self.subject,
            Some(&self.key.id),
            credential_id,
            Some(&self.connection),
            &target,
            &self.catalog,
            "GET",
            "/api/v1/agents",
            Some("limit=1"),
            true,
        )
        .await
        .unwrap()
        .unwrap()
    }

    async fn target(&self) -> super::super::proxy_service::ProxyTarget {
        let service = self
            .state
            .db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&self.catalog})
            .await
            .unwrap()
            .unwrap();
        super::super::proxy_service::ProxyTarget {
            workspace_destinations_pending: false,
            target_id: None,
            base_url: service.base_url.clone(),
            auth_method: "none".into(),
            auth_key_name: String::new(),
            credential: Default::default(),
            service,
            catalog_default_headers: Vec::new(),
            user_service_default_headers: Vec::new(),
            ws_frame_injections: Vec::new(),
            connection_id: None,
        }
    }

    async fn signed(&self) -> (String, ReadDescriptor) {
        let mut outbound = request("GET", "/api/v1/agents?limit=1");
        outbound
            .headers_mut()
            .append(HEADER, "caller-spoof".parse().unwrap());
        outbound
            .headers_mut()
            .append(HEADER, "user-default-spoof".parse().unwrap());
        let issuer = self.issuer().await;
        issuer.attach(&mut outbound).await.unwrap();
        assert_eq!(outbound.headers().get_all(HEADER).iter().count(), 1);
        assert!(outbound.headers()[HEADER].is_sensitive());
        assert!(!format!("{outbound:?}").contains(outbound.headers()[HEADER].to_str().unwrap()));
        (
            outbound.headers()[HEADER].to_str().unwrap().into(),
            ReadDescriptor::from_request(&outbound).unwrap(),
        )
    }
}

#[tokio::test]
async fn final_egress_rejects_destination_method_and_query_changes() {
    let f = Fixture::new().await;
    let issuer = f.issuer().await;
    for (method, path) in [
        ("GET", "/api/v1/agents?limit=2"),
        ("GET", "/api/v1/sandboxes?limit=1"),
        ("HEAD", "/api/v1/agents?limit=1"),
    ] {
        assert!(issuer.attach(&mut request(method, path)).await.is_err());
    }
    let mut outbound = request("GET", "/api/v1/agents?limit=1");
    outbound.url_mut().set_host(Some("other.invalid")).unwrap();
    assert!(issuer.attach(&mut outbound).await.is_err());
}

#[tokio::test]
async fn expired_forged_identity_and_future_proofs_are_inactive() {
    let f = Fixture::new().await;
    let (token, descriptor) = f.signed().await;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&f.catalog]);
    for (iat, exp) in [
        (Utc::now().timestamp() - 120, Utc::now().timestamp() - 60),
        (Utc::now().timestamp() + 120, Utc::now().timestamp() + 180),
        (Utc::now().timestamp(), Utc::now().timestamp() + 3600),
    ] {
        let mut claims = decode::<Claims>(&token, &f.state.jwt_keys.decoding, &validation)
            .unwrap()
            .claims;
        claims.identity.iat = iat;
        claims.identity.exp = exp;
        let changed = encode(
            &decode_header(&token).unwrap(),
            &claims,
            &f.state.jwt_keys.encoding,
        )
        .unwrap();
        assert!(
            introspect(
                &f.state,
                &f.client,
                &f.catalog,
                &changed,
                descriptor.clone()
            )
            .await
            .is_err()
        );
    }
    let claims = decode::<Claims>(&token, &f.state.jwt_keys.decoding, &validation)
        .unwrap()
        .claims;
    let mut header = decode_header(&token).unwrap();
    header.typ = Some("JWT".into());
    let ordinary = encode(&header, &claims.identity, &f.state.jwt_keys.encoding).unwrap();
    assert!(
        introspect(
            &f.state,
            &f.client,
            &f.catalog,
            &ordinary,
            descriptor.clone()
        )
        .await
        .is_err()
    );
    let mut segments: Vec<_> = token.split('.').map(str::to_owned).collect();
    let first = if segments[2].starts_with('a') {
        "b"
    } else {
        "a"
    };
    segments[2].replace_range(..1, first);
    assert!(
        introspect(
            &f.state,
            &f.client,
            &f.catalog,
            &segments.join("."),
            descriptor
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn verifier_overlap_preserves_read_proofs_until_key_retirement() {
    let (old, current) = crate::crypto::jwt::test_rotation_keys();
    let mut f = Fixture::new().await;
    f.state.jwt_keys = old;
    let (token, descriptor) = f.signed().await;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&f.catalog]);
    let claims = decode::<Claims>(&token, &f.state.jwt_keys.decoding, &validation)
        .unwrap()
        .claims;
    let mut rotated = f.state.clone();
    rotated.jwt_keys = current;
    assert!(
        introspect(&rotated, &f.client, &f.catalog, &token, descriptor.clone())
            .await
            .is_ok()
    );
    for kid in [None, Some("unregistered-signing-key".into())] {
        let mut header = decode_header(&token).unwrap();
        header.kid = kid;
        let changed = encode(&header, &claims, &f.state.jwt_keys.encoding).unwrap();
        assert!(
            introspect(
                &rotated,
                &f.client,
                &f.catalog,
                &changed,
                descriptor.clone()
            )
            .await
            .is_err()
        );
    }
    f.state.jwt_keys = rotated.jwt_keys.clone();
    let (active_token, active_descriptor) = f.signed().await;
    assert!(
        introspect(
            &rotated,
            &f.client,
            &f.catalog,
            &active_token,
            active_descriptor.clone()
        )
        .await
        .is_ok()
    );
    rotated.jwt_keys.retained.clear();
    assert!(
        introspect(&rotated, &f.client, &f.catalog, &token, descriptor)
            .await
            .is_err()
    );
    assert!(
        introspect(
            &rotated,
            &f.client,
            &f.catalog,
            &active_token,
            active_descriptor
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn current_turn_and_exact_child_credential_must_still_exist() {
    use crate::models::api_key_credential::COLLECTION_NAME as CHILDREN;
    let f = Fixture::new().await;
    let child = Uuid::new_v4().to_string();
    f.state.db.collection::<Document>(CHILDREN).insert_one(doc! {
        "_id":&child,"api_key_id":&f.key.id,"user_id":&f.subject,"secret_hash":"child-secret-hash",
        "secret_prefix":"nyxid_ag_fixture","label":"fixture","login_request_id":"fixture",
        "is_active":true,"created_at":mongodb::bson::DateTime::now(),
    }).await.unwrap();
    let issuer = f.issuer_for(Some(&child)).await;
    let mut outbound = request("GET", "/api/v1/agents?limit=1");
    issuer.attach(&mut outbound).await.unwrap();
    let token = outbound.headers()[HEADER].to_str().unwrap();
    let descriptor = ReadDescriptor::from_request(&outbound).unwrap();
    assert!(
        introspect(&f.state, &f.client, &f.catalog, token, descriptor.clone())
            .await
            .is_ok()
    );
    f.state
        .db
        .collection::<Document>(CHILDREN)
        .update_one(doc! {"_id":&child}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        introspect(&f.state, &f.client, &f.catalog, token, descriptor.clone())
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<Document>(CHILDREN)
        .update_one(doc! {"_id":&child}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.conversation},
            doc! {"$set":{"active_turn.turn_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(
        introspect(&f.state, &f.client, &f.catalog, token, descriptor)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn authenticated_introspection_rejects_missing_secret_and_disabled_feature() {
    use axum::{body::to_bytes, extract::State};
    use axum_extra::extract::Form;
    let f = Fixture::new().await;
    let (token, descriptor) = f.signed().await;
    for (secret, expected) in [("fixture-secret", true), ("wrong-secret", false)] {
        let encoded = serde_urlencoded::to_string([
            ("token", token.as_str()),
            ("token_type_hint", "resource_request"),
            ("client_id", f.client.id.as_str()),
            ("client_secret", secret),
            ("service_id", f.catalog.as_str()),
            ("request_method", "GET"),
            ("request_path_and_query", descriptor.path_and_query.as_str()),
            ("request_body_sha256", descriptor::EMPTY_BODY_SHA256),
        ])
        .unwrap();
        let body = serde_urlencoded::from_str(&encoded).unwrap();
        let response = crate::handlers::oauth::introspect(State(f.state.clone()), Form(body)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
        assert_eq!(value["active"], expected);
        if expected {
            assert_eq!(value["independent_commands"], false);
        }
    }
    let mut disabled = f.state.clone();
    disabled.config.resource_request_proof_enabled = false;
    assert!(
        introspect(&disabled, &f.client, &f.catalog, &token, descriptor)
            .await
            .is_err()
    );
}

mod egress;
#[cfg(unix)]
mod joined;

#[tokio::test]
async fn exact_live_owner_read_is_signed_and_cannot_be_replayed_as_another_request() {
    let f = Fixture::new().await;
    let (token, descriptor) = f.signed().await;
    let evidence = introspect(&f.state, &f.client, &f.catalog, &token, descriptor.clone())
        .await
        .unwrap();
    assert_eq!(evidence.sub, f.subject);
    assert_eq!(evidence.aud, f.catalog);
    assert!(!evidence.independent_commands);
    assert_eq!(evidence.exp - evidence.iat, 60);
    for (path, content_type) in [
        ("/api/v1/agents?limit=2", None),
        ("/api/v1/agents/?limit=1", None),
        ("/api/v1/agents?limit=1", Some("application/json")),
    ] {
        let mut changed = descriptor.clone();
        changed.path_and_query = path.into();
        changed.content_type = content_type.map(str::to_owned);
        assert!(
            introspect(&f.state, &f.client, &f.catalog, &token, changed)
                .await
                .is_err()
        );
    }
    let mut changed = descriptor.clone();
    changed.method = "POST".into();
    assert!(
        introspect(&f.state, &f.client, &f.catalog, &token, changed)
            .await
            .is_err()
    );
    assert!(
        introspect(
            &f.state,
            &f.client,
            &Uuid::new_v4().to_string(),
            &token,
            descriptor.clone()
        )
        .await
        .is_err()
    );
    let mut client = f.client.clone();
    client.id = Uuid::new_v4().to_string();
    assert!(
        introspect(&f.state, &client, &f.catalog, &token, descriptor)
            .await
            .is_err()
    );
    // The proof intentionally lacks normal access-token scope/type claims.
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&f.catalog]);
    assert!(
        decode::<crate::crypto::jwt::Claims>(&token, &f.state.jwt_keys.decoding, &validation)
            .is_err()
    );
}

#[tokio::test]
async fn stopped_guest_revoked_and_replaced_credentials_invalidate_issued_reads() {
    let f = Fixture::new().await;
    let (token, descriptor) = f.signed().await;
    for patch in [
        doc! {"active_turn.stop_requested":true},
        doc! {"guest_turn":true},
    ] {
        f.state
            .db
            .collection::<Document>(CONVERSATIONS)
            .update_one(doc! {"_id":&f.conversation}, doc! {"$set":patch})
            .await
            .unwrap();
        assert!(
            introspect(&f.state, &f.client, &f.catalog, &token, descriptor.clone())
                .await
                .is_err()
        );
        f.state
            .db
            .collection::<Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&f.conversation},
                doc! {"$set":{"active_turn.stop_requested":false,"guest_turn":false}},
            )
            .await
            .unwrap();
    }
    let key = f
        .state
        .db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&f.key.id})
        .await
        .unwrap()
        .unwrap();
    for patch in [
        doc! {"is_active":false},
        doc! {"key_hash":"replaced"},
        doc! {"allowed_service_ids":[],"allow_auto_connected_services":false},
    ] {
        f.state
            .db
            .collection::<ApiKey>(KEYS)
            .update_one(doc! {"_id":&f.key.id}, doc! {"$set":patch})
            .await
            .unwrap();
        assert!(
            introspect(&f.state, &f.client, &f.catalog, &token, descriptor.clone())
                .await
                .is_err()
        );
        f.state
            .db
            .collection::<ApiKey>(KEYS)
            .replace_one(doc! {"_id":&f.key.id}, &key)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn changed_connection_and_new_approval_requirements_refuse_reads() {
    let f = Fixture::new().await;
    let (token, descriptor) = f.signed().await;
    f.state
        .db
        .collection::<Document>(CONNECTIONS)
        .update_one(
            doc! {"_id":&f.connection},
            doc! {"$set":{"node_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(
        introspect(&f.state, &f.client, &f.catalog, &token, descriptor.clone())
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<Document>(CONNECTIONS)
        .update_one(doc! {"_id":&f.connection}, doc! {"$set":{"node_id":null}})
        .await
        .unwrap();
    let now = mongodb::bson::DateTime::now();
    f.state
        .db
        .collection::<Document>(crate::models::service_approval_config::COLLECTION_NAME)
        .insert_one(
            doc! {"_id":Uuid::new_v4().to_string(),"user_id":&f.subject,"service_id":&f.catalog,
            "service_name":"CMA","approval_required":true,"created_at":now,"updated_at":now},
        )
        .await
        .unwrap();
    assert!(
        introspect(&f.state, &f.client, &f.catalog, &token, descriptor)
            .await
            .is_err()
    );
}
