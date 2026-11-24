//! Two server revisions exchange raw test rows, using the real create, move
//! and clear paths and a loopback gateway. Only the channel key hash is compared.
use super::*;
use bson::Document;
use futures::TryStreamExt;

#[tokio::test]
async fn channel_key_mixed_records() {
    let Ok(step) = std::env::var("NYXID_RECORD_FIXTURE_STEP") else {
        return;
    };
    let path = std::env::var("NYXID_RECORD_FIXTURE_FILE").unwrap();
    let (mut state, _, upstream) = setup("channel_key_mixed").await;
    let active = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    state.config.contextual_record_writes = active;
    state.config.allow_legacy_record_reads =
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false");
    state.encryption_keys = Arc::new(crate::crypto::aes::EncryptionKeys::from_config(
        &state.config,
    ));
    let (_, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    let collections = [
        CHANNELS,
        crate::models::channel_bot::COLLECTION_NAME,
        ROUTES,
        crate::models::api_key::COLLECTION_NAME,
        crate::models::assistant_agent::COLLECTION_NAME,
    ];
    let mut file = if step == "write" {
        Document::new()
    } else {
        bson::from_slice::<Document>(&std::fs::read(&path).unwrap()).unwrap()
    };
    for name in collections {
        if let Ok(rows) = file.get_array(name) {
            for row in rows {
                state
                    .db
                    .collection::<Document>(name)
                    .insert_one(row.as_document().unwrap())
                    .await
                    .unwrap();
            }
        }
    }
    let row = if step == "write" {
        set_flag(&state, FlagTarget::User(OWNER.into()), true).await;
        Box::pin(linked(&state, None, "lark")).await
    } else {
        let stored = load_channel(&state, OWNER, file.get_str("channel_id").unwrap())
            .await
            .unwrap();
        TEST_BOT_USER_IDS
            .lock()
            .unwrap()
            .insert(stored.channel_bot_id.clone(), "ou_bot".into());
        stored
    };
    let rows = state.db.collection::<Document>(CHANNELS);
    let filter = doc! {"_id":&row.id};
    match step.as_str() {
        "write" => assert_eq!(row.transport, "gateway"),
        "replace" | "clear" => {
            if row.transport == "gateway" {
                set_flag(&state, FlagTarget::User(OWNER.into()), false).await;
                due(&state, &row.id).await;
                Box::pin(switch_to_gateway(&state)).await.unwrap();
                assert_eq!(
                    load_channel(&state, OWNER, &row.id)
                        .await
                        .unwrap()
                        .transport,
                    "direct"
                );
            }
            if step == "replace" {
                set_flag(&state, FlagTarget::User(OWNER.into()), true).await;
                let direct = load_channel(&state, OWNER, &row.id).await.unwrap();
                Box::pin(move_to_gateway(&state, &direct)).await.unwrap();
            }
        }
        "migrate" => {
            use crate::services::encrypted_record_migration::{Mode, advance, start, status};
            let generation = start(&state.db, &state.encryption_keys, Mode::Migrate, None)
                .await
                .unwrap();
            loop {
                let r = status(&state.db, &state.encryption_keys)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(r.phase, "running");
                Box::pin(advance(&state.db, &state.encryption_keys, &generation))
                    .await
                    .unwrap();
                if r.collection == Some(CHANNELS) {
                    break;
                }
            }
        }
        _ => {
            let mut requested = row.clone();
            match step.as_str() {
                "swap-owner" => requested.user_id = Uuid::new_v4().to_string(),
                "swap-id" => requested.id = Uuid::new_v4().to_string(),
                "swap-key" => {
                    requested.pending_agent_api_key_id = requested.agent_api_key_id.take();
                    requested.agent_api_key_id = Some(requested.route_api_key_id.clone());
                }
                "read" | "reject" => {}
                other => panic!("unknown fixture step {other}"),
            }
            let before = rows.find_one(filter.clone()).await.unwrap().unwrap();
            let opened = decrypt_agent_key(&state, &requested).await;
            if step == "read" {
                let digest = opened.unwrap().map(|key| sha256_hex(key.as_bytes()));
                assert_eq!(digest.as_deref(), file.get_str("digest").ok());
            } else {
                assert!(opened.is_err());
            }
            assert!(rows.find_one(filter).await.unwrap().unwrap() == before);
            gateway.abort();
            upstream.abort();
            return;
        }
    }
    let after = load_channel(&state, OWNER, &row.id).await.unwrap();
    let raw = rows.find_one(filter).await.unwrap().unwrap();
    let marker = raw
        .get_document("secret_context_versions")
        .ok()
        .and_then(|v| v.get_i32("agent_key_ciphertext").ok());
    let previous = file.remove("digest");
    if step == "clear" {
        assert!(
            after.agent_api_key_id.is_none()
                && !raw.contains_key("agent_key_ciphertext")
                && marker.is_none()
        );
    } else {
        assert!(raw.get_binary_generic("agent_key_ciphertext").is_ok());
        assert_eq!(marker, active.then_some(3));
        let secret = decrypt_agent_key(&state, &after).await.unwrap().unwrap();
        let digest = sha256_hex(secret.as_bytes());
        let key =
            key_service::get_api_key(&state.db, OWNER, after.agent_api_key_id.as_deref().unwrap())
                .await
                .unwrap();
        assert_eq!(digest, key.key_hash);
        if step == "migrate" {
            assert_eq!(
                previous.as_ref().and_then(bson::Bson::as_str),
                Some(digest.as_str())
            );
        }
        if step == "replace" {
            assert_ne!(row.agent_api_key_id, after.agent_api_key_id);
        }
        file.insert("digest", digest);
    }
    file.insert("channel_id", &row.id);
    for name in collections {
        let docs: Vec<Document> = state
            .db
            .collection::<Document>(name)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        file.insert(name, docs);
    }
    std::fs::write(path, bson::to_vec(&file).unwrap()).unwrap();
    gateway.abort();
    upstream.abort();
}
