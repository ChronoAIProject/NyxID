//! Shared by two independently compiled servers. Exchange raw binding rows
//! and compare retained credentials to digests recorded at the setup boundary.
use super::*;
use crate::crypto::record_field::{self, RecordContext};
use crate::models::{api_key, assistant_agent};
use crate::services::{
    encrypted_record_migration as migration, nyxbot_event_binding_service as bindings,
};
use bson::Document;
use tokio::sync::{Notify, mpsc};

const PROVIDER: &str = "provider_key_ciphertext";
const PRODUCER: &str = "producer_secret_ciphertext";

pub(super) struct Gateway {
    pub(super) entered: mpsc::UnboundedReceiver<[String; 2]>,
    pub(super) resume: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn gateway(state: &AppState, pause: bool, reject: bool) -> Gateway {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "cmaeg".into();
    catalog.base_url = base;
    catalog.requires_user_credential = false;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(catalog)
        .await
        .unwrap();
    let (tx, entered) = mpsc::unbounded_channel();
    let resume = Arc::new(Notify::new());
    let gate = resume.clone();
    let app=Router::new().route("/channels",post(move |Json(body):Json<Value>| {
        let tx=tx.clone();
        let gate=gate.clone();
        async move {
            let provider=body["credentials"]["agent_key"].as_str().unwrap();
            let producer=body["credentials"]["hmac_secrets"][0]["secret"].as_str().unwrap();
            tx.send([sha256_hex(provider),sha256_hex(producer)]).unwrap();
            if pause {gate.notified().await;}
            if reject {return (StatusCode::BAD_REQUEST,Json(Value::Null));}
            (StatusCode::CREATED,Json(json!({"channel_id":"fixture-channel","record_id":body["record_id"],
                "version":1,"provider":{"binding_id":body["record_id"]},
                "endpoints":{"custom_event_url":"https://example.invalid/events"},
                "definition":{"provider":body["provider"],"sources":body["sources"],"partition":body["partition"]}})))
        }
    })).route("/channels/{id}",axum::routing::delete(|| async {StatusCode::NO_CONTENT}));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Gateway {
        entered,
        resume,
        task,
    }
}

pub(super) async fn destination_fixture(state: &AppState, agent: &str) -> String {
    flags::set_platform_override(
        &state.db,
        flags::NYXBOT_CUSTOM_EVENT_GATEWAY_FLAG_KEY,
        &FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    let (row, _) = Box::pin(channel(state, "direct")).await;
    state
        .db
        .collection::<Document>(CHANNELS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"agent_id":agent}})
        .await
        .unwrap();
    let now = bson::DateTime::now();
    state.db.collection::<Document>(THREADS).insert_one(doc!{"_id":Uuid::new_v4().to_string(),"channel_id":&row.id,
        "user_id":OWNER,"partition":PARTITION,"last_message_id":"fixture-anchor","created_at":now,"updated_at":now}).await.unwrap();
    row.id
}

async fn run_generation(state: &AppState, mode: migration::Mode) -> bool {
    // The runner requires cutover even for a Verify scan. Keep the reader's
    // legacy policy so strict-mode migration exercises the same stored row.
    let mut config = state.config.clone();
    config.contextual_record_writes = true;
    let keys = crate::crypto::aes::EncryptionKeys::from_config(&config);
    let id = migration::start(&state.db, &keys, mode, None)
        .await
        .unwrap();
    loop {
        let before = migration::status(&state.db, &keys).await.unwrap().unwrap();
        assert_eq!(before.phase, "running");
        Box::pin(migration::advance(&state.db, &keys, &id))
            .await
            .unwrap();
        if before.collection == Some(EVENT_BINDINGS) {
            let after = migration::status(&state.db, &keys).await.unwrap().unwrap();
            assert_eq!(after.scanned - before.scanned, 1);
            return before.unresolved == after.unresolved;
        }
    }
}

#[tokio::test]
async fn mixed_event_binding_keys() {
    let Ok(step) = std::env::var("NYXID_RECORD_FIXTURE_STEP") else {
        return;
    };
    let path = std::env::var("NYXID_RECORD_FIXTURE_FILE").unwrap();
    let (mut state, _, upstream) = setup("mixed_event_binding_keys").await;
    let _upstream = tokio_util::task::AbortOnDropHandle::new(upstream);
    let active = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    state.config.contextual_record_writes = active;
    state.config.allow_legacy_record_reads =
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false");
    state.encryption_keys = Arc::new(crate::crypto::aes::EncryptionKeys::from_config(
        &state.config,
    ));
    let mut remote = gateway(&state, false, false).await;
    let rows = state.db.collection::<Document>(EVENT_BINDINGS);
    let mut file = Document::new();
    if step != "write" {
        file = bson::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for name in [
            EVENT_BINDINGS,
            api_key::COLLECTION_NAME,
            assistant_agent::COLLECTION_NAME,
        ] {
            state
                .db
                .collection::<Document>(name)
                .insert_one(file.get_document(name).unwrap())
                .await
                .unwrap();
        }
    }
    if matches!(step.as_str(), "replace" | "clear") {
        let id = file
            .get_document(EVENT_BINDINGS)
            .unwrap()
            .get_str("_id")
            .unwrap();
        Box::pin(bindings::revoke(&state, OWNER, id)).await.unwrap();
        let retired = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_eq!(retired.get_str("status"), Ok("revoked"));
        for field in [PROVIDER, PRODUCER] {
            assert!(!retired.contains_key(field));
            assert!(
                retired
                    .get_document("secret_context_versions")
                    .ok()
                    .is_none_or(|m| !m.contains_key(field))
            );
        }
        file.insert(EVENT_BINDINGS, retired);
        let key = file
            .get_document(api_key::COLLECTION_NAME)
            .unwrap()
            .get_str("_id")
            .unwrap();
        let retired_key = state
            .db
            .collection::<Document>(api_key::COLLECTION_NAME)
            .find_one(doc! {"_id":key})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retired_key.get_bool("is_active"), Ok(false));
        file.insert(api_key::COLLECTION_NAME, retired_key);
    }
    if matches!(step.as_str(), "write" | "replace") {
        let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
            .await
            .unwrap();
        let destination = Box::pin(destination_fixture(&state, &agent.id)).await;
        let (row, secret, _) = Box::pin(bindings::create(
            &state,
            OWNER,
            &agent.id,
            &destination,
            PARTITION,
        ))
        .await
        .unwrap();
        let [provider, producer] = remote.entered.recv().await.unwrap();
        assert_eq!(producer, sha256_hex(secret.as_bytes()));
        file.insert("producer_hash", producer);
        file.insert("provider_hash", &provider);
        let key = state
            .db
            .collection::<Document>(api_key::COLLECTION_NAME)
            .find_one(doc! {"_id":&row.provider_api_key_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.get_str("key_hash"), Ok(provider.as_str()));
        file.insert(api_key::COLLECTION_NAME, key);
        file.insert(
            assistant_agent::COLLECTION_NAME,
            state
                .db
                .collection::<Document>(assistant_agent::COLLECTION_NAME)
                .find_one(doc! {"_id":&agent.id})
                .await
                .unwrap()
                .unwrap(),
        );
        let stored = rows.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap();
        for field in [PROVIDER, PRODUCER] {
            assert!(stored.get_binary_generic(field).is_ok());
            assert_eq!(
                stored
                    .get_document("secret_context_versions")
                    .ok()
                    .and_then(|m| m.get_i32(field).ok()),
                active.then_some(3)
            );
        }
        if !active {
            assert!(!stored.contains_key("secret_context_versions"));
        }
        file.insert(EVENT_BINDINGS, stored);
    } else if step == "migrate" {
        assert!(Box::pin(run_generation(&state, migration::Mode::Migrate)).await);
        file.insert(
            EVENT_BINDINGS,
            rows.find_one(doc! {}).await.unwrap().unwrap(),
        );
    } else if step != "clear" {
        let original = file.get_document(EVENT_BINDINGS).unwrap();
        let id = original.get_str("_id").unwrap();
        let row = bindings::get(&state, OWNER, id).await.unwrap();
        let listed = bindings::list(&state, OWNER).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, row.id);
        assert_eq!(
            Box::pin(bindings::authorize_provider(&state, &row))
                .await
                .is_ok(),
            row.active()
        );
        match step.as_str() {
            "read" if row.active() => {
                // Test-only digest oracle. Neither binary serves retained plaintext.
                for (field, key, hash) in [
                    (
                        PROVIDER,
                        row.provider_api_key_id.as_deref().unwrap(),
                        "provider_hash",
                    ),
                    (PRODUCER, &row.producer_key_id, "producer_hash"),
                ] {
                    let path = [field, key, &row.agent_id, &row.agent_owner_id];
                    let ctx =
                        RecordContext::new(EVENT_BINDINGS, &row.id, &row.user_id, &path).unwrap();
                    let version = original
                        .get_document("secret_context_versions")
                        .ok()
                        .map(|m| m.get_i32(field).unwrap() as u8);
                    let clear = record_field::decrypt_context(
                        &state.encryption_keys,
                        &ctx,
                        version,
                        original.get_binary_generic(field).unwrap(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(sha256_hex(clear.as_slice()), file.get_str(hash).unwrap());
                }
            }
            "metadata" | "read" => {}
            "verify" => assert!(Box::pin(run_generation(&state, migration::Mode::Verify)).await),
            "reject" => assert!(!Box::pin(run_generation(&state, migration::Mode::Migrate)).await),
            other => panic!("unknown fixture step {other}"),
        }
        assert!(rows.find_one(doc! {"_id":id}).await.unwrap().as_ref() == Some(original));
        assert!(remote.entered.try_recv().is_err());
        return;
    }
    std::fs::write(path, bson::to_vec(&file).unwrap()).unwrap();
}
