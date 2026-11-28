use super::super::companions::Parents;
use super::retained::check;
use super::*;
use crate::services::channel_guest_access::{relay, transport};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};

async fn channel_fixture(f: &Family, k: &EncryptionKeys, current: bool) -> (Document, usize) {
    let now = DateTime::from_millis(0);
    let mut row = doc! {"_id":"row","user_id":"owner","created_at":now,"updated_at":now};
    let mut versions = Document::new();
    let mut count = 0;
    for &field in f.entry.fields {
        let (path, payload): (Vec<&str>, Vec<u8>) = match f.entry.adapter {
            Adapter::ChannelBot => {
                row.extend(doc!{"platform":"lark","platform_bot_id":"bot","platform_bot_username":"bot","label":"bot","is_active":false,"webhook_registered":false,"status":"inactive"});
                row.insert(
                    "webhook_secret_hash",
                    hash_token("webhook_secret_encrypted"),
                );
                (vec![field], field.as_bytes().to_vec())
            }
            Adapter::NyxbotChannelKey => {
                row.extend(doc!{"bot_owner_id":"org","channel_bot_id":"bot","platform":"lark","bot_label":"bot","route_api_key_id":"route","transport":"gateway","status":"active","agent_api_key_id":"key"});
                (vec![field, "key"], b"credential".to_vec())
            }
            Adapter::EventBinding => {
                row.extend(doc!{"agent_id":"agent","agent_owner_id":"org","destination_channel_id":"chat","destination_partition":"partition","provider_api_key_id":"key","producer_key_id":"producer","status":"ready"});
                if field == "provider_key_ciphertext" {
                    (vec![field, "key", "agent", "org"], b"credential".to_vec())
                } else {
                    (
                        vec![field, "producer", "agent", "org"],
                        STANDARD.encode([9; 32]).into_bytes(),
                    )
                }
            }
            Adapter::NativeWhatsapp => {
                row.extend(doc!{"owner_id":"owner","ownership_version":2,"meta_app_id":"111","waba_id":"222","phone_number_id":"333","dedicated_app":true,"revision":5,"callback":{"phase":"gateway"}});
                (vec![field, "2"], b"fixture-verification-token".to_vec())
            }
            Adapter::ChannelReceiver => {
                let bytes = seal(k, f.collection(), "row", "owner", &[field], b"receiver").await;
                row.extend(doc!{"created_by_user_id":"owner","platform":"discord","label":"x","token_hash":"h","status":"expired","delivery_attempts":0,"expires_at":now,
                    "receiver":{"kind":"direct","secret_encrypted":bin(bytes),"url":"https://example.invalid","key_id":"key"}});
                if current {
                    versions.insert(field, 3);
                }
                count += 1;
                continue;
            }
            Adapter::MetaConsentAssets => {
                use crate::services::channel_meta_service::asset_record;
                let subject = "123";
                row.extend(doc!{"owner_id":"owner","actor_id":"actor","platform":"messenger","app_id":"42","state_hash":"state","redirect_uri":"uri","label":"x","status":"assets","grant_subject_id":subject,"expires_at":now});
                let session = bson::from_document(row.clone()).unwrap();
                let attempt = asset_record::attempt(&session).unwrap();
                let path = asset_record::path(&session, subject, &attempt);
                let bytes = seal(
                    k,
                    f.collection(),
                    "row",
                    "owner",
                    &path,
                    br#"[{"id":"99","name":"page","token":"token","grant_subject_id":"123"}]"#,
                )
                .await;
                row.insert(field, bin(bytes));
                if current {
                    row.insert("assets_context_version", 3);
                }
                count += 1;
                continue;
            }
            Adapter::ChannelGuestReplyHandle => {
                let dest = transport::destination("chat", Some("discord-interaction:event"));
                row.extend(doc!{"platform":"discord","direction":"inbound","channel_bot_id":"bot","platform_message_id":"event","platform_conversation_id":"chat","conversation_id":"binding","content_type":"text","guest_relay":{"bot_id":"bot","binding_id":"binding","platform":"discord","conversation_id":&dest,"source_event_id":"event","actor":{"id":"actor","assurance":"provider_verified"},"audience":"private","policy_revision":1,"activation_epoch":1}});
                let contextual = test_encryption_keys();
                let bytes = seal(
                    &contextual,
                    f.collection(),
                    "row",
                    "owner",
                    &[
                        field,
                        "guest-relay-discord",
                        "bot",
                        "discord",
                        "binding",
                        "event",
                        &dest,
                    ],
                    b"interaction:app:reply",
                )
                .await;
                row.insert(
                    field,
                    format!(
                        "{}{}",
                        relay::REPLY_HANDLE_PREFIX,
                        URL_SAFE_NO_PAD.encode(bytes)
                    ),
                );
                count += 1;
                continue;
            }
            _ => unreachable!(),
        };
        row.insert(
            field,
            bin(seal(k, f.collection(), "row", "owner", &path, &payload).await),
        );
        if current {
            versions.insert(field, 3);
        }
        count += 1;
    }
    if !versions.is_empty() {
        row.insert("secret_context_versions", versions);
    }
    (row, count)
}

#[tokio::test]
async fn channel_registry_checks_assets_digests_and_inactive_material() {
    let mut reader = test_encryption_keys();
    reader.set_allow_legacy_record_reads(false);
    let parent: Parents = [(
        "api_keys",
        doc! {"_id":"key","user_id":"owner","key_hash":hash_token("credential"),"is_active":false},
    )]
    .into();
    let mut seen = 0;
    for f in families().filter(|f| super::super::channels::source_companions(f.entry).is_some()) {
        seen += 1;
        for current in [false, true] {
            let mut k = test_encryption_keys();
            k.set_contextual_record_writes(current);
            let (mut row, count) = channel_fixture(&f, &k, current).await;
            let out = check(&reader, &f, &row, &parent).await;
            assert_eq!(
                materials(&out).count(),
                count,
                "{} {:?}",
                f.collection(),
                out.findings
            );
            assert!(
                out.findings
                    .iter()
                    .all(|(_, why)| !current && *why == Finding::AmbiguousBinding),
                "{} {:?}",
                f.collection(),
                out.findings
            );
            let field = f.entry.fields[0];
            match f.entry.adapter {
                Adapter::NyxbotChannelKey | Adapter::EventBinding => {
                    let mut wrong = parent.clone();
                    wrong
                        .get_mut("api_keys")
                        .unwrap()
                        .insert("key_hash", hash_token("other"));
                    assert!(
                        check(&reader, &f, &row, &wrong)
                            .await
                            .findings
                            .contains(&(field, Finding::InvalidValue))
                    );
                    assert!(
                        check(&reader, &f, &row, &Parents::new())
                            .await
                            .findings
                            .contains(&(field, Finding::MissingParent))
                    );
                }
                Adapter::MetaConsentAssets => {
                    row.extend(doc! {"status":"installing","selected_asset_id":"999"});
                    let bad = check(&reader, &f, &row, &parent).await;
                    assert_eq!(materials(&bad).count(), 1);
                    assert!(bad.findings.contains(&(field, Finding::AmbiguousBinding)));
                    if !current {
                        row.remove("grant_subject_id");
                        assert_eq!(
                            materials(&check(&reader, &f, &row, &parent).await).count(),
                            1
                        );
                    }
                }
                Adapter::ChannelBot => {
                    row.insert(field, 0);
                    assert_eq!(
                        materials(&check(&reader, &f, &row, &parent).await).count(),
                        count - 1
                    );
                }
                Adapter::ChannelGuestReplyHandle => {
                    row.insert(field, "interaction:expired");
                    assert!(matches!(
                        inspect(&f, &reader, &row).await.instances.as_slice(),
                        [(_, Storage::Plaintext)]
                    ));
                }
                _ => {}
            }
            let missing = check(&reader, &f, &doc! {"_id":"row"}, &parent).await;
            assert_eq!(materials(&missing).count(), 0);
            assert!(
                !missing.findings.is_empty()
                    || matches!(f.entry.adapter, Adapter::ChannelGuestReplyHandle)
            );
        }
    }
    assert_eq!(seen, 7);
}

#[tokio::test]
async fn independent_key_digest_is_read_at_the_cut_and_keeps_missing_edges() {
    use crate::services::encrypted_record_census::{Census, Scope};
    let db = crate::test_utils::connect_transaction_test_database("observed_channel_key_cut").await;
    let family = family("nyxbot_channels", "agent_key_ciphertext");
    let k = test_encryption_keys();
    let (row, _) = channel_fixture(&family, &k, true).await;
    db.collection::<Document>(family.collection())
        .insert_one(&row)
        .await
        .unwrap();
    db.create_collection("api_keys").await.unwrap();
    let read_cuts = async {
        let scope = Scope {
            registry_digest: "fixture".into(),
            deployment_revision: "one".into(),
            restore_epoch: "one".into(),
            namespaces: [family.collection(), "api_keys"].map(str::to_owned).into(),
        };
        let (census, start) = Census::start(&db, &k, scope).await.unwrap();
        db.collection::<Document>("api_keys").insert_one(doc!{"_id":"key","user_id":"owner","is_active":false,"key_hash":hash_token("credential")}).await.unwrap();
        let out = family
            .observe_at(&k, &row, &census.snapshot(&start).await.unwrap())
            .await
            .unwrap();
        assert!(
            out.findings
                .contains(&("agent_key_ciphertext", Finding::MissingParent))
        );
        assert_eq!(out.parents.len(), 1);
        assert_eq!(
            (&*out.parents[0].collection, out.parents[0].id.as_str()),
            ("api_keys", Some("key"))
        );
        census.request_cut().await.unwrap();
        let cut = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if let Some(cut) = census.capture().await.unwrap() {
                    break cut;
                }
            }
        })
        .await
        .unwrap();
        db.collection::<Document>("api_keys")
            .update_one(
                doc! {"_id":"key"},
                doc! {"$set":{"key_hash":hash_token("unrelated")}},
            )
            .await
            .unwrap();
        let out = family
            .observe_at(&k, &row, &census.snapshot(&cut).await.unwrap())
            .await
            .unwrap();
        assert!(out.findings.is_empty());
        assert_eq!(materials(&out).count(), 1);
    };
    crate::services::encrypted_record_census::tests::with_history(&db, 120, read_cuts).await;
    db.drop().await.unwrap();
}
