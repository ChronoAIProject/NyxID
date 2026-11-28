//! Real Graph exchange, progress and selection on each side of the rollout.
#[path = "historical_assets.rs"]
mod historical;

use super::*;
use crate::{AppState, crypto::aes::EncryptionKeys, test_utils};
use bson::Document;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn keys(writes: bool, legacy: bool) -> EncryptionKeys {
    let mut keys = test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(writes);
    keys.set_allow_legacy_record_reads(legacy);
    keys
}

#[derive(Serialize, Deserialize)]
struct Attempt {
    row: Document,
    actor: String,
    id: String,
    progress: Vec<u8>,
}

impl Attempt {
    async fn issue(app: &AppState, server: &MockServer, bot: &ChannelBot) -> Self {
        if bot.platform == "instagram" {
            instagram(server).await;
        } else {
            messenger_graph(server).await;
        }
        let api = service(app, server);
        let started = api
            .start(&bot.user_id, &bot.user_id, &bot.platform, "Inbox", None)
            .await
            .unwrap();
        let nonce = state_value(&started);
        let progress = api
            .exchange(&bot.user_id, &bot.platform, &nonce, Some("code"))
            .await
            .unwrap();
        assert_eq!(
            progress.assets.len(),
            if bot.platform == "instagram" { 1 } else { 2 }
        );
        let requests = server.received_requests().await.unwrap().len();
        let public = serde_json::to_vec(&progress).unwrap();
        let repeat = api
            .exchange(&bot.user_id, &bot.platform, &nonce, Some("retry"))
            .await
            .unwrap();
        assert_eq!(public, serde_json::to_vec(&repeat).unwrap());
        assert_eq!(requests, server.received_requests().await.unwrap().len());
        Self {
            row: app
                .db
                .collection::<Document>(SESSIONS)
                .find_one(doc! {"_id": &started.session_id})
                .await
                .unwrap()
                .unwrap(),
            actor: bot.user_id.clone(),
            id: started.session_id,
            progress: public,
        }
    }

    async fn row(&self, app: &AppState) -> Document {
        app.db
            .collection::<Document>(SESSIONS)
            .find_one(doc! {"_id": &self.id})
            .await
            .unwrap()
            .unwrap()
    }

    async fn select(&self, app: &AppState, server: &MockServer, platform: &str, reject: bool) {
        let api = service(app, server);
        let before = self.row(app).await;
        let collection = app.db.collection::<Document>(BOTS);
        let count = || collection.count_documents(doc! {});
        let bots = count().await.unwrap();
        let calls = server.received_requests().await.unwrap().len();
        let progress = api.progress(&self.actor, &self.id).await;
        let asset = if platform == "messenger" {
            "888"
        } else {
            "777"
        };
        if reject {
            assert!(progress.is_err());
            assert!(
                api.finish(&self.actor, &self.id, asset, true)
                    .await
                    .is_err()
            );
            assert!(self.row(app).await == before);
            assert_eq!(server.received_requests().await.unwrap().len(), calls);
            assert_eq!(count().await.unwrap(), bots);
            return;
        }
        assert_eq!(
            serde_json::to_vec(&progress.unwrap()).unwrap(),
            self.progress
        );
        assert_eq!(server.received_requests().await.unwrap().len(), calls);
        // Complete the installation with a serving writer. The driver also probes
        // strict reads with the writer disabled, an unsupported startup combination.
        let mut writer = app.clone();
        if !app.encryption_keys.allow_legacy_record_reads() {
            writer.encryption_keys = keys(true, false).into();
        }
        let api = service(&writer, server);
        let first = api
            .finish(&self.actor, &self.id, asset, true)
            .await
            .unwrap();
        assert_eq!(first.platform_bot_id, asset);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(
                    |r| r.method.as_str() == "POST" && r.url.path().ends_with("/subscribed_apps")
                )
                .count(),
            1
        );
        let repeat = api
            .finish(&self.actor, &self.id, asset, true)
            .await
            .unwrap();
        assert_eq!(repeat.id, first.id);
        assert_eq!(count().await.unwrap(), bots + 1);
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            requests.len()
        );
        let completed = self.row(app).await;
        assert_eq!(completed.get_str("status"), Ok("completed"));
        assert!(
            !completed.contains_key("assets_encrypted")
                && !completed.contains_key("assets_context_version")
        );
    }
}

fn fixture_path(platform: &str) -> Option<PathBuf> {
    std::env::var_os("NYXID_RECORD_FIXTURE_FILE")
        .map(|path| PathBuf::from(path).with_extension(platform))
}

async fn restore(file: &std::path::Path) -> (AppState, Attempt) {
    let attempt: Attempt = bson::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let platform = attempt.row.get_str("platform").unwrap();
    let (app, _, _, mut bot) = fixture(platform).await;
    use crate::models::user::{COLLECTION_NAME, User, UserType};
    app.db
        .collection::<User>(COLLECTION_NAME)
        .insert_one(test_utils::test_user(&attempt.actor, UserType::Person))
        .await
        .unwrap();
    bot.user_id.clone_from(&attempt.actor);
    configure(&app, &bot).await;
    app.db
        .collection::<Document>(SESSIONS)
        .insert_one(&attempt.row)
        .await
        .unwrap();
    (app, attempt)
}

#[tokio::test]
async fn mixed_consent_exchange_and_selection() {
    let writes = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    let legacy = std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false");
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    for platform in ["messenger", "instagram"] {
        let file = fixture_path(platform);
        let (app, server, attempt) = if step == "write" || step == "roundtrip" {
            let (mut app, _, server, bot) = fixture(platform).await;
            configure(&app, &bot).await;
            app.encryption_keys = keys(writes, legacy).into();
            let attempt = Attempt::issue(&app, &server, &bot).await;
            if let Some(file) = file {
                std::fs::write(file, bson::to_vec(&attempt).unwrap()).unwrap();
                continue;
            }
            (app, server, attempt)
        } else {
            assert!(matches!(step.as_str(), "read" | "reject"));
            let (mut app, attempt) = restore(&file.unwrap()).await;
            app.encryption_keys = keys(writes, legacy).into();
            let server = MockServer::start().await;
            if platform == "instagram" {
                instagram_selection(&server).await;
            } else {
                messenger_graph(&server).await;
            }
            (app, server, attempt)
        };
        attempt
            .select(&app, &server, platform, step == "reject")
            .await;
    }
}

// Candidate storage tests
#[tokio::test]
async fn rewrap_preserves_exchange_bytes() {
    for platform in ["messenger", "instagram"] {
        let (app, mut attempt) = if let Some(file) = fixture_path(platform) {
            restore(&file).await
        } else {
            let (mut app, _, server, bot) = fixture(platform).await;
            configure(&app, &bot).await;
            app.encryption_keys = keys(false, true).into();
            let attempt = Attempt::issue(&app, &server, &bot).await;
            (app, attempt)
        };
        let active = keys(true, true);
        let before = attempt.row(&app).await;
        assert!(!before.contains_key("assets_context_version"));
        assert!(
            asset_record::migrate(&app.db, &active, &before)
                .await
                .unwrap()
        );
        assert!(
            !asset_record::migrate(&app.db, &active, &before)
                .await
                .unwrap()
        );
        let mut after = attempt.row(&app).await;
        asset_record::verify(&active, &after).await.unwrap();
        let s: OAuthSession = bson::from_document(after.clone()).unwrap();
        let identity = serde_json::to_string(&(
            s.created_at.timestamp_millis(),
            &s.bot_id,
            &s.reconnect_bot_id,
            s.expected_generation,
        ))
        .unwrap();
        let path = [
            "assets_encrypted",
            &s.actor_id,
            &s.platform,
            &s.app_id,
            s.grant_subject_id.as_deref().unwrap(),
            &s.state_hash,
            &identity,
        ];
        let ctx =
            crate::crypto::aes::RecordContext::new(SESSIONS, &s.id, &s.owner_id, &path).unwrap();
        let original = crate::crypto::record_field::decrypt_context(
            &active,
            &ctx,
            None,
            before.get_binary_generic("assets_encrypted").unwrap(),
        )
        .await
        .unwrap();
        let decrypted = active
            .decrypt_record(after.get_binary_generic("assets_encrypted").unwrap(), &ctx)
            .await
            .unwrap();
        assert!(decrypted == original);
        if let Some(file) = fixture_path(platform) {
            attempt.row = after.clone();
            std::fs::write(file, bson::to_vec(&attempt).unwrap()).unwrap();
        }
        assert_eq!(
            after.remove("assets_context_version"),
            Some(bson::Bson::Int32(3))
        );
        after.insert(
            "assets_encrypted",
            before.get("assets_encrypted").unwrap().clone(),
        );
        assert!(after == before);
    }
}

async fn consent(platform: &str, writes: bool) -> (AppState, MockServer, Attempt) {
    let (mut app, _, server, bot) = fixture(platform).await;
    configure(&app, &bot).await;
    app.encryption_keys = keys(writes, true).into();
    let attempt = Attempt::issue(&app, &server, &bot).await;
    (app, server, attempt)
}

#[tokio::test]
async fn contextual_assets_reject_changes_to_each_binding_before_selection() {
    let (app, server, attempt) = consent("messenger", true).await;
    let api = service(&app, &server);
    let source = attempt.row(&app).await;
    let calls = server.received_requests().await.unwrap().len();
    let rows = app.db.collection::<Document>(SESSIONS);
    for delta in [
        doc! {"_id":uuid::Uuid::new_v4().to_string()},
        doc! {"actor_id":"other-actor"},
        doc! {"owner_id":"other-owner"},
        doc! {"app_id":"333"},
        doc! {"platform":"instagram"},
        doc! {"grant_subject_id":"555"},
        doc! {"state_hash":"other-state"},
        doc! {"created_at":bson::DateTime::from_millis(source.get_datetime("created_at").unwrap().timestamp_millis()+1)},
        doc! {"bot_id":null},
        doc! {"reconnect_bot_id":"other-bot"},
        doc! {"expected_generation":2_i64},
        doc! {"assets_context_version":4},
        doc! {"assets_context_version":null},
        doc! {"grant_identity":"other-identity"},
    ] {
        let mut changed = source.clone();
        changed.extend(delta);
        let session: OAuthSession = bson::from_document(changed.clone()).unwrap();
        assert!(api.assets(&session).await.is_err());
        assert!(
            asset_record::verify(&app.encryption_keys, &changed)
                .await
                .is_err()
        );
    }
    // Keep all other components identical and change only the field or collection.
    let s: OAuthSession = bson::from_document(source.clone()).unwrap();
    let attempt_id = serde_json::to_string(&(
        s.created_at.timestamp_millis(),
        &s.bot_id,
        &s.reconnect_bot_id,
        s.expected_generation,
    ))
    .unwrap();
    let subject = s.grant_subject_id.as_deref().unwrap();
    let plain = Zeroizing::new(serde_json::to_vec(&api.assets(&s).await.unwrap()).unwrap());
    for (collection, field) in [
        (SESSIONS, "other_field"),
        ("other_collection", "assets_encrypted"),
    ] {
        let path = [
            field,
            &s.actor_id,
            &s.platform,
            &s.app_id,
            subject,
            &s.state_hash,
            &attempt_id,
        ];
        let ctx =
            crate::crypto::aes::RecordContext::new(collection, &s.id, &s.owner_id, &path).unwrap();
        let bytes = app
            .encryption_keys
            .encrypt_record(&plain, &ctx)
            .await
            .unwrap();
        rows.update_one(
            doc! {"_id":&s.id},
            doc! {"$set":{"assets_encrypted":binary(bytes)}},
        )
        .await
        .unwrap();
        assert!(api.finish(&s.actor_id, &s.id, "888", true).await.is_err());
    }
    assert_eq!(server.received_requests().await.unwrap().len(), calls);
}

#[tokio::test]
async fn ambiguous_legacy_assets_block_the_walk_with_metadata_only() {
    use crate::services::encrypted_record_migration as migration;
    let (app, server, attempt) = consent("messenger", false).await;
    let active = keys(true, true);
    let source = attempt.row(&app).await;
    let source_session: OAuthSession = bson::from_document(source.clone()).unwrap();
    let source_subject = source_session.grant_subject_id.as_deref().unwrap();
    let assets = asset_record::open(&active, &source_session, source_subject)
        .await
        .unwrap();
    let mut ambiguous = source.clone();
    for (case, ciphertext) in [
        ("duplicate asset ID", historical::DUPLICATE_ID),
        ("invalid asset ID", historical::INVALID_ID),
        ("empty token", historical::EMPTY_TOKEN),
        ("wrong grant subject", historical::WRONG_SUBJECT),
        ("unexpected expiry", historical::UNEXPECTED_EXPIRY),
    ] {
        ambiguous.insert("assets_encrypted", binary(hex::decode(ciphertext).unwrap()));
        assert!(
            matches!(asset_record::migrate(&app.db,&active,&ambiguous).await,Err(AppError::Conflict(s)) if s == asset_record::AMBIGUOUS),
            "{case}"
        );
    }
    for update in [
        doc! {"status":"installing","selected_asset_id":"123"},
        doc! {"grant_identity":"different"},
        doc! {"platform":"instagram"},
    ] {
        let mut row = source.clone();
        row.extend(update);
        assert!(asset_record::migrate(&app.db, &active, &row).await.is_err());
    }
    let rows = app.db.collection::<Document>(SESSIONS);
    rows.replace_one(doc! {"_id": &attempt.id}, &ambiguous)
        .await
        .unwrap();
    let api = service(&app, &server);
    let calls = server.received_requests().await.unwrap().len();
    assert!(
        api.finish(&attempt.actor, &attempt.id, "888", true)
            .await
            .is_err()
    );
    let generation = migration::start(&app.db, &active, migration::Mode::Migrate, None)
        .await
        .unwrap();
    for _ in 0..80 {
        let report = migration::status(&app.db, &active).await.unwrap().unwrap();
        if report.phase == "observed" {
            break;
        }
        migration::advance(&app.db, &active, &generation)
            .await
            .unwrap();
    }
    let report = migration::status(&app.db, &active).await.unwrap().unwrap();
    assert_eq!(report.phase, "observed");
    assert!(
        report
            .field_outcomes
            .iter()
            .any(|f| f.collection == SESSIONS && f.verification_blocked == 1)
    );
    let findings = migration::list_findings(&app.db, None).await.unwrap();
    let finding = findings
        .findings
        .iter()
        .find(|f| f.collection == SESSIONS)
        .unwrap();
    assert_eq!(finding.category, asset_record::AMBIGUOUS);
    let summary = serde_json::to_string(finding).unwrap();
    for private in [
        &attempt.actor,
        &attempt.id,
        assets[0].token.as_str(),
        "First Page",
    ] {
        assert!(!summary.contains(private));
    }
    assert!(attempt.row(&app).await == ambiguous);
    assert_eq!(calls, server.received_requests().await.unwrap().len());
}

#[tokio::test]
async fn rewrap_compares_the_entire_consent_attempt_and_preserves_delayed_ttl() {
    let (app, server, attempt) = consent("instagram", false).await;
    let active = keys(true, true);
    let source = attempt.row(&app).await;
    let rows = app.db.collection::<Document>(SESSIONS);
    let id = doc! {"_id": &attempt.id};
    for delta in [
        doc! {"status":"installing","selected_asset_id":"777"},
        doc! {"expected_generation":7_i64},
        doc! {"connect_link_id":"replacement"},
        doc! {"grant_identity":null},
        doc! {"assets_encrypted":bson::to_bson(source.get_binary_generic("assets_encrypted").unwrap()).unwrap()},
    ] {
        let mut winner = source.clone();
        winner.extend(delta);
        rows.replace_one(id.clone(), &winner).await.unwrap();
        assert!(
            !asset_record::migrate(&app.db, &active, &source)
                .await
                .unwrap()
        );
        assert!(attempt.row(&app).await == winner);
    }
    // Hold the physical document until the test explicitly delivers its TTL delete.
    rows.drop_index("expires_at_1").await.unwrap();
    let mut expired = source;
    let deadline = bson::DateTime::from_millis(Utc::now().timestamp_millis() - 1);
    expired.insert("expires_at", deadline);
    rows.replace_one(id.clone(), &expired).await.unwrap();
    assert!(
        asset_record::migrate(&app.db, &active, &expired)
            .await
            .unwrap()
    );
    let winner = attempt.row(&app).await;
    assert_eq!(winner.get_datetime("expires_at"), Ok(&deadline));
    asset_record::verify(&active, &winner).await.unwrap();
    assert!(
        service(&app, &server)
            .finish(&attempt.actor, &attempt.id, "777", true)
            .await
            .is_err()
    );
    let rotated = EncryptionKeys::with_provider(std::sync::Arc::new(
        crate::crypto::local_key_provider::LocalKeyProvider::new([0x42; 32], Some([0x11; 32])),
    ));
    assert!(asset_record::verify(&rotated, &winner).await.is_err());
    assert!(
        asset_record::migrate(&app.db, &rotated, &winner)
            .await
            .unwrap()
    );
    let rotated_row = attempt.row(&app).await;
    asset_record::verify(&rotated, &rotated_row).await.unwrap();
    assert_eq!(rotated_row.get_datetime("expires_at"), Ok(&deadline));
    rows.delete_one(id.clone()).await.unwrap();
    assert!(
        !asset_record::migrate(&app.db, &active, &expired)
            .await
            .unwrap()
    );
    assert!(rows.find_one(id).await.unwrap().is_none());
}

#[tokio::test]
async fn stale_rewrap_preserves_install_revocation_and_erasure_winners() {
    for operation in ["install", "revoke", "provider_erase", "account_erase"] {
        for migrate_first in [false, true] {
            let (app, server, attempt) = consent("instagram", false).await;
            let active = keys(true, true);
            let source = attempt.row(&app).await;
            if migrate_first {
                assert!(
                    asset_record::migrate(&app.db, &active, &source)
                        .await
                        .unwrap()
                );
            }
            let api = service(&app, &server);
            match operation {
                "install" => {
                    attempt.select(&app, &server, "instagram", false).await;
                }
                "revoke" | "provider_erase" => {
                    accept_provider_request(
                        &api,
                        "instagram",
                        &provider_request(json!(900), Utc::now().timestamp()),
                        operation == "provider_erase",
                    )
                    .await
                    .unwrap();
                }
                _ => erase_personal_owner(&app, &attempt.actor).await,
            }
            let rows = app.db.collection::<Document>(SESSIONS);
            let calls = server.received_requests().await.unwrap().len();
            assert!(
                !asset_record::migrate(&app.db, &active, &source)
                    .await
                    .unwrap()
            );
            let winner = rows.find_one(doc! {"_id":&attempt.id}).await.unwrap();
            if operation.ends_with("erase") {
                assert!(winner.is_none(), "{operation}");
            } else {
                let winner = winner.unwrap();
                assert_eq!(
                    winner.get_str("status"),
                    Ok(if operation == "revoke" {
                        "revoked"
                    } else {
                        "completed"
                    })
                );
                assert!(
                    !winner.contains_key("assets_encrypted")
                        && !winner.contains_key("assets_context_version")
                );
                asset_record::verify(&active, &winner).await.unwrap();
            }
            assert_eq!(calls, server.received_requests().await.unwrap().len());
        }
    }
}
