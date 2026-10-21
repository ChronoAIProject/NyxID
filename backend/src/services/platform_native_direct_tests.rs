use super::*;
use crate::models::platform_entry_identity::NATIVE_RECEIPTS;
use crate::services::{
    channel_platform::*,
    platform_manager_command_service as commands,
    platform_manager_contract::{DeliveryOutcome, OriginalManagerCommand, TurnInput},
    platform_manager_delivery as delivery,
    platform_personal_binding_service::{IdentityProducer, LifecycleProducer},
};
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::sync::Arc;

// Delegate real WhatsApp authentication, filtering, parsing, credential
// declarations and send implementation. Every HTTP send uses the counted local
// server; unsupported fixture operations refuse before making any request.
struct LocalWhatsApp {
    base: String,
    real: crate::services::channel_adapters::whatsapp::WhatsAppAdapter,
}
#[async_trait::async_trait]
impl PlatformAdapter for LocalWhatsApp {
    fn platform_id(&self) -> &str {
        self.real.platform_id()
    }
    fn outbound_capabilities(&self) -> OutboundCapabilities {
        self.real.outbound_capabilities()
    }
    fn media_capabilities(&self) -> MediaCapabilities {
        self.real.media_capabilities()
    }
    fn registration(&self) -> RegistrationDescriptor {
        self.real.registration()
    }
    async fn build_verify_secrets(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        bot: &ChannelBot,
    ) -> AppResult<PlatformVerifySecrets> {
        self.real.build_verify_secrets(keys, bot).await
    }
    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        self.real.verify_webhook(bot, secrets, headers, body).await
    }
    async fn prepare_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<PreparedWebhook> {
        self.real.prepare_webhook(bot, secrets, headers, body).await
    }
    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        self.real.parse_inbound(body).await
    }
    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        crate::services::channel_adapters::whatsapp::send_outcome_at(
            http,
            credentials,
            conversation,
            reply,
            &self.base,
        )
        .await?
        .into_result()
    }
    async fn register_webhook(
        &self,
        _: &reqwest::Client,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<()> {
        Err(crate::services::platform_entry_identity::refused())
    }
    async fn verify_bot_token(
        &self,
        _: &reqwest::Client,
        _: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        Err(crate::services::platform_entry_identity::refused())
    }
}

struct Fixture {
    state: AppState,
    bot: ChannelBot,
    configuration: NativeEntryConfiguration,
    profile: String,
    adapter: Arc<dyn PlatformAdapter>,
}
impl Fixture {
    async fn new(platform: &str, base: &str) -> Self {
        let (state, mut bot, original) = discord_fixture(base).await;
        bot.platform = platform.into();
        bot.platform_bot_id = if platform == "whatsapp" {
            "123456"
        } else {
            "cli_native_app"
        }
        .into();
        bot.app_id = (platform == "whatsapp").then(|| "987654".into());
        bot.public_key = None;
        bot.bot_token_encrypted = state
            .encryption_keys
            .encrypt(if platform == "whatsapp" {
                b"native-whatsapp-access-token".as_slice()
            } else {
                b"cli_native_app:native-app-secret".as_slice()
            })
            .await
            .unwrap();
        if platform == "whatsapp" {
            bot.app_secret_encrypted = Some(
                state
                    .encryption_keys
                    .encrypt(b"native-meta-app-secret")
                    .await
                    .unwrap(),
            );
        } else {
            bot.lark_verification_token_encrypted = Some(
                state
                    .encryption_keys
                    .encrypt(b"native-verification-token")
                    .await
                    .unwrap(),
            );
            bot.lark_encrypt_key_encrypted = Some(
                state
                    .encryption_keys
                    .encrypt(b"native-event-encrypt-key")
                    .await
                    .unwrap(),
            );
        }
        let adapter: Arc<dyn PlatformAdapter> = if platform == "whatsapp" {
            Arc::new(LocalWhatsApp {
                base: base.into(),
                real: crate::services::channel_adapters::whatsapp::WhatsAppAdapter,
            })
        } else {
            Arc::new(
                crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                    base, platform,
                ),
            )
        };
        let mut configuration = native::configuration(&bot, NativeControlSurface::PrivateMessage);
        configuration.credential_fingerprint =
            EntryCredentialSnapshot::load(&state.db, adapter.as_ref(), &bot)
                .await
                .unwrap()
                .fingerprint()
                .unwrap();
        let mut stored = bson::to_document(&bot).unwrap();
        stored.insert("platform_profile_id", &original.profile_id);
        stored.insert("platform_entry_version", 2);
        state
            .db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id":&bot.id}, stored)
            .await
            .unwrap();
        state.db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(doc! {"_id":&original.profile_id}, doc! {"$set":{
                "manager_bot_id":&bot.platform_bot_id, "native_entry":bson::to_bson(&configuration).unwrap(),
                "observation_id":&configuration.credential_fingerprint}}).await.unwrap();
        state.db.collection::<bson::Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(doc! {"platform_config_id":&original.profile_id}, doc! {"$set":{
                "manager_bot_id":&bot.platform_bot_id,"platform_entry":bson::to_bson(&configuration.identity).unwrap()}}).await.unwrap();
        state
            .db
            .collection::<bson::Document>(NATIVE_RECEIPTS)
            .delete_many(doc! {})
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>(HANDLES)
            .delete_many(doc! {})
            .await
            .unwrap();
        Self {
            state,
            bot,
            configuration,
            profile: original.profile_id,
            adapter,
        }
    }
    fn payload(&self, text: &str) -> Value {
        if self.bot.platform == "whatsapp" {
            json!({"object":"whatsapp_business_account","entry":[{"id":"987654","changes":[{
                "field":"messages","value":{"messaging_product":"whatsapp","metadata":{"phone_number_id":"123456"},
                "contacts":[{"wa_id":"15551234567","profile":{"name":"Person"}}],
                "messages":[{"id":"wamid.native-original","from":"15551234567","timestamp":Utc::now().timestamp().to_string(),
                    "type":"text","text":{"body":text}}]}}]}]})
        } else {
            json!({"schema":"2.0","header":{"event_id":"ev_native_original","event_type":"im.message.receive_v1",
                "app_id":"cli_native_app","token":"native-verification-token"},
                "event":{"sender":{"sender_id":{"open_id":"ou_native_person"},"sender_type":"user"},
                    "message":{"message_id":"om_native_original","chat_id":"oc_native_private","chat_type":"p2p", "message_type":"text",
                        "content":json!({"text":text}).to_string()}}})
        }
    }
    fn request(&self, payload: &Value) -> (HeaderMap, Vec<u8>) {
        if self.bot.platform == "whatsapp" {
            use hmac::{Hmac, Mac};
            let body = serde_json::to_vec(payload).unwrap();
            let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"native-meta-app-secret").unwrap();
            mac.update(&body);
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-hub-signature-256",
                format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
                    .parse()
                    .unwrap(),
            );
            (headers, body)
        } else {
            use base64::Engine;
            use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
            use sha2::{Digest, Sha256};
            let plain = serde_json::to_vec(payload).unwrap();
            let iv = [17_u8; 16];
            let key = Sha256::digest(b"native-event-encrypt-key");
            let mut buffer = plain.clone();
            buffer.resize(plain.len() + 16, 0);
            let ciphertext = cbc::Encryptor::<aes::Aes256>::new(&key, (&iv).into())
                .encrypt_padded_mut::<Pkcs7>(&mut buffer, plain.len())
                .unwrap();
            let mut combined = iv.to_vec();
            combined.extend_from_slice(ciphertext);
            let body = serde_json::to_vec(
                &json!({"encrypt":base64::engine::general_purpose::STANDARD.encode(combined)}),
            )
            .unwrap();
            let timestamp = Utc::now().timestamp().to_string();
            let nonce = "native-direct-test-nonce";
            let mut signed = format!("{timestamp}{nonce}native-event-encrypt-key").into_bytes();
            signed.extend_from_slice(&body);
            let mut headers = HeaderMap::new();
            headers.insert("x-lark-request-timestamp", timestamp.parse().unwrap());
            headers.insert("x-lark-request-nonce", nonce.parse().unwrap());
            headers.insert(
                "x-lark-signature",
                hex::encode(Sha256::digest(signed)).parse().unwrap(),
            );
            (headers, body)
        }
    }
    async fn prepare(
        &self,
        payload: &Value,
    ) -> AppResult<crate::services::platform_entry_native::AuthenticatedNativeBatch> {
        let (headers, body) = self.request(payload);
        crate::services::platform_entry_native::prepare(
            &self.state.db,
            &self.state.encryption_keys,
            self.adapter.as_ref(),
            &self.bot,
            &headers,
            &body,
            &self.configuration,
        )
        .await
    }
    async fn control_reply_once(&self, base: &str) -> String {
        crate::services::platform_manager_ingress_service::ensure_indexes(&self.state.db)
            .await
            .unwrap();
        delivery::ensure_indexes(&self.state.db).await.unwrap();
        let (headers, body) = self.request(&self.payload("help"));
        let admission = retain_admission(
            &self.state,
            &self.bot,
            &self.configuration,
            &self.profile,
            self.adapter.as_ref(),
            &headers,
            &body,
        )
        .await;
        assert_eq!(admission.text(), "/help");
        let command = OriginalManagerCommand::from_verified(
            admission.verified_update(),
            TurnInput {
                channel_uploads: Vec::new(),
                text: admission.text().into(),
            },
        )
        .unwrap();
        let producers = crate::services::platform_manager_transport::compose(
            self.state.clone(),
            Arc::new(NativeSettledHook),
        );
        let lifecycle = LifecycleProducer {
            personal: producers.personal.clone(),
            public: producers.public.clone(),
        };
        let committed = commands::execute(
            &self.state.db,
            self.state.auth_device_hmac_key.as_slice(),
            self.state.audit_chain_hmac_key.as_slice(),
            &command,
            commands::PersonalCommandKind::Help,
            &lifecycle,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            commands::execute(
                &self.state.db,
                self.state.auth_device_hmac_key.as_slice(),
                self.state.audit_chain_hmac_key.as_slice(),
                &command,
                commands::PersonalCommandKind::Help,
                &lifecycle
            )
            .await
            .unwrap()
            .is_none()
        );
        let expected = committed
            .reply
            .render(&self.state.config.frontend_url, &committed.manager.entry)
            .unwrap();
        let transport = Arc::new(NativeTestTransport {
            adapter: self.adapter.clone(),
            discord_base: base.into(),
        });
        let api = crate::services::telegram_new_api::TelegramApi {
            http: &self.state.http_client,
            base_url: base,
        };
        let outcome = with_transport_for_test(
            transport,
            delivery::deliver_command(&self.state, Arc::new(IdentityProducer), committed, &api),
        )
        .await
        .unwrap();
        assert!(
            matches!(outcome,DeliveryOutcome::NativeSent {platform_message_ids} if platform_message_ids==["native-direct-sent"])
        );
        let stored = self
            .state
            .db
            .collection::<PlatformManagerDelivery>(DELIVERIES)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.state, DeliveryState::Sent);
        assert_eq!(stored.platform_message_ids, ["native-direct-sent"]);
        assert!(stored.claim_id.is_some());
        assert!(stored.telegram_message_ids.is_empty());
        let ingress = self
            .state
            .db
            .collection::<PlatformManagerIngress>(
                crate::models::platform_manager_ingress::COLLECTION_NAME,
            )
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ingress.observation_id, admission.receipt().id);
        assert_eq!(ingress.phase, IngressPhase::Settled);
        assert_eq!(
            delivery::sweep(
                &self.state,
                producers.identity.clone(),
                producers.public.clone(),
                10
            )
            .await
            .unwrap()
            .processed,
            0
        );
        expected.to_string()
    }
}

#[tokio::test]
async fn stored_whatsapp_signature_receiving_account_and_original_message_reach_one_real_control_reply()
 {
    let server = MockServer::start().await;
    let f = Fixture::new("whatsapp", &server.uri()).await;
    for (pointer, value) in [
        ("/entry/0/id", json!("other-waba")),
        (
            "/entry/0/changes/0/value/metadata/phone_number_id",
            json!("other-phone"),
        ),
        (
            "/entry/0/changes/0/value/messages/0/type",
            json!("group_notification"),
        ),
    ] {
        let mut payload = f.payload("help");
        *payload.pointer_mut(pointer).unwrap() = value;
        assert!(
            f.prepare(&payload).await.unwrap().fact(0).is_none(),
            "{pointer}"
        );
    }
    let mut forwarded = f.payload("help");
    forwarded["entry"][0]["changes"][0]["value"]["messages"][0]["context"] =
        json!({"forwarded":true});
    assert!(f.prepare(&forwarded).await.unwrap().fact(0).is_none());
    let (_, body) = f.request(&f.payload("help"));
    assert!(
        crate::services::platform_entry_native::prepare(
            &f.state.db,
            &f.state.encryption_keys,
            f.adapter.as_ref(),
            &f.bot,
            &HeaderMap::new(),
            &body,
            &f.configuration
        )
        .await
        .is_err()
    );
    Mock::given(method("POST"))
        .and(path("/123456/messages"))
        .and(header(
            "authorization",
            "Bearer native-whatsapp-access-token",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages":[{"id":"native-direct-sent"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let expected = f.control_reply_once(&server.uri()).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let posted: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(posted["to"], "15551234567");
    assert_eq!(posted["type"], "text");
    assert_eq!(posted["text"]["body"], expected);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stored_lark_and_feishu_encrypted_original_private_events_reach_one_real_control_reply() {
    for platform in ["lark", "feishu"] {
        let server = MockServer::start().await;
        let f = Fixture::new(platform, &server.uri()).await;
        for (pointer, value) in [
            ("/header/app_id", json!("cli_other_app")),
            ("/event/message/chat_type", json!("group")),
            ("/event/sender/sender_type", json!("app")),
        ] {
            let mut payload = f.payload("help");
            *payload.pointer_mut(pointer).unwrap() = value;
            assert!(
                f.prepare(&payload).await.unwrap().fact(0).is_none(),
                "{platform} {pointer}"
            );
        }
        let mut forwarded = f.payload("help");
        forwarded["event"]["message"]["forwarded"] = true.into();
        assert!(f.prepare(&forwarded).await.unwrap().fact(0).is_none());
        let mut wrong = f.payload("help");
        wrong["header"]["token"] = "wrong-token".into();
        assert!(f.prepare(&wrong).await.is_err());
        let (_, body) = f.request(&f.payload("help"));
        assert!(
            crate::services::platform_entry_native::prepare(
                &f.state.db,
                &f.state.encryption_keys,
                f.adapter.as_ref(),
                &f.bot,
                &HeaderMap::new(),
                &body,
                &f.configuration
            )
            .await
            .is_err()
        );
        Mock::given(method("POST"))
            .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"code":0,"tenant_access_token":"local-tenant-token","expire":7200}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/open-apis/im/v1/messages"))
            .and(header("authorization", "Bearer local-tenant-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"native-direct-sent"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let expected = f.control_reply_once(&server.uri()).await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            2,
            "one tenant exchange and one send; all egress local"
        );
        let send = requests
            .iter()
            .find(|r| r.url.path() == "/open-apis/im/v1/messages")
            .unwrap();
        let posted: Value = serde_json::from_slice(&send.body).unwrap();
        assert_eq!(posted["receive_id"], "oc_native_private");
        let content: Value = serde_json::from_str(posted["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["text"], expected);
        f.state.db.drop().await.unwrap();
    }
}
