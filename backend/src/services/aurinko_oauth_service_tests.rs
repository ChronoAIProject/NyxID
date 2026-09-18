use super::*;
use crate::models::user::{User, UserType};
use crate::services::{
    channel_credentials, channel_retry_ingress, user_api_key_service, user_service_service,
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};

struct Fixture {
    db: Database,
    keys: EncryptionKeys,
    owner: String,
    actor: String,
    session: String,
    key: UserApiKey,
    service: UserService,
}
impl Fixture {
    async fn new() -> Self {
        let db = crate::test_utils::connect_test_database("aurinko_managed")
            .await
            .expect("real MongoDB replica set required");
        super::super::coordination_service::ensure_indexes(&db)
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let owner = Uuid::new_v4().to_string();
        db.collection::<User>("users")
            .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let session = Uuid::new_v4().to_string();
        db.collection::<Document>("sessions").insert_one(doc! {"_id":&session,"user_id":&owner,"revoked":false,"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(1))}).await.unwrap();
        let now = bson::DateTime::now();
        let encrypted_id = keys.encrypt(b"platform-client").await.unwrap();
        let encrypted_secret = keys.encrypt(b"platform-secret").await.unwrap();
        let p:ProviderConfig=bson::from_document(doc! {"_id":"aurinko-provider","slug":"aurinko","name":"Aurinko","provider_type":"api_key","is_active":true,"created_by":"system","created_at":now,"updated_at":now,
            "client_id_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted_id},
            "client_secret_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted_secret}}).unwrap();
        db.collection::<ProviderConfig>(PROVIDERS)
            .insert_one(p)
            .await
            .unwrap();
        let key:UserApiKey=bson::from_document(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&owner,"label":"Mailbox","credential_type":"oauth2","credential_source":"platform","provider_config_id":"aurinko-provider","connection_id":Uuid::new_v4().to_string(),"status":"pending_auth","credential_epoch":1_i64,"created_at":now,"updated_at":now}).unwrap();
        db.collection::<UserApiKey>(KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        let endpoint = crate::test_utils::test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &owner,
            "Mailbox",
            ORIGIN,
            None,
            None,
        );
        let mut service = crate::test_utils::test_user_service(
            &Uuid::new_v4().to_string(),
            &owner,
            "mailbox",
            &endpoint.id,
            None,
            None,
        );
        service.api_key_id = Some(key.id.clone());
        service.auth_method = "bearer".into();
        service.auth_key_name = "Authorization".into();
        db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        Self {
            db,
            keys,
            actor: owner.clone(),
            owner,
            session,
            key,
            service,
        }
    }
    async fn start(&self, provider: MailProvider, link: Option<&str>) -> StartResult {
        start(
            &self.db,
            &self.keys,
            "https://nyxid.test",
            &self.actor,
            &self.session,
            &self.owner,
            "Mailbox",
            provider,
            Some(&self.key.id),
            link,
            None,
        )
        .await
        .unwrap()
    }
    async fn finish(&self, attempt: &StartResult, server: &MockServer) -> AppResult<()> {
        complete_with_client(
            &self.db,
            &self.keys,
            CallbackInput {
                state_id: &attempt.state_id,
                browser_nonce: &attempt.browser_nonce,
                actor: &self.actor,
                session_id: &self.session,
                code: Some("code"),
                error: None,
                status: None,
            },
            &AurinkoClient {
                origin: Some(server.uri()),
            },
        )
        .await
    }
    async fn key(&self) -> UserApiKey {
        self.db
            .collection::<UserApiKey>(KEYS)
            .find_one(doc! {"_id":&self.key.id})
            .await
            .unwrap()
            .unwrap()
    }
    async fn activate(&self) {
        let token = self.keys.encrypt(b"mailbox-bearer").await.unwrap();
        self.db.collection::<Document>(KEYS).update_one(doc! {"_id":&self.key.id},doc! {"$set":{"status":"active","access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:token},"token_scopes":"Mail.Read Mail.Send Mail.Drafts","aurinko_account":{"account_id":"42","service_type":"IMAP","mailbox_address":"mail@example.com","application_id_hash":"app"}}}).await.unwrap();
    }
}
fn account(provider: &str) -> Value {
    json!({"id":42,"serviceType":provider,"tokenStatus":"active","active":true,"authScopes":["Mail.ReadWrite","Mail.Drafts"],"email":"$mailbox@example.com"})
}

#[tokio::test]
async fn proxy_validates_actual_mailbox_destination_snapshot() {
    use crate::models::user_endpoint::UserEndpoint;

    let f = Fixture::new().await;
    f.activate().await;
    let key = f.key().await;
    let endpoint =
        f.db.collection::<UserEndpoint>("user_endpoints")
            .find_one(doc! {"_id": &f.service.endpoint_id})
            .await
            .unwrap()
            .unwrap();
    validate_connection_route_snapshot(&f.db, &f.service, &key, &endpoint)
        .await
        .unwrap();

    // The live database is safe, but a previously loaded routing snapshot may
    // still point elsewhere. Validate that exact snapshot before decryption.
    let mut stale = endpoint.clone();
    stale.url = "https://mailbox-token-recipient.invalid".into();
    validate_connection_route(&f.db, &f.service, &key)
        .await
        .unwrap();
    assert!(
        validate_connection_route_snapshot(&f.db, &f.service, &key, &stale)
            .await
            .is_err()
    );

    let mut foreign_owner = endpoint.clone();
    foreign_owner.user_id = Uuid::new_v4().to_string();
    assert!(
        validate_connection_route_snapshot(&f.db, &f.service, &key, &foreign_owner)
            .await
            .is_err()
    );
    let mut different_endpoint = endpoint.clone();
    different_endpoint.id = Uuid::new_v4().to_string();
    assert!(
        validate_connection_route_snapshot(&f.db, &f.service, &key, &different_endpoint)
            .await
            .is_err()
    );

    // Destination restrictions are specific to the managed protocol.
    let mut manual = key;
    manual.credential_type = "api_key".into();
    manual.credential_source = None;
    manual.aurinko_account = None;
    validate_connection_route_snapshot(&f.db, &f.service, &manual, &stale)
        .await
        .unwrap();
}

#[tokio::test]
async fn mailbox_rebind_and_endpoint_mutations_fail_without_changing_connection() {
    use crate::models::user_endpoint::UserEndpoint;
    use crate::services::user_endpoint_service::{
        self, OpenApiSpecUrlUpdate, RecommendedSkillsUpdate,
    };

    let f = Fixture::new().await;
    f.activate().await;
    let mut manual = f.key().await;
    manual.id = Uuid::new_v4().to_string();
    manual.credential_type = "api_key".into();
    manual.credential_source = None;
    manual.aurinko_account = None;
    f.db.collection::<UserApiKey>(KEYS)
        .insert_one(&manual)
        .await
        .unwrap();
    assert!(
        user_service_service::rebind_user_service_api_key(
            &f.db,
            &f.owner,
            &f.service.slug,
            &manual.id,
        )
        .await
        .is_err()
    );
    assert!(
        user_endpoint_service::update_endpoint(
            &f.db,
            &f.owner,
            &f.service.endpoint_id,
            Some("https://mailbox-token-recipient.invalid"),
            None,
            OpenApiSpecUrlUpdate::Leave,
            RecommendedSkillsUpdate::Leave,
        )
        .await
        .is_err()
    );

    for mutation in [
        doc! {"api_key_id": &manual.id},
        doc! {"endpoint_id": Uuid::new_v4().to_string()},
        doc! {"auth_method": "query"},
        doc! {"auth_key_name": "X-Token"},
        doc! {"node_id": Uuid::new_v4().to_string()},
    ] {
        let mut session = f.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            user_service_service::commit_user_service_mutation(
                &f.db,
                &mut session,
                &f.owner,
                &f.service.id,
                mutation,
            )
            .await
            .is_err()
        );
        session.abort_transaction().await.unwrap();
    }
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        user_endpoint_service::update_endpoint_in_session(
            &f.db,
            &mut session,
            &f.owner,
            &f.service.endpoint_id,
            Some("https://mailbox-token-recipient.invalid"),
            None,
            OpenApiSpecUrlUpdate::Leave,
            RecommendedSkillsUpdate::Leave,
        )
        .await
        .is_err()
    );
    session.abort_transaction().await.unwrap();

    let service = user_service_service::get_user_service(&f.db, &f.owner, &f.service.id)
        .await
        .unwrap();
    assert_eq!(service.api_key_id.as_deref(), Some(f.key.id.as_str()));
    assert_eq!(service.endpoint_id, f.service.endpoint_id);
    assert_eq!(service.auth_method, "bearer");
    assert_eq!(service.auth_key_name, "Authorization");
    assert!(service.node_id.is_none());
    let endpoint =
        f.db.collection::<UserEndpoint>("user_endpoints")
            .find_one(doc! {"_id": &f.service.endpoint_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(endpoint.url, ORIGIN);
}

#[tokio::test]
async fn managed_mailbox_has_one_service_even_when_disabled_or_concurrently_created() {
    async fn attach(f: &Fixture, slug: &str) -> AppResult<UserService> {
        user_service_service::create_user_service(
            &f.db,
            &f.owner,
            &f.actor,
            slug,
            &f.service.endpoint_id,
            Some(&f.key.id),
            "bearer",
            "Authorization",
            None,
            None,
            0,
            "http",
            crate::models::ssh_auth_mode::SshAuthMode::ProxyOnly,
            None,
            None,
            None,
            &user_service_service::IdentityConfig::none(),
            None,
            false,
        )
        .await
    }

    let f = Fixture::new().await;
    f.activate().await;
    assert!(attach(&f, "mailbox-clone").await.is_err());
    f.db.collection::<UserService>(SERVICES)
        .update_one(
            doc! {"_id": &f.service.id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    assert!(attach(&f, "mailbox-clone-disabled").await.is_err());

    let mut clone = f.service.clone();
    clone.id = Uuid::new_v4().to_string();
    clone.slug = "unbound-service".into();
    clone.api_key_id = None;
    f.db.collection::<UserService>(SERVICES)
        .insert_one(&clone)
        .await
        .unwrap();
    assert!(
        user_service_service::link_api_key(&f.db, &f.owner, &clone.id, &f.key.id)
            .await
            .is_err()
    );
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        user_service_service::commit_user_service_mutation(
            &f.db,
            &mut session,
            &f.owner,
            &clone.id,
            doc! {"api_key_id": &f.key.id},
        )
        .await
        .is_err()
    );
    session.abort_transaction().await.unwrap();
    assert!(
        user_service_service::get_user_service(&f.db, &f.owner, &clone.id)
            .await
            .unwrap()
            .api_key_id
            .is_none()
    );

    // Even a malformed preexisting clone cannot revive a disabled connection.
    f.db.collection::<UserService>(SERVICES)
        .update_one(
            doc! {"_id": &clone.id},
            doc! {"$set": {"api_key_id": &f.key.id}},
        )
        .await
        .unwrap();
    assert!(
        require_live_connection(&f.db, &f.owner, &f.key.id, true)
            .await
            .is_err()
    );

    let fresh = Fixture::new().await;
    fresh
        .db
        .collection::<UserService>(SERVICES)
        .delete_one(doc! {"_id": &fresh.service.id})
        .await
        .unwrap();
    let (first, second) = tokio::join!(attach(&fresh, "first"), attach(&fresh, "second"));
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        fresh
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {"api_key_id": &fresh.key.id})
            .await
            .unwrap(),
        1
    );
}
async fn upstream_mock(provider: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/token/code"))
        .and(header(
            "authorization",
            "Basic cGxhdGZvcm0tY2xpZW50OnBsYXRmb3JtLXNlY3JldA==",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"accessToken":"mailbox-bearer","accountId":42})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/account"))
        .and(header("authorization", "Bearer mailbox-bearer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(account(provider)))
        .mount(&server)
        .await;
    server
}

#[test]
fn official_provider_matrix_and_hosted_password_boundary() {
    for provider in ["Google", "Office365", "Zoho", "IMAP", "EWS", "iCloud"] {
        let selected: MailProvider = serde_json::from_value(json!(provider)).unwrap();
        assert_eq!(selected.service_type(), provider);
        let wire = authorization_url(
            "public-client",
            "https://nyxid.test/api/v1/providers/aurinko/mailboxes/callback",
            "1cc_00000000-0000-4000-8000-000000000001",
            provider,
            None,
        )
        .unwrap();
        let url = url::Url::parse(&wire).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.origin().ascii_serialization(), ORIGIN);
        assert_eq!(query["responseType"], "code");
        assert_eq!(query["serviceType"], provider);
        assert_eq!(query["scopes"], "Mail.Read Mail.Send Mail.Drafts");
        assert_eq!(query.len(), 6); // No raw mailbox password, SMTP credential, app secret or bearer.
    }
    assert!(serde_json::from_value::<MailProvider>(json!("POP3")).is_err());
    for bad in [
        "http://nyxid.test",
        "https://evil@nyxid.test",
        "https://nyxid.test/path",
        "https://nyxid.test?redirect=evil",
        "https://nyxid.test/#fragment",
    ] {
        assert!(callback_url(bad).is_err());
    }
}
#[test]
fn account_identity_scope_implications_and_inactive_grants() {
    let (verified, scopes) = verify_account(&account("Zoho"), "42", "Zoho", SCOPES).unwrap();
    assert_eq!(verified.mailbox_address, "$mailbox@example.com");
    assert!(scopes.split_whitespace().any(|s| s == "Mail.Send"));
    for bad in [
        json!("001"),
        json!("+1"),
        json!(0),
        json!(-1),
        json!("9223372036854775808"),
    ] {
        assert!(account_id(&bad).is_none());
    }
    for (field, value) in [
        ("tokenStatus", json!("revoked")),
        ("id", json!(43)),
        ("daemon", json!(true)),
        ("serviceType", json!("Google")),
        ("active", json!(false)),
        ("authScopes", json!(["Mail.Read"])),
    ] {
        let mut row = account("IMAP");
        row[field] = value;
        assert!(
            verify_account(&row, "42", "IMAP", SCOPES).is_err(),
            "{field}"
        );
    }
}
#[tokio::test]
async fn account_ping_is_provider_specific() {
    for provider in ["Google", "Office365", "IMAP", "Zoho", "EWS", "iCloud"] {
        let server = MockServer::start().await;
        let ping = matches!(provider, "Google" | "Office365" | "IMAP");
        let mut mock = Mock::given(method("GET")).and(path("/v1/account"));
        if ping {
            mock = mock.and(query_param("pingProvider", "true"));
        }
        mock.respond_with(ResponseTemplate::new(200).set_body_json(account(provider)))
            .expect(1)
            .mount(&server)
            .await;
        AurinkoClient {
            origin: Some(server.uri()),
        }
        .account("mailbox-bearer", "42", provider, SCOPES)
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        if !ping {
            assert_eq!(requests[0].url.query(), None);
        }
    }
}
#[tokio::test]
async fn callback_commits_one_encrypted_connection_and_literal_mailbox() {
    let f = Fixture::new().await;
    let server = upstream_mock("IMAP").await;
    let attempt = f.start(MailProvider::IMAP, None).await;
    assert!(
        super::super::user_token_service::chat_attempt_nonce_from_state(&attempt.state_id)
            .is_some()
    );
    f.finish(&attempt, &server).await.unwrap();
    let key = f.key().await;
    assert_eq!(key.credential_epoch, 2);
    assert_eq!(key.credential_source.as_deref(), Some("platform"));
    assert_eq!(
        key.aurinko_account.as_ref().unwrap().mailbox_address,
        "$mailbox@example.com"
    );
    assert_eq!(
        f.keys
            .decrypt(key.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap(),
        b"mailbox-bearer"
    );
    assert!(key.refresh_token_encrypted.is_none());
    assert!(key.expires_at.is_none());
    assert!(key.credential_encrypted.is_none());
    assert_eq!(
        f.db.collection::<Document>(KEYS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(f.finish(&attempt, &server).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    let token =
        channel_credentials::connection_token(&f.db, &f.keys, &f.owner, &key.id, "aurinko", SCOPES)
            .await
            .unwrap();
    assert_eq!(&*token, "mailbox-bearer");
    let reconnect = f.start(MailProvider::IMAP, None).await;
    assert!(reconnect.authorization_url.contains("accountId=42"));
    f.finish(&reconnect, &server).await.unwrap();
    assert_eq!(f.key().await.id, key.id);
    assert_eq!(f.key().await.credential_epoch, 3);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() != "DELETE")
    );
}
#[tokio::test]
async fn cancellation_and_replaced_attempts_never_exchange() {
    let f = Fixture::new().await;
    let server = upstream_mock("IMAP").await;
    let old = f.start(MailProvider::IMAP, None).await;
    let replacement = f.start(MailProvider::IMAP, None).await;
    assert!(f.finish(&old, &server).await.is_err());
    cancel(
        &f.db,
        &f.owner,
        &f.session,
        super::super::user_token_service::chat_attempt_nonce_from_state(&replacement.state_id)
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(f.finish(&replacement, &server).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(f.key().await.status, "pending_auth");
}
#[tokio::test]
async fn authority_and_epoch_drift_reject_before_exchange() {
    for drift in ["session", "owner", "epoch", "config"] {
        let f = Fixture::new().await;
        let server = upstream_mock("IMAP").await;
        let attempt = f.start(MailProvider::IMAP, None).await;
        match drift {
            "session" => {
                f.db.collection::<Document>("sessions")
                    .update_one(doc! {"_id":&f.session}, doc! {"$set":{"revoked":true}})
                    .await
                    .unwrap();
            }
            "owner" => {
                f.db.collection::<Document>("users")
                    .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            "epoch" => {
                f.db.collection::<Document>(KEYS)
                    .update_one(
                        doc! {"_id":&f.key.id},
                        vec![doc! {"$set":{"credential_epoch":{"$add":["$credential_epoch",1]}}}],
                    )
                    .await
                    .unwrap();
            }
            _ => {
                f.db.collection::<Document>(PROVIDERS)
                    .update_one(doc! {"slug":"aurinko"}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
        }
        assert!(f.finish(&attempt, &server).await.is_err(), "{drift}");
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "{drift}"
        );
    }
}
#[tokio::test]
async fn wrong_browser_and_generic_callback_cannot_consume_aurinko_state() {
    let f = Fixture::new().await;
    let attempt = f.start(MailProvider::IMAP, None).await;
    assert!(
        bound_connect_link(
            &f.db,
            &attempt.state_id,
            &f.owner,
            &f.session,
            "other-browser"
        )
        .await
        .is_err()
    );
    assert!(
        super::super::user_token_service::peek_oauth_state(&f.db, &attempt.state_id)
            .await
            .is_err()
    );
    assert!(
        !f.db
            .collection::<OAuthState>(STATES)
            .find_one(doc! {"_id":&attempt.state_id})
            .await
            .unwrap()
            .unwrap()
            .consumed
    );
    assert!(
        super::super::user_token_service::poll_device_code(
            &f.db,
            &f.keys,
            &f.owner,
            "aurinko-provider",
            &attempt.state_id
        )
        .await
        .is_err()
    );
    assert_eq!(f.key().await.status, "pending_auth");
}
#[tokio::test]
async fn hosted_link_cancel_fences_callback_and_success_is_atomic() {
    for cancelled in [true, false] {
        let f = Fixture::new().await;
        let server = upstream_mock("Zoho").await;
        let id = Uuid::new_v4().to_string();
        f.db.collection::<Document>("connect_links").insert_one(doc!{"_id":&id,"user_id":&f.owner,"status":"pending","completed_user_service_id":&f.service.id,"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10))}).await.unwrap();
        let attempt = f.start(MailProvider::Zoho, Some(&id)).await;
        if cancelled {
            f.db.collection::<Document>("connect_links")
                .update_one(doc! {"_id":&id}, doc! {"$set":{"status":"cancelled"}})
                .await
                .unwrap();
        }
        let result = f.finish(&attempt, &server).await;
        assert_eq!(result.is_err(), cancelled);
        let link =
            f.db.collection::<Document>("connect_links")
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            link.get_str("status").unwrap(),
            if cancelled { "cancelled" } else { "completed" }
        );
        assert_eq!(
            f.key().await.status,
            if cancelled { "failed" } else { "active" }
        );
    }
}
#[tokio::test]
async fn manual_token_rotates_but_managed_requires_reconnect() {
    let f = Fixture::new().await;
    f.activate().await;
    assert!(
        user_api_key_service::update_api_key(
            &f.db,
            &f.keys,
            &f.owner,
            &f.key.id,
            None,
            Some("raw-replacement")
        )
        .await
        .is_err()
    );
    user_api_key_service::update_api_key(
        &f.db,
        &f.keys,
        &f.owner,
        &f.key.id,
        Some("New label"),
        None,
    )
    .await
    .unwrap();
    f.db.collection::<Document>(KEYS).update_one(doc!{"_id":&f.key.id},doc!{"$set":{"credential_type":"api_key","credential_source":null},"$unset":{"aurinko_account":""}}).await.unwrap();
    user_api_key_service::update_api_key(
        &f.db,
        &f.keys,
        &f.owner,
        &f.key.id,
        None,
        Some("manual-token"),
    )
    .await
    .unwrap();
    let key = f.key().await;
    assert_eq!(
        f.keys
            .decrypt(key.credential_encrypted.as_ref().unwrap())
            .await
            .unwrap(),
        b"manual-token"
    );
    assert_eq!(key.credential_epoch, 2);
}
#[tokio::test]
async fn paused_mailbox_effect_blocks_disable_and_disable_blocks_live_tokens() {
    let f = Fixture::new().await;
    f.activate().await;
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    let effect = channel_retry_ingress::with_connection(&f.db, &f.key.id, async {
        entered.send(()).unwrap();
        resume.await.unwrap();
        Ok(())
    });
    let mutation = async {
        ready.await.unwrap();
        let result = user_service_service::update_user_service(
            &f.db,
            &f.owner,
            &f.owner,
            &f.service.id,
            None,
            None,
            None,
            None,
            Some(false),
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(
            user_service_service::get_user_service(&f.db, &f.owner, &f.service.id)
                .await
                .unwrap()
                .is_active
        );
        release.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(effect, mutation);
    result.unwrap();
    user_service_service::update_user_service(
        &f.db,
        &f.owner,
        &f.owner,
        &f.service.id,
        None,
        None,
        None,
        None,
        Some(false),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        channel_credentials::connection_token(
            &f.db, &f.keys, &f.owner, &f.key.id, "aurinko", SCOPES
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn reconnect_advances_bot_generation_even_when_wall_clock_is_behind() {
    let f = Fixture::new().await;
    let server = upstream_mock("IMAP").await;
    let before = bson::DateTime::from_chrono(Utc::now() + Duration::hours(1));
    let bot = Uuid::new_v4().to_string();
    f.db.collection::<Document>("channel_bots").insert_one(doc! {"_id":&bot,"user_id":&f.owner,"platform":"aurinko","credential_source":"connection","connection_id":&f.key.id,"is_active":true,"updated_at":before}).await.unwrap();
    let attempt = f.start(MailProvider::IMAP, None).await;
    f.finish(&attempt, &server).await.unwrap();
    let row =
        f.db.collection::<Document>("channel_bots")
            .find_one(doc! {"_id":&bot})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        row.get_datetime("updated_at").unwrap().timestamp_millis(),
        before.timestamp_millis() + 1
    );
    let attempt = f.start(MailProvider::IMAP, None).await;
    f.finish(&attempt, &server).await.unwrap();
    let row =
        f.db.collection::<Document>("channel_bots")
            .find_one(doc! {"_id":&bot})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        row.get_datetime("updated_at").unwrap().timestamp_millis(),
        before.timestamp_millis() + 2
    );
}

#[tokio::test]
async fn pending_cleanup_is_local_and_cannot_delete_completed_mailbox() {
    let f = Fixture::new().await;
    let server = upstream_mock("IMAP").await;
    let attempt = f.start(MailProvider::IMAP, None).await;
    f.finish(&attempt, &server).await.unwrap();
    assert!(
        !unified::revoke_key_if_pending(&f.db, &f.owner, &f.owner, &f.service.id)
            .await
            .unwrap()
    );
    assert_eq!(f.key().await.status, "active");
    let reconnect = f.start(MailProvider::IMAP, None).await;
    cancel(
        &f.db,
        &f.owner,
        &f.session,
        super::super::user_token_service::chat_attempt_nonce_from_state(&reconnect.state_id)
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(f.key().await.status, "active");
    assert_eq!(f.key().await.credential_epoch, 2);
    let fresh = Fixture::new().await;
    let attempt = fresh.start(MailProvider::IMAP, None).await;
    cancel(
        &fresh.db,
        &fresh.owner,
        &fresh.session,
        super::super::user_token_service::chat_attempt_nonce_from_state(&attempt.state_id).unwrap(),
    )
    .await
    .unwrap();
    assert!(
        unified::revoke_key_if_pending(&fresh.db, &fresh.owner, &fresh.owner, &fresh.service.id)
            .await
            .unwrap()
    );
    assert!(fresh.finish(&attempt, &server).await.is_err());
    assert!(
        !user_service_service::get_user_service(&fresh.db, &fresh.owner, &fresh.service.id)
            .await
            .unwrap()
            .is_active
    );
}

#[tokio::test]
async fn existing_api_key_provider_provisions_managed_and_manual_connections() {
    let f = Fixture::new().await;
    super::super::provider_service::seed_default_services(&f.db, &f.keys)
        .await
        .unwrap();
    let started = start(
        &f.db,
        &f.keys,
        "https://nyxid.test",
        &f.owner,
        &f.session,
        &f.owner,
        "Managed mailbox",
        MailProvider::IMAP,
        None,
        None,
        Some("enterprise-mail"),
    )
    .await
    .unwrap();
    let managed =
        f.db.collection::<UserApiKey>(KEYS)
            .find_one(doc! {"_id":&started.key_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(managed.status, "pending_auth");
    assert_eq!(managed.credential_type, "oauth2");
    assert_eq!(managed.credential_source.as_deref(), Some("platform"));
    assert!(managed.access_token_encrypted.is_none());
    assert!(managed.user_oauth_client_id_encrypted.is_none());
    let service = user_service_service::get_user_service(&f.db, &f.owner, &started.service_id)
        .await
        .unwrap();
    assert_eq!(service.slug, "enterprise-mail");
    let manual = unified::create_key(
        &f.db,
        &f.keys,
        &f.owner,
        &f.owner,
        Some("api-aurinko"),
        None,
        "manual-mailbox-token",
        "Manual mailbox",
        None,
        None,
        None,
        None,
        None,
        None,
        unified::OpenApiSpecUrlInput::Inherit,
        None,
        false,
        unified::OauthClientCredentialsInput::None,
        false,
    )
    .await
    .unwrap();
    let manual = manual.api_key.unwrap();
    assert_eq!(manual.credential_type, "api_key");
    assert!(manual.aurinko_account.is_none());
    assert_eq!(
        f.keys
            .decrypt(manual.credential_encrypted.as_ref().unwrap())
            .await
            .unwrap(),
        b"manual-mailbox-token"
    );
    let original =
        f.db.collection::<ProviderConfig>(PROVIDERS)
            .find_one(doc! {"slug":"aurinko"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(original.provider_type, "api_key");
    assert!(available(&original));
}

#[tokio::test]
async fn cancellation_during_paused_account_verification_fences_late_commit() {
    use axum::{
        Json, Router,
        extract::State,
        routing::{get, post},
    };
    use std::sync::Arc;
    let f = Fixture::new().await;
    let attempt = f.start(MailProvider::IMAP, None).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route(
            "/v1/auth/token/code",
            post(|| async { Json(json!({"accountId":42,"accessToken":"mailbox-bearer"})) }),
        )
        .route(
            "/v1/account",
            get(
                |State((entered, release)): State<(
                    Arc<tokio::sync::Notify>,
                    Arc<tokio::sync::Notify>,
                )>| async move {
                    entered.notify_one();
                    release.notified().await;
                    Json(account("IMAP"))
                },
            ),
        )
        .with_state((entered.clone(), release.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = AurinkoClient {
        origin: Some(origin),
    };
    let callback = complete_with_client(
        &f.db,
        &f.keys,
        CallbackInput {
            state_id: &attempt.state_id,
            browser_nonce: &attempt.browser_nonce,
            actor: &f.owner,
            session_id: &f.session,
            code: Some("code"),
            error: None,
            status: None,
        },
        &client,
    );
    let cancellation = async {
        entered.notified().await;
        cancel(
            &f.db,
            &f.owner,
            &f.session,
            super::super::user_token_service::chat_attempt_nonce_from_state(&attempt.state_id)
                .unwrap(),
        )
        .await
        .unwrap();
        release.notify_one();
    };
    let (result, ()) = tokio::join!(callback, cancellation);
    server.abort();
    assert!(result.is_err());
    let key = f.key().await;
    assert_eq!(key.status, "pending_auth");
    assert!(key.access_token_encrypted.is_none());
    assert_eq!(key.credential_epoch, 1);
    assert!(key.oauth_attempt_nonce.is_none());
}

#[tokio::test]
async fn org_admin_commit_revalidates_membership_and_inherited_scope_after_exchange() {
    use crate::models::org_membership::{MemberScopeSource, OrgMembership, OrgRole};
    use axum::{
        Json, Router,
        extract::State,
        routing::{get, post},
    };
    use std::sync::Arc;
    for drift in ["none", "membership", "scope"] {
        let mut f = Fixture::new().await;
        let org = Uuid::new_v4().to_string();
        f.db.collection::<User>("users")
            .insert_one(crate::test_utils::test_user(&org, UserType::Org))
            .await
            .unwrap();
        let mut membership =
            crate::test_utils::test_membership(&org, &f.actor, OrgRole::Admin, None);
        membership.scope_source = MemberScopeSource::Inherit;
        f.db.collection::<OrgMembership>("org_memberships")
            .insert_one(&membership)
            .await
            .unwrap();
        f.db.collection::<Document>("org_role_scopes")
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"org_user_id":1,"role":1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        for collection in [KEYS, SERVICES, "user_endpoints"] {
            f.db.collection::<Document>(collection)
                .update_many(doc! {"user_id":&f.owner}, doc! {"$set":{"user_id":&org}})
                .await
                .unwrap();
        }
        f.owner = org;
        let attempt = f.start(MailProvider::IMAP, None).await;
        let scope =
            f.db.collection::<Document>("org_role_scopes")
                .find_one(doc! {"org_user_id":&f.owner,"role":"admin"})
                .await
                .unwrap()
                .unwrap();
        assert!(matches!(
            scope.get("allowed_service_ids"),
            Some(bson::Bson::Null)
        ));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let app = Router::new()
            .route(
                "/v1/auth/token/code",
                post(|| async { Json(json!({"accountId":42,"accessToken":"mailbox-bearer"})) }),
            )
            .route(
                "/v1/account",
                get(
                    |State((entered, release)): State<(
                        Arc<tokio::sync::Notify>,
                        Arc<tokio::sync::Notify>,
                    )>| async move {
                        entered.notify_one();
                        release.notified().await;
                        Json(account("IMAP"))
                    },
                ),
            )
            .with_state((entered.clone(), release.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = AurinkoClient {
            origin: Some(format!("http://{}", listener.local_addr().unwrap())),
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let callback = complete_with_client(
            &f.db,
            &f.keys,
            CallbackInput {
                state_id: &attempt.state_id,
                browser_nonce: &attempt.browser_nonce,
                actor: &f.actor,
                session_id: &f.session,
                code: Some("code"),
                error: None,
                status: None,
            },
            &client,
        );
        let mutate = async {
            entered.notified().await;
            if drift == "membership" {
                f.db.collection::<Document>("org_memberships")
                    .update_one(
                        doc! {"_id":&membership.id},
                        doc! {"$set":{"revoked_at":bson::DateTime::now()}},
                    )
                    .await
                    .unwrap();
            }
            if drift == "scope" {
                f.db.collection::<Document>("org_role_scopes")
                    .update_one(
                        doc! {"org_user_id":&f.owner,"role":"admin"},
                        doc! {"$set":{"allowed_service_ids":[]}},
                    )
                    .await
                    .unwrap();
            }
            release.notify_one();
        };
        let (result, ()) = tokio::join!(callback, mutate);
        server.abort();
        assert_eq!(result.is_ok(), drift == "none", "{drift}");
        let key = f.key().await;
        assert_eq!(
            key.status,
            if drift == "none" { "active" } else { "failed" }
        );
        assert_eq!(key.credential_source.as_deref(), Some("platform"));
        assert_eq!(key.user_id, f.owner);
        assert_eq!(key.access_token_encrypted.is_some(), drift == "none");
    }
}
