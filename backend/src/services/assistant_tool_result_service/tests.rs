use super::*;
use crate::services::assistant_authority_tests::fixture;

async fn reassemble(
    f: &crate::services::assistant_authority_tests::Fixture,
    first: Value,
) -> String {
    let mut output = first["content"].as_str().unwrap().to_owned();
    for part in 1..first["parts"].as_u64().unwrap() as usize {
        let value = read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &ReadRequest {
                reference: first["ref"].as_str().unwrap().into(),
                part,
            },
        )
        .await
        .unwrap();
        assert!(envelope(&value.to_string()).to_string().len() <= BUDGET);
        assert_eq!(
            value["complete"],
            part + 1 == first["parts"].as_u64().unwrap() as usize
        );
        output.push_str(value["content"].as_str().unwrap());
    }
    assert_eq!(
        output.len(),
        first["total_bytes"].as_u64().unwrap() as usize
    );
    output
}
#[tokio::test]
async fn text_json_multibyte_and_escaped_results_reassemble_exactly() {
    let f = fixture("tool_result_parts").await;
    for text in ["x".repeat(30_000), json!({"tabs":[{"documentTab":{"body":{"content":[{"paragraph":{"elements":[{"textRun":{"content":"中文 😀\n\"\\\t\u{0001}".repeat(4000)}}]}}]}}}]}).to_string(), "line\n".repeat(6000), "中文 😀\n\"\\\t\u{0001}".repeat(3000)] {
        let first = retain(&f.state.db,&f.state.encryption_keys,&f.chat,&f.owner,&text).await.unwrap();
        assert!(first["parts"].as_u64().unwrap()>1);
        assert!(envelope(&first.to_string()).to_string().len()<=BUDGET);
        assert_eq!(reassemble(&f,first).await,text);
    }
    let row = f
        .state
        .db
        .collection::<AssistantToolResult>(COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{row:?}").contains(&f.owner));
    assert!(std::str::from_utf8(&row.data_encrypted).is_err());
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn exact_binding_stop_expiry_unknown_and_key_revocation_fail_closed() {
    let f = fixture("tool_result_binding").await;
    let first = retain(
        &f.state.db,
        &f.state.encryption_keys,
        &f.chat,
        &f.owner,
        &"private\n".repeat(4000),
    )
    .await
    .unwrap();
    let request = ReadRequest {
        reference: first["ref"].as_str().unwrap().into(),
        part: 1,
    };
    for field in [
        "owner",
        "conversation",
        "turn",
        "key",
        "agent",
        "guest",
        "stop",
    ] {
        let mut chat = f.chat.clone();
        match field {
            "owner" => chat.user_id = "other".into(),
            "conversation" => chat.conversation_id = "other".into(),
            "turn" => chat.turn_id = Some("other".into()),
            "key" => chat.api_key_id = "other".into(),
            "agent" => chat.agent_id = "other".into(),
            "guest" => chat.guest = true,
            _ => chat.turn_stopped = true,
        }
        assert!(
            read(
                &f.state.db,
                &f.state.encryption_keys,
                &chat,
                &f.owner,
                &request
            )
            .await
            .is_err(),
            "{field}"
        );
    }
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            "other",
            &request
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &ReadRequest {
                reference: Uuid::new_v4().to_string(),
                part: 1
            }
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &ReadRequest {
                reference: request.reference.clone(),
                part: usize::MAX
            }
        )
        .await
        .is_err()
    );
    let records = f.state.db.collection::<Document>(COLLECTION_NAME);
    records
        .update_one(doc! {"_id":&request.reference}, doc! {"$set":{"version":2}})
        .await
        .unwrap();
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &request
        )
        .await
        .is_err()
    );
    records.update_one(doc!{"_id":&request.reference},doc!{"$set":{"version":1,"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &request
        )
        .await
        .is_err()
    );
    records.update_one(doc!{"_id":&request.reference},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10))}}).await.unwrap();
    transactions::update_one(
        &f.state.db,
        doc! {"_id":&f.chat.api_key_id},
        doc! {"$set":{"last_used_at":bson::DateTime::now()}},
        None,
    )
    .await
    .unwrap();
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &request
        )
        .await
        .is_ok()
    );
    transactions::update_one(
        &f.state.db,
        doc! {"_id":&f.chat.api_key_id},
        doc! {"$set":{"scopes":""}},
        None,
    )
    .await
    .unwrap();
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &request
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn parallel_retention_is_bounded_and_conversation_deletion_cleans_rows() {
    let f = fixture("tool_result_quota").await;
    let text = "quota".repeat(2000);
    let calls = (0..20).map(|_| {
        retain(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &text,
        )
    });
    let outcomes = futures::future::join_all(calls).await;
    assert_eq!(
        outcomes.iter().filter(|v| v.is_ok()).count(),
        MAX_RESULTS as usize
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        MAX_RESULTS as u64
    );
    assert!(
        retain(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &"x".repeat(MAX_BYTES + 1)
        )
        .await
        .is_err()
    );
    super::super::assistant_nyxagent::delete(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn guest_snapshot_is_same_turn_only_and_stop_refuses_storage() {
    let mut f = fixture("tool_result_guest").await;
    f.state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"guest_turn":true}})
        .await
        .unwrap();
    f.chat.guest = true;
    let first = retain(
        &f.state.db,
        &f.state.encryption_keys,
        &f.chat,
        &f.owner,
        &"guest\n".repeat(4000),
    )
    .await
    .unwrap();
    assert_eq!(reassemble(&f, first.clone()).await, "guest\n".repeat(4000));
    // Retry gets a different server turn; the old ref never adopts it.
    let original = f.chat.turn_id.clone();
    f.chat.turn_id = Some(Uuid::new_v4().to_string());
    f.state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.turn_id":&f.chat.turn_id}},
        )
        .await
        .unwrap();
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &ReadRequest {
                reference: first["ref"].as_str().unwrap().into(),
                part: 1
            }
        )
        .await
        .is_err()
    );
    f.chat.turn_id = original;
    f.state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.turn_id":&f.chat.turn_id,"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        retain(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &"x".repeat(20000)
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn byte_quota_and_expired_live_lease_refuse_retention_and_reads() {
    let f = fixture("tool_result_byte_quota").await;
    let text = "x".repeat(MAX_BYTES);
    for _ in 0..4 {
        retain(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &text,
        )
        .await
        .unwrap();
    }
    assert!(
        retain(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            "overflow"
        )
        .await
        .is_err()
    );
    let row = f
        .state
        .db
        .collection::<AssistantToolResult>(COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    f.state.db.collection::<Document>(CONVERSATIONS).update_one(doc!{"_id":&f.row.id},doc!{"$set":{"active_turn.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(
        read(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            &ReadRequest {
                reference: row.id,
                part: 0
            }
        )
        .await
        .is_err()
    );
    assert!(
        retain(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &f.owner,
            "expired"
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn storage_rewrap_preserves_result_and_authenticates_context() {
    let f = fixture("tool_result_rewrap").await;
    let mut legacy = crate::test_utils::test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let first = retain(
        &f.state.db,
        &legacy,
        &f.chat,
        &f.owner,
        &"retained text".repeat(2000),
    )
    .await
    .unwrap();
    let records = f.state.db.collection::<Document>(COLLECTION_NAME);
    let before = records.find_one(doc! {}).await.unwrap().unwrap();
    assert!(
        storage::verify(&f.state.encryption_keys, &before)
            .await
            .is_err()
    );
    assert!(
        storage::rewrap(&f.state.db, &f.state.encryption_keys, &before)
            .await
            .unwrap()
    );
    let after = records.find_one(doc! {}).await.unwrap().unwrap();
    storage::verify(&f.state.encryption_keys, &after)
        .await
        .unwrap();
    assert!(
        !storage::rewrap(&f.state.db, &f.state.encryption_keys, &after)
            .await
            .unwrap()
    );
    assert_eq!(reassemble(&f, first).await, "retained text".repeat(2000));
    let mut swapped = after.clone();
    swapped.insert("actor_id", "another actor");
    assert!(
        storage::verify(&f.state.encryption_keys, &swapped)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}
