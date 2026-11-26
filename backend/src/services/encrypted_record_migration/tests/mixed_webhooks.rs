//! Run this same test module in the baseline and candidate server test binaries.
//! The driver exchanges a raw BSON fixture containing only test credentials.
use std::sync::Arc;

use crate::{
    crypto::aes::EncryptionKeys,
    handlers::developer_apps::{
        CreateDeveloperOAuthClientRequest, create_my_oauth_client, update_my_oauth_client,
    },
    models::security_effect_intent::{
        COLLECTION_NAME as INTENTS, SecurityEffect, SecurityEffectIntent,
    },
    services::{
        developer_webhook_service as connection, security_effect_service as effects,
        webhook_delivery_service as wire,
    },
    test_utils::{
        connect_transaction_test_database, test_app_config, test_app_state_with_config,
        test_auth_user,
    },
};
use axum::{Json, Router, body::Bytes, extract::State, http::HeaderMap, routing::post};
use bson::{Document, doc};

const OWNER: &str = "10a82af4-a595-4b33-ab42-f73322e45739";
const REVOCATION_KEY: &str = "fixture-revocation-é";

#[tokio::test]
async fn mixed_binary_webhook_signatures() {
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let db = connect_transaction_test_database("mixed_binary_webhooks").await;
    let mut config = test_app_config();
    config.contextual_record_writes =
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    if config.contextual_record_writes {
        assert_eq!(
            std::env::var("CONTEXTUAL_RECORD_WRITERS_DRAINED").as_deref(),
            Ok("true")
        );
    }
    config.allow_legacy_record_reads =
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false");
    let keys = Arc::new(EncryptionKeys::from_config(&config));
    if step.starts_with("registry-") {
        check_registry_generation(&db, &keys, &step, file.as_deref().unwrap().as_ref()).await;
        return;
    }
    let mut state = test_app_state_with_config(db.clone(), config);
    state.encryption_keys = keys.clone();
    let (send, mut received) = tokio::sync::mpsc::channel(4);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/hook");
    let _origin =
        crate::services::resource_adapter::register_test_origin(&format!("http://{address}"));
    let app = Router::new().route(
        "/hook",
        post(move |headers: HeaderMap, body: Bytes| {
            let send = send.clone();
            async move {
                send.send((headers, body)).await.unwrap();
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let rows = db.collection::<Document>("oauth_clients");

    let fixture = if matches!(step.as_str(), "write" | "roundtrip") {
        let request =
            serde_json::from_value::<CreateDeveloperOAuthClientRequest>(serde_json::json!({
                "name":"webhook fixture", "redirect_uris":["https://example.test/callback"],
                "revocation_webhook_url": &url, "revocation_webhook_secret": REVOCATION_KEY,
            }))
            .unwrap();
        let Json(client) = wire::with_test_webhook_target(
            address,
            create_my_oauth_client(
                State(state.clone()),
                test_auth_user(OWNER),
                Default::default(),
                Json(request),
            ),
        )
        .await
        .unwrap();
        let (_, secret, _) = wire::with_test_webhook_target(
            address,
            connection::configure(&db, &keys, &client.id, OWNER, &url),
        )
        .await
        .unwrap();
        let stored = rows
            .find_one(doc! {"_id":&client.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.contains_key("secret_context_versions"),
            keys.contextual_record_writes()
        );
        let fixture =
            doc! {"row":stored, "test_signing_key":secret, "test_cae_key": REVOCATION_KEY};
        if step == "write" {
            std::fs::write(file.as_ref().unwrap(), bson::to_vec(&fixture).unwrap()).unwrap();
            server.abort();
            return;
        }
        fixture
    } else {
        assert!(matches!(
            step.as_str(),
            "read"
                | "replace"
                | "clear"
                | "reject"
                | "swap-collection"
                | "swap-fields"
                | "swap-owner"
                | "swap-id"
        ));
        let mut fixture: Document =
            bson::from_slice(&std::fs::read(file.as_ref().unwrap()).unwrap()).unwrap();
        let row = fixture.get_document_mut("row").unwrap();
        match step.as_str() {
            "swap-collection" => {
                let id = row.get_str("_id").unwrap().to_owned();
                for field in [
                    "connection_webhook_secret_encrypted",
                    "revocation_webhook_secret_encrypted",
                ] {
                    let bytes = crate::crypto::record_field::encrypt(
                        &keys,
                        "other_clients",
                        &id,
                        &format!("creator:{OWNER}"),
                        field,
                        b"test-key",
                    )
                    .await
                    .unwrap();
                    row.insert(
                        field,
                        bson::Binary {
                            subtype: bson::spec::BinarySubtype::Generic,
                            bytes,
                        },
                    );
                }
            }
            "swap-fields" => {
                let revocation = row.remove("revocation_webhook_secret_encrypted").unwrap();
                let connection = row
                    .insert("connection_webhook_secret_encrypted", revocation)
                    .unwrap();
                row.insert("revocation_webhook_secret_encrypted", connection);
            }
            "swap-owner" => {
                row.insert("created_by", "different-owner");
            }
            "swap-id" => {
                row.insert("_id", uuid::Uuid::new_v4().to_string());
            }
            _ => {}
        }
        row.insert("revocation_webhook_url", &url);
        if row.get_bool("connection_webhook_enabled").unwrap() {
            row.insert("connection_webhook_url", &url);
        }
        rows.insert_one(&*row).await.unwrap();
        if matches!(step.as_str(), "replace" | "clear") {
            let id = row.get_str("_id").unwrap().to_owned();
            if step == "replace" {
                let key = "fixture-replacement";
                let request =
                    serde_json::from_value(serde_json::json!({"revocation_webhook_secret": key}))
                        .unwrap();
                let Json(_) = update_my_oauth_client(
                    State(state),
                    test_auth_user(OWNER),
                    axum::extract::Path(id.clone()),
                    Json(request),
                )
                .await
                .unwrap();
                let (_, rotated, _) = connection::rotate_secret(&db, &keys, &id, OWNER)
                    .await
                    .unwrap();
                fixture.insert("test_signing_key", rotated);
                fixture.insert("test_cae_key", key);
            } else {
                connection::disable(&db, &id, OWNER).await.unwrap();
            }
            let updated = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
            assert!(!updated.contains_key("secret_context_versions"));
            fixture.insert("row", updated);
            std::fs::write(file.unwrap(), bson::to_vec(&fixture).unwrap()).unwrap();
            server.abort();
            return;
        }
        fixture
    };
    let client_id = fixture.get_document("row").unwrap().get_str("_id").unwrap();
    let rejected = step == "reject" || step.starts_with("swap-");
    let result = connection::DeveloperWebhookDispatcher::new(keys.clone())
        .deliver_for_app(
            &db,
            client_id,
            "occurrence",
            "connect_link.completed",
            serde_json::json!({"user_id": OWNER}),
        )
        .await;
    if rejected {
        assert_eq!(result.unwrap_err().reason, "secret_decrypt_failed");
    } else if !fixture
        .get_document("row")
        .unwrap()
        .get_bool("connection_webhook_enabled")
        .unwrap()
    {
        result.unwrap();
        assert!(received.try_recv().is_err());
    } else {
        result.unwrap();
        let (headers, body) = received.try_recv().unwrap();
        let signature = wire::compute_timestamped_signature(
            fixture.get_str("test_signing_key").unwrap().as_bytes(),
            headers["X-NyxID-Timestamp"].to_str().unwrap(),
            &body,
        );
        assert_eq!(headers["X-NyxID-Signature"], format!("sha256={signature}"));
    }

    db.collection::<Document>("oauth_broker_bindings").insert_one(doc! {
        "_id":"binding", "client_id":client_id,"user_id":OWNER,"refresh_token_jti":"fixture",
        "revoked":true,"revoke_reason":"user_revoked","revoked_at":bson::DateTime::now(),"created_at":bson::DateTime::now()
    }).await.unwrap();
    let intent = effects::new_intent(
        uuid::Uuid::new_v4().to_string(),
        SecurityEffect::Cae {
            binding_id: "binding".into(),
            client_id: client_id.into(),
            revoke_source: "user".into(),
        },
    );
    db.collection::<SecurityEffectIntent>(INTENTS)
        .insert_one(&intent)
        .await
        .unwrap();
    effects::ensure_indexes(&db).await.unwrap();
    effects::sweep(&db, &keys).await.unwrap();
    if rejected {
        let saved = db
            .collection::<Document>(INTENTS)
            .find_one(doc! {"_id":&intent.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.get_str("state"), Ok("pending"));
        assert!(received.try_recv().is_err());
    } else {
        let (headers, body) = received.try_recv().unwrap();
        let signature = wire::compute_body_signature(
            fixture.get_str("test_cae_key").unwrap().as_bytes(),
            &body,
        );
        assert_eq!(headers["X-NyxID-Signature"], format!("sha256={signature}"));
        assert_eq!(headers["X-NyxID-Delivery-Id"], intent.id);
    }
    let after = rows
        .find_one(doc! {"_id":client_id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        fixture.get_document("row").unwrap() == &after,
        "delivery changed the client row"
    );
    server.abort();
}

async fn check_registry_generation(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    step: &str,
    file: &std::path::Path,
) {
    use super::{Mode, REGISTRY, RUN_ID, RUNS, advance, start, status};
    let runs = db.collection::<Document>(RUNS);
    if step == "registry-write" {
        start(db, keys, Mode::Verify, None).await.unwrap();
        let index = REGISTRY
            .iter()
            .position(|e| e.collection == "oauth_clients")
            .unwrap();
        runs.update_one(
            doc! {"_id": RUN_ID},
            doc! {"$set":{
                "entry":index as i32,"scanned":4_i64,
                "field_outcomes": { index.to_string(): {"1": {"authenticated_current":4_i64}} }
            }},
        )
        .await
        .unwrap();
        let row = runs.find_one(doc! {"_id": RUN_ID}).await.unwrap().unwrap();
        std::fs::write(file, bson::to_vec(&row).unwrap()).unwrap();
        return;
    }
    let row: Document = bson::from_slice(&std::fs::read(file).unwrap()).unwrap();
    runs.insert_one(row).await.unwrap();
    let report = status(db, keys).await.unwrap().unwrap();
    assert_eq!(report.scanned, 4);
    let live = step == "registry-live";
    assert_eq!(
        report
            .blockers
            .contains(&"registry_or_primary_provider_changed"),
        !live
    );
    if step == "registry-unknown" {
        assert_eq!(report.collection, None);
        assert!(report.field_outcomes.is_empty());
    } else {
        assert!(live || step == "registry-known");
        assert_eq!(report.collection, Some("oauth_clients"));
        assert!(
            report
                .field_outcomes
                .iter()
                .any(|f| f.collection == "oauth_clients"
                    && f.field == "connection_webhook_secret_encrypted"
                    && f.authenticated_current == 4)
        );
    }
    assert_eq!(advance(db, keys, &report.generation).await.is_ok(), live);
}
