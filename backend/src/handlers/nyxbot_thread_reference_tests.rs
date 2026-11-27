use super::*;
use crate::services::nyxbot_thread_mutation;
use bson::Document;

async fn activity(f: &Fixture, relay: &str) {
    let sender = Sender {
        id: "fixture-sender",
        display_name: None,
        kind: crate::models::channel_thread::ThreadSenderKind::Human,
        original_kind: None,
        original_message: true,
        owner_eligible: false,
        media: vec![],
        truncated_original_bytes: None,
        guest_facts: Default::default(),
        guest_mode: None,
        native_discord_activity: None,
    };
    let activity = json!({
        "conversation": {"kind":"private","id":"123"},
        "source":{"type":"nyxid_relay"}, "event_id":relay,
    });
    let result = Box::pin(gateway_inbound_inner(
        &f.state,
        &f.channel,
        PARTITION,
        &activity,
        Some("opaque:%2F/λ"),
        &sender,
        "",
        false,
        relay,
        &mut false,
        None,
    ))
    .await
    .unwrap();
    assert!(matches!(result, Inbound::Silent(_)));
}

async fn stored(f: &Fixture) -> Option<Document> {
    f.state
        .db
        .collection::<Document>(THREADS)
        .find_one(doc! {"channel_id":&f.channel.id, "partition": PARTITION})
        .await
        .unwrap()
}

#[tokio::test]
async fn mixed_thread_reference() {
    let (mut f, platform, conversation) = Box::pin(notice_fixture()).await;
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "write".into());
    let path = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let mut keys = crate::test_utils::test_encryption_keys();
    let writes = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() != Ok("false");
    keys.set_contextual_record_writes(writes);
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    f.state.encryption_keys = Arc::new(keys);
    let rows = f.state.db.collection::<Document>(THREADS);
    if step != "write" {
        let fixture: Document =
            bson::from_slice(&std::fs::read(path.as_ref().unwrap()).unwrap()).unwrap();
        rows.delete_many(doc! {"channel_id":&f.channel.id})
            .await
            .unwrap();
        if let Ok(source) = fixture.get_document("thread") {
            f.channel.id = source.get_str("channel_id").unwrap().into();
            rows.insert_one(source).await.unwrap();
        }
    }
    match step.as_str() {
        "write" | "replace" => Box::pin(activity(&f, &step)).await,
        "migrate" => migrate_fixture(&f, &stored(&f).await.unwrap()).await,
        "clear" => {
            let source = stored(&f).await.unwrap();
            let context = (f.state.db.clone(), vec![source]);
            crate::services::transaction_service::run_db_only(
                &f.state.db,
                &context,
                |tx, (db, sources)| {
                    Box::pin(nyxbot_thread_mutation::delete_observed(db, tx, sources))
                },
            )
            .await
            .unwrap();
        }
        "read" | "reject" => (),
        _ => panic!("unknown fixture step"),
    }
    let source = stored(&f).await;
    if let Some(source) = &source {
        let thread: NyxbotThread = bson::from_document(source.clone()).unwrap();
        assert!(sources::reference_live(&thread).is_ok());
        assert!(sources::reference_anchor(&thread).is_ok());
        if matches!(step.as_str(), "write" | "replace") {
            assert_eq!(sources::reference_anchor(&thread).ok(), Some(step.as_str()));
            assert_eq!(source.contains_key("event_ref_context_version"), writes);
        }
        // The real supplemental reader validates the reference before checking
        // the agent key. Stop at that local authority boundary in each binary.
        f.channel.agent_key_ciphertext = None;
        f.channel
            .secret_context_versions
            .remove(crate::services::nyxbot_channel_key::FIELD);
        let origin = &f.event.delivery.as_ref().unwrap().origin;
        let mut decision = supplemental::Decision::new(Purpose::CheckingNotice);
        let result = Box::pin(supplemental::relay(
            &f.state,
            &conversation,
            origin,
            &f.channel,
            &thread,
            "notice",
            &mut decision,
        ))
        .await;
        assert_eq!(result.is_err(), step == "reject");
        assert!(platform.received_requests().await.unwrap().is_empty());
    } else {
        assert!(matches!(step.as_str(), "clear" | "read"));
    }
    if let Some(path) = path
        && !matches!(step.as_str(), "read" | "reject")
    {
        std::fs::write(path, bson::to_vec(&doc! {"thread":source}).unwrap()).unwrap();
    }
    f.state.db.drop().await.unwrap();
}

async fn migrate_fixture(f: &Fixture, source: &Document) {
    assert!(
        crate::services::nyxbot_thread_reference::migrate(
            &f.state.db,
            &f.state.encryption_keys,
            source
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn thread_reference_rewrap_preserves_each_anchor_and_retained_generation() {
    use crate::services::{
        encrypted_record_migration as migration, nyxbot_thread_reference as reference,
    };
    let mut compatible = crate::test_utils::test_encryption_keys();
    compatible.set_contextual_record_writes(false);
    let (f, _, _) = Box::pin(notice_fixture_with_reference_keys(Some(&compatible))).await;
    let source = stored(&f).await.unwrap();
    let rows = f.state.db.collection::<Document>(THREADS);
    let keys = &f.state.encryption_keys;
    let bytes = source.get_binary_generic("event_ref_ciphertext").unwrap();
    for case in [
        "expired",
        "stale-bytes",
        "stale-expiry",
        "missing",
        "relay-era",
    ] {
        let mut row = source.clone();
        row.insert("_id", case);
        row.insert("partition", case);
        match case {
            "expired" | "stale-expiry" => {
                let expires = Utc::now() - ChronoDuration::days(1);
                row.insert("event_ref_expires_at", bson::DateTime::from_chrono(expires));
                if case == "expired" {
                    row.insert(
                        "event_ref_relay_binding",
                        sources::reference_binding(bytes, expires),
                    );
                }
            }
            "stale-bytes" => {
                let newer = seal_thread_reference(
                    &compatible,
                    case,
                    row.get_str("user_id").unwrap(),
                    row.get_str("channel_id").unwrap(),
                    b"new-reference",
                )
                .await;
                row.insert(
                    "event_ref_ciphertext",
                    newer.into_iter().map(i32::from).collect::<Vec<_>>(),
                );
            }
            "missing" => {
                row.remove("event_ref_relay_binding");
            }
            _ => {
                for key in [
                    "event_ref_ciphertext",
                    "event_ref_expires_at",
                    "event_ref_relay_binding",
                    "event_ref_relay_message_id",
                ] {
                    row.remove(key);
                }
            }
        }
        rows.insert_one(&row).await.unwrap();
        let expected: NyxbotThread = bson::from_document(row.clone()).unwrap();
        let prior = sources::reference_anchor(&expected);
        assert_eq!(prior.is_ok(), case == "expired");
        assert_eq!(
            reference::migrate(&f.state.db, keys, &row).await.unwrap(),
            case != "relay-era"
        );
        let current = rows.find_one(doc! {"_id":case}).await.unwrap().unwrap();
        reference::verify(keys, &current).await.unwrap();
        let after: NyxbotThread = bson::from_document(current.clone()).unwrap();
        assert_eq!(prior.ok(), sources::reference_anchor(&after).ok());
        assert_eq!(
            sources::reference_live(&expected).is_ok(),
            sources::reference_live(&after).is_ok()
        );
        if case != "relay-era" {
            assert!(
                reference::open(keys, &expected).await.unwrap().as_str()
                    == reference::open(keys, &after).await.unwrap().as_str()
            );
            row.insert(
                "event_ref_ciphertext",
                current.get("event_ref_ciphertext").unwrap(),
            );
            row.insert("event_ref_context_version", 3);
            if case == "expired" {
                assert_ne!(
                    row.get("event_ref_relay_binding"),
                    current.get("event_ref_relay_binding")
                );
                row.insert(
                    "event_ref_relay_binding",
                    current.get("event_ref_relay_binding").unwrap(),
                );
            }
        }
        assert!(row == current, "{case}");
        assert!(
            !reference::migrate(&f.state.db, keys, &current)
                .await
                .unwrap()
        );
    }
    rows.insert_one(doc! {"_id":"orphan","event_ref_context_version":3})
        .await
        .unwrap();
    let generation = migration::start(&f.state.db, keys, migration::Mode::Migrate, None)
        .await
        .unwrap();
    // Interrupt after the encrypted field committed, before the generation checkpoint.
    assert!(
        reference::migrate(&f.state.db, keys, &source)
            .await
            .unwrap()
    );
    loop {
        let report = migration::status(&f.state.db, keys).await.unwrap().unwrap();
        if report.phase == "observed" {
            let field = report
                .field_outcomes
                .iter()
                .find(|v| v.collection == THREADS)
                .unwrap();
            assert_eq!(
                (
                    field.authenticated_current,
                    field.no_material_observed,
                    field.verification_blocked
                ),
                (5, 1, 1)
            );
            break;
        }
        Box::pin(migration::advance(&f.state.db, keys, &generation))
            .await
            .unwrap();
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn thread_reference_authenticates_identity_utf8_marker_and_primary() {
    use crate::{
        crypto::{aes::EncryptionKeys, record_field::RecordContext},
        services::nyxbot_thread_reference as reference,
    };
    let mut compatible = crate::test_utils::test_encryption_keys();
    compatible.set_contextual_record_writes(false);
    let (f, _, _) = Box::pin(notice_fixture_with_reference_keys(Some(&compatible))).await;
    let legacy = stored(&f).await.unwrap();
    let keys = &f.state.encryption_keys;
    migrate_fixture(&f, &legacy).await;
    let source = stored(&f).await.unwrap();
    let row: NyxbotThread = bson::from_document(source.clone()).unwrap();
    for dimension in [
        "_id",
        "user_id",
        "channel_id",
        "collection",
        "field",
        "utf8",
        "damaged",
        "missing-marker",
        "unknown-marker",
    ] {
        let mut changed = source.clone();
        match dimension {
            "_id" | "user_id" | "channel_id" => {
                changed.insert(dimension, "different");
            }
            "collection" | "field" | "utf8" => {
                let collection = if dimension == "collection" {
                    "nyxbot_events"
                } else {
                    THREADS
                };
                let field = if dimension == "field" {
                    "event_context_ciphertext"
                } else {
                    "event_ref_ciphertext"
                };
                let path = [field, &row.channel_id];
                let ctx = RecordContext::new(collection, &row.id, &row.user_id, &path).unwrap();
                let bytes = if dimension == "utf8" {
                    b"\xff".as_slice()
                } else {
                    b"opaque".as_slice()
                };
                changed.insert(
                    "event_ref_ciphertext",
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: keys.encrypt_record(bytes, &ctx).await.unwrap(),
                    },
                );
            }
            "damaged" => {
                let mut bytes = row.event_ref_ciphertext.clone().unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                changed.insert(
                    "event_ref_ciphertext",
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes,
                    },
                );
            }
            "missing-marker" => {
                changed.remove("event_ref_context_version");
            }
            _ => {
                changed.insert("event_ref_context_version", 7);
            }
        }
        let changed_row = bson::from_document(changed.clone()).unwrap();
        assert!(
            reference::open(keys, &changed_row).await.is_err(),
            "{dimension}"
        );
        assert!(
            reference::verify(keys, &changed).await.is_err(),
            "{dimension}"
        );
        assert!(
            reference::migrate(&f.state.db, keys, &changed)
                .await
                .is_err(),
            "{dimension}"
        );
    }
    let mut cfg = crate::test_utils::test_app_config();
    cfg.encryption_key_previous = cfg.encryption_key.replace("33".repeat(32));
    let rotated = EncryptionKeys::from_config(&cfg);
    assert!(reference::verify(&rotated, &source).await.is_err());
    assert!(
        reference::migrate(&f.state.db, &rotated, &source)
            .await
            .unwrap()
    );
    let current = stored(&f).await.unwrap();
    reference::verify(&rotated, &current).await.unwrap();
    assert!(reference::verify(keys, &current).await.is_err());
    let after = bson::from_document(current).unwrap();
    assert_eq!(
        sources::reference_anchor(&after).ok(),
        sources::reference_anchor(&row).ok()
    );
    assert_eq!(after.event_ref_expires_at, row.event_ref_expires_at);
    f.state.db.drop().await.unwrap();
}
