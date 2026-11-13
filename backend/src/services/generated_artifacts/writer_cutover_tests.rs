#[tokio::test]
async fn generated_artifacts_writer_cutover_persists_materializes_and_reads_both_formats() {
    for contextual in [false, true] {
        for reader_writes_contextual in [false, true] {
            let db =
                crate::test_utils::connect_transaction_test_database("artifact_writer_cutover")
                    .await;
            let mut keys = crate::test_utils::test_encryption_keys();
            keys.set_contextual_record_writes(contextual);
            ensure_indexes(&db).await.unwrap();
            let (w, mut conversation) = seeded(&db, &keys).await;
            let pending = db
                .collection::<PendingArtifact>(ARTIFACTS)
                .find_one(doc! {"watch_id":&w.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                pending
                    .secret_context_versions
                    .get("data_encrypted")
                    .copied(),
                contextual.then_some(3)
            );
            let ciphertext = pending.data_encrypted.as_deref().unwrap();
            assert!(!ciphertext.windows(MP4.len()).any(|bytes| bytes == MP4));
            keys.set_contextual_record_writes(reader_writes_contextual);
            assert_eq!(
                storage::decrypt(&keys, &pending).await.unwrap().as_slice(),
                MP4
            );
            let mut wrong = pending.clone();
            if contextual {
                wrong.secret_context_versions.clear();
            } else {
                wrong
                    .secret_context_versions
                    .insert("data_encrypted".into(), 3);
            }
            assert!(storage::decrypt(&keys, &wrong).await.is_err());
            wrong = pending.clone();
            wrong
                .secret_context_versions
                .insert("data_encrypted".into(), 2);
            assert!(storage::decrypt(&keys, &wrong).await.is_err());
            if !contextual {
                keys.set_allow_legacy_record_reads(false);
                assert!(storage::decrypt(&keys, &pending).await.is_err());
                keys.set_allow_legacy_record_reads(true);
            }
            let reservation = byte_budget::reserve(sandbox::RESERVATION).await.unwrap();
            let mut session = db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            bind_in_session(
                &db,
                &keys,
                &mut conversation,
                &w,
                &mut session,
                Some(&reservation),
            )
            .await
            .unwrap();
            session.commit_transaction().await.unwrap();
            drop(reservation);
            let id = &conversation.active_turn.as_ref().unwrap().attachments[0].id;
            let attachment = db
                .collection::<AssistantAttachment>(ATTACHMENTS)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                attachment
                    .secret_context_versions
                    .get("data_encrypted")
                    .copied(),
                reader_writes_contextual.then_some(3)
            );
            let read = super::read(&db, &keys, &w.user_id, &w.conversation_id, id)
                .await
                .unwrap();
            assert_eq!(&*read.bytes, MP4);
            drop(read);
            let stored = db
                .collection::<PendingArtifact>(ARTIFACTS)
                .find_one(doc! {"_id":&pending.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.state, "materialized");
            assert!(stored.data_encrypted.is_none() && stored.secret_context_versions.is_empty());
            db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn generated_artifacts_writer_cutover_migrates_legacy_with_exact_snapshot_fence() {
    let db = crate::test_utils::connect_transaction_test_database("artifact_cutover_migrate").await;
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(false);
    let (w, _) = seeded(&db, &keys).await;
    let collection = db.collection::<Document>(ARTIFACTS);
    let legacy = collection
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    assert!(storage::verify(&keys, &legacy).await.is_err());
    assert!(storage::rewrap(&db, &keys, &legacy).await.is_err());
    assert_eq!(
        collection
            .find_one(doc! {"watch_id":&w.id})
            .await
            .unwrap()
            .unwrap(),
        legacy
    );
    keys.set_contextual_record_writes(true);
    let mut wrong = legacy.clone();
    wrong.insert("secret_context_versions", doc! {"data_encrypted":3});
    assert!(storage::rewrap(&db, &keys, &wrong).await.is_err());
    assert!(storage::rewrap(&db, &keys, &legacy).await.unwrap());
    let current = collection
        .find_one(doc! {"watch_id":&w.id})
        .await
        .unwrap()
        .unwrap();
    storage::verify(&keys, &current).await.unwrap();
    assert!(!storage::rewrap(&db, &keys, &current).await.unwrap());
    assert!(
        !storage::rewrap(&db, &keys, &legacy).await.unwrap(),
        "stale source cannot overwrite current ciphertext"
    );
    let row: PendingArtifact = bson::from_document(current.clone()).unwrap();
    assert_eq!(storage::decrypt(&keys, &row).await.unwrap().as_slice(), MP4);
    let mut unchanged = current.clone();
    for field in ["data_encrypted", "secret_context_versions"] {
        unchanged.insert(field, legacy.get(field).unwrap().clone());
    }
    assert_eq!(
        unchanged, legacy,
        "rewrap only changes ciphertext and its marker"
    );
    assert_eq!(
        collection
            .find_one(doc! {"watch_id":&w.id})
            .await
            .unwrap()
            .unwrap(),
        current
    );
    db.drop().await.unwrap();
}
